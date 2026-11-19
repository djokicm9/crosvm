// Copyright 2020 The ChromiumOS Authors
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#[cfg(target_arch = "x86_64")]
mod x86_64;
#[cfg(target_arch = "aarch64")]
mod aarch64;
//mod cap;
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

pub struct Xen {
    xch: NonNull<xc_interface>,
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

        Ok(Xen { xch })
    }
}

impl Drop for Xen {
    fn drop(&mut self) {
        // SAFETY:
        // xch was successfully returned by xc_interface_open()
        // and is owned by this Xen instance.
        unsafe {
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
    pub fn new(xen: &Xen, guest_mem: GuestMemory, cfg: Config) -> Result<XenVm> {
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
        let vm = XenVm {
            xen: xen.try_clone()?,
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


