#![no_std]
mod area;
mod mmap;
mod ring;

extern crate alloc;

pub use area::AreaFd;
pub use mmap::{Mapper, MapError, VTable};
pub use ring::{Ring, Descriptor};
