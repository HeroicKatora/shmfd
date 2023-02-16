#![no_std]
mod area;
mod mmap;
mod ring;
mod seq;

extern crate alloc;

pub use area::AreaFd;
pub use mmap::{Mapper, MapError, VTable};
pub use ring::{Ring, RingOptions, Descriptor};

/// Exports the different atomic, restorable checkpoint loggers.
///
/// The performance characteristics and modification methods vary.
pub mod logs {
    pub use crate::seq::Seq;
}
