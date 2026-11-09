// Copyright 2026 EPAM Systems
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Bindings for the Linux Xen API.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use base::ioctl_ioc_nr;

pub mod bindings;
pub use bindings::*;

// These ioctls are commonly defined on all/multiple platforms.
ioctl_ioc_nr!(
    IOCTL_PRIVCMD_HYPERCALL,
    0, // _IOC_NONE
    b'P',
    0x00,
    std::mem::size_of::<privcmd_hypercall>() as u32
);
