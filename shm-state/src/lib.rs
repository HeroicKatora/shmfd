//! 
//!
//! ## shm-state is not a database
//!
//! [Databases should not use `mmap`](https://www.cidrdb.org/cidr2022/papers/p13-crotty.pdf).
//!
//! Explicitly out-of-scope is a synchronization with an external medium such as a drive; and
//! related recovery from _system crashes_. To put this into more technical terms, we assume that
//! the writes to the shared memory region are observable according to shared memory effects even
//! during recovery. This would _not_ be the case for page cache write-backs, that may occur in an
//! order which violates the one established by our write events.
//!
//! Indeed, the OS does not observe these order constraints. The only method of adhering to this
//! unknown order constraint during concurrent modifications would thus be atomic snapshots of the
//! _entire_ memory region which, short of hardware support, would be prohibitively expensive.
//!
//! Note: making a snapshot _after_ the program, the only source of modifications, exits is
//! entirely feasible by reordering the copy _after_ all the program's memory effects. This is not
//! a crash strategy of course. Also possible is a snapshot strategy that coordinates with the
//! program by suspending modifications while the snapshots take place.
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
