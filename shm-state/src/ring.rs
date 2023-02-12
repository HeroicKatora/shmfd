use crate::{MapError, Mapper};
use core::sync::atomic::AtomicU32;
use shm_fd::{SharedFd, Shm};

/// A transaction descriptor  ring-based abstraction.
///
/// Similar in design to io-uring/XDP rings. There's a difference, the creating program is always
/// in control of buffers. What we're trying to solve is not a synchronization between parties but
/// only atomicity. The creating program has to opt-in to potentially blocking hazards.
///
/// The producer writes a sequence of descriptors to the ring. Each descriptor comes with an owner
/// mark, an aligned `u32`, and denotes a slice of memory within the shared memory. The mark is in
/// an open state when its least bit is `0` and in frozen state otherwise; and monotonically
/// incremented.
///
/// The producer ensures that all writes to the denoted, page aligned slice as well as the payload
/// of the descriptor are *sequenced before* the mark is incremented to the next frozen state. And
/// that the increment away from the frozen state is sequenced before all subsequent writes.
///
/// The consumer _may_ write backups by atomically:
/// 1. finding a frozen descriptor.
/// 2. reading the data corresponding *at least* to the indicated slice and writing its backup.
/// 3. checking that the descriptor is still in the same state as it was found in.
/// 4. replacing its current backup with the new backup.
pub struct Ring {
    inner: SharedFd,
    mapper: Mapper,
    mapping: &'static [AtomicU32],
}

pub struct Descriptor {
    pub mark: u64,
    pub start: u64,
    pub end: u64,
}

struct Header {
    magic: [AtomicU32; 2],
    options: [AtomicU32; 2],
}

struct Producer {
    head: AtomicU32,
}

struct DescriptorInner {
    /// One mark from the producer, one for the consumer if used.
    mark: [AtomicU32; 2],
    payload: [AtomicU32; 2],
    /// The `start` marker.
    start: [AtomicU32; 2],
    /// The `end` offset.
    end: [AtomicU32; 2],
}

impl Ring {
    pub fn new(mapper: Mapper, fd: SharedFd, shm: &Shm) -> Result<Self, MapError> {
        // FIXME: return the actual status code?
        let stat = shm.stat(&fd).map_err(|_| MapError(11))?;
        let size = usize::try_from(stat.st_size).map_err(|_| MapError(11))?;

        let mapping = mapper.mmap_shared(fd.as_raw_fd(), size)?;
        Ok(Ring {
            inner: fd,
            mapper,
            mapping,
        })
    }

    pub fn push(&self, descriptor: Descriptor) {
        todo!()
    }
}
