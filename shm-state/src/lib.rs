#![no_std]
mod mmap;
mod ring;

extern crate alloc;

pub use mmap::{Mapper, MapError, VTable};
