// Copyright 2020 The ChromiumOS Authors
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

// We have u32 constants from bindings that are passed into archiitecture-dependent functions
// taking u32/64 parameters. So on 32 bit platforms we may have needless casts.
#![allow(clippy::useless_conversion)]

use std::collections::BTreeMap;
use std::sync::Arc;

use aarch64_sys_reg::AArch64SysRegId;
use base::Error;
use base::Result;
use cros_fdt::Fdt;
use snapshot::AnySnapshot;
use vm_memory::GuestAddress;
use super::XenVcpu;
use super::XenVm;
use crate::Hypervisor;
use crate::PsciVersion;
use crate::VcpuAArch64;
use crate::VcpuFeature;
use crate::VcpuRegAArch64;
use crate::VmAArch64;

fn not_supported<T>() -> Result<T> {
    Err(Error::new(libc::ENOSYS))
}

impl VmAArch64 for XenVm {
    fn get_hypervisor(&self) -> &dyn Hypervisor {
        &self.xen
    }

    fn load_protected_vm_firmware(
        &self,
        _fw_addr: GuestAddress,
        _fw_max_size: u64,
    ) -> Result<()> {
        Err(Error::new(libc::ENOSYS))
    }

    fn create_vcpu(&self, _id: usize) -> Result<Arc<dyn VcpuAArch64>> {
        Err(Error::new(libc::ENOSYS))
    }

    fn create_fdt(
        &self,
        _fdt: &mut Fdt,
        _phandles: &BTreeMap<&str, u32>,
    ) -> cros_fdt::Result<()> {
        // Return an appropriate error for your cros_fdt version.
        todo!()
    }

    fn init_arch(
        &self,
        _payload_entry_address: GuestAddress,
        _fdt_address: GuestAddress,
        _fdt_size: usize,
    ) -> anyhow::Result<()> {
        anyhow::bail!("Guest architecture initialization is handled by Xen")
    }
}

impl VcpuAArch64 for XenVcpu {
    fn init(&self, _features: &[VcpuFeature]) -> Result<()> {
        Err(Error::new(libc::ENOSYS))
    }

    fn init_pmu(&self, _irq: u64) -> Result<()> {
        Err(Error::new(libc::ENOSYS))
    }

    fn has_pvtime_support(&self) -> bool {
        false
    }

    fn init_pvtime(&self, _pvtime_ipa: u64) -> Result<()> {
        Err(Error::new(libc::ENOSYS))
    }

    fn set_one_reg(&self, _reg_id: VcpuRegAArch64, _data: u64) -> Result<()> {
        Err(Error::new(libc::ENOSYS))
    }

    fn get_one_reg(&self, _reg_id: VcpuRegAArch64) -> Result<u64> {
        Err(Error::new(libc::ENOSYS))
    }

    fn set_vector_reg(&self, _reg_num: u8, _data: u128) -> Result<()> {
        Err(Error::new(libc::ENOSYS))
    }

    fn get_vector_reg(&self, _reg_num: u8) -> Result<u128> {
        Err(Error::new(libc::ENOSYS))
    }

    fn get_system_regs(&self) -> Result<BTreeMap<AArch64SysRegId, u64>> {
        Err(Error::new(libc::ENOSYS))
    }

    fn hypervisor_specific_snapshot(&self) -> anyhow::Result<AnySnapshot> {
        anyhow::bail!("Xen vCPU snapshot is not supported")
    }

    fn hypervisor_specific_restore(&self, _data: AnySnapshot) -> anyhow::Result<()> {
        anyhow::bail!("Xen vCPU restore is not supported")
    }

    fn get_psci_version(&self) -> Result<PsciVersion> {
        Err(Error::new(libc::ENOSYS))
    }

    fn set_guest_debug(
        &self,
        _addrs: &[GuestAddress],
        _enable_singlestep: bool,
    ) -> Result<()> {
        Err(Error::new(libc::ENOSYS))
    }

    fn get_max_hw_bps(&self) -> Result<usize> {
        Err(Error::new(libc::ENOSYS))
    }

    fn get_cache_info(&self) -> Result<BTreeMap<u8, u64>> {
        Err(Error::new(libc::ENOSYS))
    }

    fn set_cache_info(&self, _cache_info: BTreeMap<u8, u64>) -> Result<()> {
        Err(Error::new(libc::ENOSYS))
    }
}
