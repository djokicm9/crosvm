#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct privcmd_hypercall {
    pub op: u64,
    pub arg: [u64; 5],
}