// Copyright 2020 The ChromiumOS Authors
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use std::arch::x86_64::CpuidResult;
use std::collections::BTreeMap;
use std::sync::Arc;

use base::errno_result;
use base::error;
use base::ioctl;
use base::ioctl_with_mut_ptr;
use base::ioctl_with_mut_ref;
use base::ioctl_with_ptr;
use base::ioctl_with_ref;
use base::ioctl_with_val;
use base::AsRawDescriptor;
use base::Error;
use base::IoctlNr;
use base::MappedRegion;
use base::Result;
use kvm_sys::*;
use libc::E2BIG;
use libc::EAGAIN;
use libc::EINVAL;
use libc::EIO;
use libc::ENOMEM;
use libc::ENXIO;
use serde::Deserialize;
use serde::Serialize;
use snapshot::AnySnapshot;
use vm_memory::GuestAddress;
use zerocopy::FromZeros;

use super::Config;
use super::Xen;
// TODO
//use super::XenCap;
use super::XenVcpu;
use super::XenVm;
use crate::host_phys_addr_bits;
use crate::ClockState;
use crate::CpuId;
use crate::CpuIdEntry;
use crate::DebugRegs;
use crate::DescriptorTable;
use crate::DeviceKind;
use crate::Fpu;
use crate::FpuReg;
use crate::HypervisorX86_64;
use crate::IoapicRedirectionTableEntry;
use crate::IoapicState;
use crate::IrqSourceChip;
use crate::LapicState;
use crate::PicSelect;
use crate::PicState;
use crate::PitChannelState;
use crate::PitState;
use crate::ProtectionType;
use crate::Regs;
use crate::Segment;
use crate::Sregs;
use crate::VcpuExit;
use crate::VcpuX86_64;
use crate::VmCap;
use crate::VmX86_64;
use crate::Xsave;
use crate::NUM_IOAPIC_PINS;

impl HypervisorX86_64 for Xen {
    fn get_supported_cpuid(&self) -> Result<CpuId> {
        unimplemented!()
    }

    fn get_msr_index_list(&self) -> Result<Vec<u32>> {
        unimplemented!()
    }
}

impl VmX86_64 for XenVm {
    fn get_hypervisor(&self) -> &dyn HypervisorX86_64 {
        unimplemented!()
    }

    fn create_vcpu(&self, id: usize) -> Result<Arc<dyn VcpuX86_64>> {
        unimplemented!()
    }

    fn set_tss_addr(&self, addr: GuestAddress) -> Result<()> {
        Ok(())
    }

    fn set_identity_map_addr(&self, addr: GuestAddress) -> Result<()> {
        Ok(())
    }

    fn load_protected_vm_firmware(
        &self,
        fw_addr: GuestAddress,
        fw_max_size: u64,
    ) -> Result<()> {
        unimplemented!()
    }
}

impl VcpuX86_64 for XenVcpu {
    fn set_interrupt_window_requested(&self, requested: bool) {
        unimplemented!()
    }

    fn ready_for_interrupt(&self) -> bool {
        unimplemented!()
    }

    fn interrupt(&self, irq: u8) -> Result<()> {
        unimplemented!()
    }

    fn inject_nmi(&self) -> Result<()> {
        unimplemented!()
    }

    fn get_regs(&self) -> Result<Regs> {
        unimplemented!()
    }

    fn set_regs(&self, regs: &Regs) -> Result<()> {
        unimplemented!()
    }

    fn get_sregs(&self) -> Result<Sregs> {
        unimplemented!()
    }

    fn set_sregs(&self, sregs: &Sregs) -> Result<()> {
        unimplemented!()
    }

    fn get_fpu(&self) -> Result<Fpu> {
        unimplemented!()
    }

    fn set_fpu(&self, fpu: &Fpu) -> Result<()> {
        unimplemented!()
    }

    fn get_debugregs(&self) -> Result<DebugRegs> {
        unimplemented!()
    }

    fn set_debugregs(&self, debugregs: &DebugRegs) -> Result<()> {
        unimplemented!()
    }

    fn get_xcrs(&self) -> Result<BTreeMap<u32, u64>> {
        unimplemented!()
    }

    fn set_xcr(&self, xcr: u32, value: u64) -> Result<()> {
        unimplemented!()
    }

    fn get_xsave(&self) -> Result<Xsave> {
        unimplemented!()
    }

    fn set_xsave(&self, xsave: &Xsave) -> Result<()> {
        unimplemented!()
    }

    fn get_hypervisor_specific_state(&self) -> Result<AnySnapshot> {
        unimplemented!()
    }

    fn set_hypervisor_specific_state(&self, data: AnySnapshot) -> Result<()> {
        unimplemented!()
    }

    fn get_msr(&self, msr_index: u32) -> Result<u64> {
        unimplemented!()
    }

    fn get_all_msrs(&self) -> Result<BTreeMap<u32, u64>> {
        unimplemented!()
    }

    fn set_msr(&self, msr_index: u32, value: u64) -> Result<()> {
        unimplemented!()
    }

    fn set_cpuid(&self, cpuid: &CpuId) -> Result<()> {
        unimplemented!()
    }

    fn set_guest_debug(
        &self,
        addrs: &[GuestAddress],
        enable_singlestep: bool,
    ) -> Result<()> {
        unimplemented!()
    }

    fn handle_cpuid(&self, entry: &CpuIdEntry) -> Result<()> {
        unimplemented!()
    }

    fn restore_timekeeping(
        &self,
        host_tsc_reference_moment: u64,
        tsc_offset: u64,
    ) -> Result<()> {
        unimplemented!()
    }
}
