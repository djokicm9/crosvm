// Copyright 2020 The ChromiumOS Authors
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//mod cap;

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
//pub use cap::KvmCap;
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
//pub use x86_64::*;
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

/// Storage for constant KVM driver caps
#[derive(Clone, Default)]
struct XenVmCaps {
}

/// A wrapper around creating and using a KVM VM.
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

impl AsRawDescriptor for XenVm {
    fn as_raw_descriptor(&self) -> RawDescriptor {
        self.vm.as_raw_descriptor()
    }
}


