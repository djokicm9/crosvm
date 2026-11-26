// Copyright 2020 The ChromiumOS Authors
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#[cfg(target_arch = "x86_64")]
mod x86_64;
#[cfg(target_arch = "aarch64")]
mod aarch64;
mod xen_sys;
use xen_sys::*;

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::BinaryHeap;
use std::fs::File;
use std::sync::Arc;

use base::errno_result;
use base::AsRawDescriptor;
use base::Event;
use base::FromRawDescriptor;
use base::MappedRegion;
use base::MemoryMapping;
use base::Protection;
use base::RawDescriptor;
use base::Result;
use base::SafeDescriptor;



use sync::Mutex;
use vm_memory::GuestAddress;
use vm_memory::GuestMemory;

use crate::BalloonEvent;
use crate::ClockState;
use crate::Config;
use crate::Datamatch;
use crate::DeviceKind;
use crate::Hypervisor;
use crate::HypervisorCap;
use crate::HypervisorKind;
use crate::IoEventAddress;
use crate::IoParams;
use crate::MemCacheType;
use crate::MemSlot;
use crate::Vcpu;
use crate::VcpuExit;
use crate::VcpuSignalHandle;
use crate::VcpuSignalHandleInner;
use crate::Vm;
use crate::VmCap;
use std::ptr::NonNull;
use std::os::raw::c_void;
use base::info;

// NOTE: This function is part of Xen memory.h API, should be generated via
// C API bindings. However, bindgen seems to have limitation with generating
// function like macros, therefore defined this function locally
#[inline]
pub const fn XENMEM_resource_ioreq_server_frame_ioreq(n: u64) -> u64 {
    1 + n
}

// Using local, casted variants of constants due to binding generation mismatch
const IOREQ_READ_U8: u8 = IOREQ_READ as u8;
const IOREQ_WRITE_U8: u8 = IOREQ_WRITE as u8;

pub struct XenIoreqMapping {
    resource: NonNull<xenforeignmemory_resource_handle>,
    addr: NonNull<shared_iopage_t>,
}

unsafe impl Send for XenIoreqMapping {}
unsafe impl Sync for XenIoreqMapping {}

pub struct XenIoreqServer {
    domid: domid_t,
    server_id: ioservid_t,
    mapping: Option<XenIoreqMapping>,
    evtchn_ports: Vec<evtchn_port_t>,
}

impl XenIoreqServer {
    pub fn new(xen: &Xen, domid: domid_t) -> Result<Self> {
        let mut server_id: ioservid_t = 0;

        let ret = unsafe {
            xendevicemodel_create_ioreq_server(
                xen.xdh.as_ptr(),
                domid,
                HVM_IOREQSRV_BUFIOREQ_OFF as i32,
                &mut server_id,
            )
        };

        if ret < 0 {
            return Err(base::Error::new(libc::EIO));
        }

        Ok(Self {
            domid,
            server_id,
            evtchn_ports: Vec::new(),
            mapping: None,
        })
    }

    pub fn enable(&self, xen: &Xen) -> Result<()> {
        let rc = unsafe {
            xendevicemodel_set_ioreq_server_state(
                xen.xdh.as_ptr(),
                self.domid,
                self.server_id,
                1,
            )
        };

        if rc != 0 {
            return Err(std::io::Error::last_os_error().into());
        }

        Ok(())
    }

    pub fn bind_vcpu_event_channels(
        &mut self,
        xen: &Xen,
        nr_vcpus: u32,
    ) -> Result<()> {
        let mapping = self.mapping.as_ref()
            .ok_or_else(|| base::Error::new(libc::EINVAL))?;

        for i in 0..nr_vcpus {
            let port = unsafe {
                mapping.addr.as_ref().vcpu_ioreq[i as usize].vp_eport
            };

            let local_port = unsafe {
                xenevtchn_bind_interdomain(
                    xen.xeh.as_ptr(),
                    self.domid as u32,
                    port,
                )
            };

            if local_port < 0 {
                return Err(std::io::Error::last_os_error().into());
            }

            self.evtchn_ports.push(local_port as evtchn_port_t);
        }

        Ok(())
    }

    pub fn set_mapping(&mut self, mapping: XenIoreqMapping) {
        self.mapping = Some(mapping);
    }

    pub fn register_mmio_range(
        &self,
        xen: &Xen,
        base: u64,
        size: u64,
    ) -> Result<()> {
        if size == 0 {
            return Err(base::Error::new(libc::EINVAL));
        }

        let end = base
            .checked_add(size - 1)
            .ok_or_else(|| base::Error::new(libc::EINVAL))?;

        let ret = unsafe {
            xendevicemodel_map_io_range_to_ioreq_server(
                xen.xdh.as_ptr(),
                self.domid,
                self.server_id,
                1, // MMIO
                base,
                end,
            )
        };

        if ret < 0 {
            return Err(base::Error::new(libc::EIO));
        }

        Ok(())
    }

    pub fn run_event_loop(&mut self, xen: &Xen) -> Result<()> {
        loop {
            // Wait for a notification from Xen.
            let port = xen.wait_event_channel()?;

            // Find the vCPU associated with this local event-channel port.
            let vcpu_id = self
                .evtchn_ports
                .iter()
                .position(|&p| p == port)
                .ok_or_else(|| base::Error::new(libc::EINVAL))?;

            // Get the corresponding IOREQ entry from the shared page.
            let ioreq = self.get_vcpu_ioreq_mut(vcpu_id)?;

            // Xen should have submitted a request.
            if ioreq.state() != STATE_IOREQ_READY as u8 {
                xen.unmask_event_channel(port)?;
                continue;
            }

            // Mark the request as being processed.
            ioreq.set_state(STATE_IOREQ_INPROCESS as u8);

            // Handle the MMIO request.
            Self::handle_ioreq(ioreq)?;

            // Notify Xen that the response is ready.
            xen.notify_event_channel(port)?;

            // Allow subsequent notifications on this event channel.
            xen.unmask_event_channel(port)?;
        }
    }

    fn get_vcpu_ioreq_mut(&mut self, vcpu_id: usize) -> Result<&mut ioreq_t> {
        let mapping = self
            .mapping
            .as_mut()
            .ok_or_else(|| base::Error::new(libc::EINVAL))?;

        let shared_page = unsafe { mapping.addr.as_mut() };

        let ioreq = shared_page
            .vcpu_ioreq
            .get_mut(vcpu_id)
            .ok_or_else(|| base::Error::new(libc::EINVAL))?;

        Ok(ioreq)
    }

    fn handle_ioreq(req: &mut ioreq_t) -> Result<()> {
        if req.type_ != IOREQ_TYPE_COPY as u8 {
            return Err(base::Error::new(libc::EINVAL));
        }

        if req.data_is_ptr() != 0 {
            // Not supported in the initial implementation.
            return Err(base::Error::new(libc::EINVAL));
        }

        match req.dir() {
            IOREQ_READ_U8 => {
                info!(
                    "MMIO read: addr={:#x}, size={}",
                    req.addr,
                    req.size
                );

                // Temporary test response.
                req.data = 0x1234_5678;
            }
            IOREQ_WRITE_U8 => {
                info!(
                    "MMIO write: addr={:#x}, size={}, value={:#x}",
                    req.addr,
                    req.size,
                    req.data
                );
            }
            _ => return Err(base::Error::new(libc::EINVAL)),
        }

        req.set_state(STATE_IORESP_READY as u8);

        Ok(())
    }
}

pub struct Xen {
    xch: NonNull<xc_interface>,
    xdh: NonNull<xendevicemodel_handle>,
    xfh: NonNull<xenforeignmemory_handle>,
    xeh: NonNull<xenevtchn_handle>,
}

// SAFETY: Xen owns the xc_interface handle and transfers its ownership
// when moved between threads. The handle is closed only when Xen is dropped.
unsafe impl Send for Xen {}
unsafe impl Sync for Xen {}

impl Xen {
    pub fn new() -> Result<Xen> {
        // SAFETY:
        // Passing null loggers is supported by xc_interface_open().
        // A null return value indicates failure.
        let xch = unsafe {
            xc_interface_open(
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        };

        let xch = NonNull::new(xch)
            .ok_or_else(|| base::Error::new(libc::ENODEV))?;

        let xdh = unsafe {
            xendevicemodel_open(
                std::ptr::null_mut(),
                0,
            )
        };

        let xdh = NonNull::new(xdh)
            .ok_or_else(|| base::Error::new(libc::ENODEV))?;

        let xfh = unsafe {
            xenforeignmemory_open(
                std::ptr::null_mut(),
                0,
            )
        };

        let xfh = NonNull::new(xfh)
            .ok_or_else(|| base::Error::new(libc::ENODEV))?;

        let xeh = unsafe {
            xenevtchn_open(
                std::ptr::null_mut(),
                0,
            )
        };

        let xeh = NonNull::new(xeh)
            .ok_or_else(|| base::Error::new(libc::ENODEV))?;

        Ok(Xen { xch, xdh, xfh, xeh })
    }

    pub fn nr_vcpus(&self, domid: domid_t) -> Result<u32> {
        let mut nr_vcpus: u32 = 0;

        let rc = unsafe {
            xendevicemodel_nr_vcpus(
                self.xdh.as_ptr(),
                domid,
                &mut nr_vcpus,
            )
        };

        if rc < 0 {
            return Err(std::io::Error::last_os_error().into());
        }

        Ok(nr_vcpus)
    }

    pub fn map_ioreq_page(
        &self,
        domid: domid_t,
        ioreq_server: &XenIoreqServer,
    ) -> Result<XenIoreqMapping> {
        let mut addr: *mut c_void = std::ptr::null_mut();

        let resource = unsafe {
            xenforeignmemory_map_resource(
                self.xfh.as_ptr(),
                domid,
                XENMEM_resource_ioreq_server,
                ioreq_server.server_id as u32,
                XENMEM_resource_ioreq_server_frame_ioreq(0),
                1,
                &mut addr,
                libc::PROT_READ | libc::PROT_WRITE,
                0,
            )
        };

        let resource = NonNull::new(resource)
            .ok_or_else(std::io::Error::last_os_error)?;

        let addr = NonNull::new(addr.cast::<shared_iopage_t>())
            .ok_or_else(|| base::Error::new(libc::EINVAL))?;

        Ok(XenIoreqMapping { resource, addr })
    }

    pub fn wait_event_channel(&self) -> Result<evtchn_port_t> {
        let mut port: evtchn_port_t = 0;

        let ret = unsafe {
            xenevtchn_pending(
                self.xeh.as_ptr(),
            )
        };

        if ret < 0 {
            return Err(base::Error::new(libc::EIO));
        }

        Ok(port)
    }

    pub fn unmask_event_channel(&self, port: evtchn_port_t) -> Result<()> {
        let ret = unsafe {
            xenevtchn_unmask(self.xeh.as_ptr(), port)
        };

        if ret < 0 {
            return Err(base::Error::new(libc::EIO));
        }

        Ok(())
    }

    pub fn notify_event_channel(&self, port: evtchn_port_t) -> Result<()> {
        let ret = unsafe {
            xenevtchn_notify(self.xeh.as_ptr(), port)
        };

        if ret < 0 {
            return Err(base::Error::new(libc::EIO));
        }

        Ok(())
    }
}

impl Drop for Xen {
    fn drop(&mut self) {
        // SAFETY:
        // xch was successfully returned by xc_interface_open()
        // and is owned by this Xen instance.
        unsafe {
            xenevtchn_close(self.xeh.as_ptr());
            xenforeignmemory_close(self.xfh.as_ptr());
            xendevicemodel_close(self.xdh.as_ptr());
            xc_interface_close(self.xch.as_ptr());
        }
    }
}

impl Hypervisor for Xen {
    fn try_clone(&self) -> Result<Self> {
        Ok(Xen::new()?)
    }

    fn check_capability(&self, _cap: HypervisorCap) -> bool {
        /* TODO
        if let Ok(xen_cap) = KvmCap::try_from(cap) {
            // SAFETY:
            // this ioctl is safe because we know this kvm descriptor is valid,
            // and we are copying over the kvm capability (u32) as a c_ulong value.
            unsafe { ioctl_with_val(self, KVM_CHECK_EXTENSION, kvm_cap as c_ulong) == 1 }
        } else {
            // this capability cannot be converted on this platform, so return false
            false
        }*/
        true
    }
}

/// Storage for constant XEN driver caps
#[derive(Clone, Default)]
struct XenVmCaps {
}

/// A wrapper around creating and using a XEN VM.
pub struct XenVm {
    xen: Xen,
    domid: domid_t,
    ioreq_server: XenIoreqServer,
    // Keep before guest_mem and mem_regions: the VM fd must close before memory is unmapped.
    vm: SafeDescriptor,
    guest_mem: GuestMemory,
    mem_regions: Mutex<BTreeMap<MemSlot, Box<dyn MappedRegion>>>,
    /// A min heap of MemSlot numbers that were used and then removed and can now be re-used
    mem_slot_gaps: Mutex<BinaryHeap<Reverse<MemSlot>>>,
    caps: XenVmCaps,
    force_disable_readonly_mem: bool,
}

impl XenVm {
    /// Constructs a new `XenVm` using the given `Xen` instance.
    pub fn new(xen: &Xen, domid: domid_t, guest_mem: GuestMemory, cfg: Config) -> Result<XenVm> {
        // SAFETY:
        // Safe because we know xen is a real xen fd as this module is the only one that can make
        // Xen objects.
        let ret = 0;
        if ret < 0 {
            return errno_result();
        }
        // SAFETY:
        // Safe because we verify that ret is valid and we own the fd.
        let vm_descriptor = unsafe { SafeDescriptor::from_raw_descriptor(ret) };
        let ioreq_server = XenIoreqServer::new(&xen, domid)?;
        let vm = XenVm {
            xen: xen.try_clone()?,
            domid: domid,
            ioreq_server: ioreq_server,
            vm: vm_descriptor,
            guest_mem,
            mem_regions: Default::default(),
            mem_slot_gaps: Default::default(),
            caps: Default::default(),
            force_disable_readonly_mem: cfg.force_disable_readonly_mem,
        };

        Ok(vm)
    }
}

impl Vm for XenVm {
    fn try_clone_descriptor(&self) -> Result<SafeDescriptor> {
        unimplemented!()
    }

    fn hypervisor_kind(&self) -> HypervisorKind {
        unimplemented!()
    }

    fn check_capability(&self, _c: VmCap) -> bool {
        true
    }

    fn get_guest_phys_addr_bits(&self) -> u8 {
        40
    }

    fn get_memory(&self) -> &GuestMemory {
        &self.guest_mem
    }

    fn add_memory_region(
        &self,
        _guest_addr: GuestAddress,
        _mem_region: Box<dyn MappedRegion>,
        _read_only: bool,
        _log_dirty_pages: bool,
        _cache: MemCacheType,
    ) -> Result<MemSlot> {
        unimplemented!()
    }

    fn msync_memory_region(&self, _slot: MemSlot, _offset: usize, _size: usize) -> Result<()> {
        unimplemented!()
    }

    #[cfg(any(target_os = "android", target_os = "linux"))]
    fn madvise_pageout_memory_region(
        &self,
        _slot: MemSlot,
        _offset: usize,
        _size: usize,
    ) -> Result<()> {
        unimplemented!()
    }

    #[cfg(any(target_os = "android", target_os = "linux"))]
    fn madvise_remove_memory_region(
        &self,
        _slot: MemSlot,
        _offset: usize,
        _size: usize,
    ) -> Result<()> {
        unimplemented!()
    }

    fn remove_memory_region(&self, _slot: MemSlot) -> Result<Box<dyn MappedRegion>> {
        unimplemented!()
    }

    fn create_device(&self, _kind: DeviceKind) -> Result<SafeDescriptor> {
        unimplemented!()
    }

    fn get_dirty_log(&self, _slot: MemSlot, _dirty_log: &mut [u8]) -> Result<()> {
        unimplemented!()
    }

    fn register_ioevent(
        &self,
        _evt: Event,
        _addr: IoEventAddress,
        _datamatch: Datamatch,
    ) -> Result<()> {
        unimplemented!()
    }

    fn unregister_ioevent(
        &self,
        _evt: Event,
        _addr: IoEventAddress,
        _datamatch: Datamatch,
    ) -> Result<()> {
        unimplemented!()
    }

    fn handle_io_events(&self, _addr: IoEventAddress, _data: &[u8]) -> Result<()> {
        unimplemented!()
    }

    fn get_pvclock(&self) -> Result<ClockState> {
        unimplemented!()
    }

    fn set_pvclock(&self, _state: &ClockState) -> Result<()> {
        unimplemented!()
    }

    fn add_fd_mapping(
        &self,
        _slot: u32,
        _offset: usize,
        _size: usize,
        _fd: &dyn AsRawDescriptor,
        _fd_offset: u64,
        _prot: Protection,
    ) -> Result<()> {
        unimplemented!()
    }

    fn remove_mapping(&self, _slot: u32, _offset: usize, _size: usize) -> Result<()> {
        unimplemented!()
    }

    fn handle_balloon_event(&self, _event: BalloonEvent) -> Result<()> {
        unimplemented!()
    }

    fn enable_hypercalls(&self, _nr: u64, _count: usize) -> Result<()> {
        unimplemented!()
    }
}

impl AsRawDescriptor for XenVm {
    fn as_raw_descriptor(&self) -> RawDescriptor {
        self.vm.as_raw_descriptor()
    }
}

/// A wrapper around using a Xen Vcpu.
pub struct XenVcpu {
    #[cfg(target_arch = "x86_64")]
    xen: Xen,
    #[cfg(not(target_arch = "riscv64"))]
    vm: SafeDescriptor,
    vcpu: File,
    id: usize,
    cap_kvmclock_ctrl: bool,
    run_mmap: Arc<MemoryMapping>,
}

impl Vcpu for XenVcpu {
    fn run(&self) -> Result<VcpuExit> {
        unimplemented!()
    }

    fn id(&self) -> usize {
        unimplemented!()
    }

    fn set_immediate_exit(&self, _exit: bool) {
        unimplemented!()
    }

    #[cfg(any(target_os = "android", target_os = "linux"))]
    fn signal_handle(&self) -> VcpuSignalHandle {
        unimplemented!()
    }

    fn handle_mmio(
        &self,
        _handle_fn: &mut dyn FnMut(IoParams) -> Result<()>,
    ) -> Result<()> {
        unimplemented!()
    }

    fn handle_io(
        &self,
        _handle_fn: &mut dyn FnMut(IoParams),
    ) -> Result<()> {
        unimplemented!()
    }

    fn on_suspend(&self) -> Result<()> {
        unimplemented!()
    }

    unsafe fn enable_raw_capability(
        &self,
        _cap: u32,
        _args: &[u64; 4],
    ) -> Result<()> {
        unimplemented!()
    }
}

struct XenVcpuSignalHandle {
    run_mmap: Arc<MemoryMapping>,
}

#[cfg(any(target_os = "android", target_os = "linux"))]
impl VcpuSignalHandleInner for XenVcpu {
    fn signal_immediate_exit(&self) {
        unimplemented!()
    }
}


