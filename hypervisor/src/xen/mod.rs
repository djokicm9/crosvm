// Copyright 2020 The ChromiumOS Authors
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#[cfg(target_arch = "x86_64")]
mod x86_64;
//mod cap;
mod xen_sys;
use xen_sys::*;

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::BinaryHeap;
use std::convert::TryFrom;
use std::ffi::CString;
use std::fs::File;
use std::os::raw::c_ulong;
use std::os::raw::c_void;
use std::os::unix::prelude::OsStrExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::OnceLock;

#[cfg(target_arch = "aarch64")]
pub use aarch64::*;
use base::errno_result;
use base::error;
use base::ioctl;
use base::ioctl_with_mut_ref;
use base::ioctl_with_ref;
use base::ioctl_with_val;
use base::pagesize;
use base::warn;
use base::AsRawDescriptor;
use base::Error;
use base::Event;
use base::FromRawDescriptor;
use base::MappedRegion;
use base::MemoryMapping;
use base::MemoryMappingBuilder;
use base::MmapError;
use base::Protection;
use base::RawDescriptor;
use base::Result;
use base::SafeDescriptor;
//pub use cap::XenCap;
use cfg_if::cfg_if;
use libc::open64;
use libc::EFAULT;
use libc::EINVAL;
use libc::EIO;
use libc::ENOENT;
use libc::ENOSPC;
use libc::ENOSYS;
#[cfg(not(target_arch = "aarch64"))]
use libc::ENOTSUP;
use libc::EOVERFLOW;
use libc::O_CLOEXEC;
use libc::O_RDWR;
#[cfg(target_arch = "riscv64")]
use riscv64::*;
use sync::Mutex;
use vm_memory::GuestAddress;
use vm_memory::GuestMemory;
#[cfg(target_arch = "x86_64")]
use zerocopy::FromZeros;

use crate::BalloonEvent;
use crate::ClockState;
use crate::Config;
use crate::Datamatch;
use crate::DeviceKind;
use crate::HypercallAbi;
use crate::Hypervisor;
use crate::HypervisorCap;
use crate::HypervisorKind;
use crate::IoEventAddress;
use crate::IoOperation;
use crate::IoParams;
use crate::IrqRoute;
use crate::IrqSource;
use crate::MPState;
use crate::MemCacheType;
use crate::MemSlot;
use crate::Vcpu;
use crate::VcpuExit;
use crate::VcpuSignalHandle;
use crate::VcpuSignalHandleInner;
use crate::Vm;
use crate::VmCap;

pub struct Xen {
    xen: SafeDescriptor
}

impl Xen {
    pub fn new_with_path(device_path: &Path) -> Result<Xen> {
        let c_path = CString::new(device_path.as_os_str().as_bytes()).unwrap();
        // SAFETY:
        // Open calls are safe because we give a nul-terminated string and verify the result.
        let ret = unsafe { open64(c_path.as_ptr(), O_RDWR | O_CLOEXEC) };
        if ret < 0 {
            return errno_result();
        }
        // SAFETY:
        // Safe because we verify that ret is valid and we own the fd.
        let xen = unsafe { SafeDescriptor::from_raw_descriptor(ret) };

        Ok(Xen {
            xen,
        })
    }

    /// Opens `/dev/xen/hypercall` and returns a Xen object on success.
    pub fn new() -> Result<Xen> {
        Xen::new_with_path(Path::new("/dev/xen/hypercall"))
    }
}

impl AsRawDescriptor for Xen {
    fn as_raw_descriptor(&self) -> RawDescriptor {
        self.xen.as_raw_descriptor()
    }
}

impl Hypervisor for Xen {
    fn try_clone(&self) -> Result<Self> {
        Ok(Xen {
            xen: self.xen.try_clone()?,
        })
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
        let mut vm = XenVm {
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

    fn check_capability(&self, c: VmCap) -> bool {
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
        guest_addr: GuestAddress,
        mem_region: Box<dyn MappedRegion>,
        read_only: bool,
        log_dirty_pages: bool,
        cache: MemCacheType,
    ) -> Result<MemSlot> {
        unimplemented!()
    }

    fn msync_memory_region(&self, slot: MemSlot, offset: usize, size: usize) -> Result<()> {
        unimplemented!()
    }

    #[cfg(any(target_os = "android", target_os = "linux"))]
    fn madvise_pageout_memory_region(
        &self,
        slot: MemSlot,
        offset: usize,
        size: usize,
    ) -> Result<()> {
        unimplemented!()
    }

    #[cfg(any(target_os = "android", target_os = "linux"))]
    fn madvise_remove_memory_region(
        &self,
        slot: MemSlot,
        offset: usize,
        size: usize,
    ) -> Result<()> {
        unimplemented!()
    }

    fn remove_memory_region(&self, slot: MemSlot) -> Result<Box<dyn MappedRegion>> {
        unimplemented!()
    }

    fn create_device(&self, kind: DeviceKind) -> Result<SafeDescriptor> {
        unimplemented!()
    }

    fn get_dirty_log(&self, slot: MemSlot, dirty_log: &mut [u8]) -> Result<()> {
        unimplemented!()
    }

    fn register_ioevent(
        &self,
        evt: Event,
        addr: IoEventAddress,
        datamatch: Datamatch,
    ) -> Result<()> {
        unimplemented!()
    }

    fn unregister_ioevent(
        &self,
        evt: Event,
        addr: IoEventAddress,
        datamatch: Datamatch,
    ) -> Result<()> {
        unimplemented!()
    }

    fn handle_io_events(&self, addr: IoEventAddress, data: &[u8]) -> Result<()> {
        unimplemented!()
    }

    fn get_pvclock(&self) -> Result<ClockState> {
        unimplemented!()
    }

    fn set_pvclock(&self, state: &ClockState) -> Result<()> {
        unimplemented!()
    }

    fn add_fd_mapping(
        &self,
        slot: u32,
        offset: usize,
        size: usize,
        fd: &dyn AsRawDescriptor,
        fd_offset: u64,
        prot: Protection,
    ) -> Result<()> {
        unimplemented!()
    }

    fn remove_mapping(&self, slot: u32, offset: usize, size: usize) -> Result<()> {
        unimplemented!()
    }

    fn handle_balloon_event(&self, event: BalloonEvent) -> Result<()> {
        unimplemented!()
    }

    fn enable_hypercalls(&self, nr: u64, count: usize) -> Result<()> {
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

    fn set_immediate_exit(&self, exit: bool) {
        unimplemented!()
    }

    #[cfg(any(target_os = "android", target_os = "linux"))]
    fn signal_handle(&self) -> VcpuSignalHandle {
        unimplemented!()
    }

    fn handle_mmio(
        &self,
        handle_fn: &mut dyn FnMut(IoParams) -> Result<()>,
    ) -> Result<()> {
        unimplemented!()
    }

    fn handle_io(
        &self,
        handle_fn: &mut dyn FnMut(IoParams),
    ) -> Result<()> {
        unimplemented!()
    }

    fn on_suspend(&self) -> Result<()> {
        unimplemented!()
    }

    unsafe fn enable_raw_capability(
        &self,
        cap: u32,
        args: &[u64; 4],
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


