use crate::{area::AreaFd, MapError, Mapper};
use core::sync::atomic::{AtomicU32, Ordering};
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
    inner: AreaFd,
    mapper: Mapper,
    /// The inner mmap'd region. It is important that we do not return any reference to it, i.e. we
    /// own this region with this pointer and need to do so on `Drop`.
    mapping: &'static [AtomicU32],
    position: u32,
    layout: Layout,
}

pub struct RingOptions {
    /// Number of descriptors desired.
    /// Must be a power-of-two.
    pub nr_descriptors: u32,
}

struct Layout {
    index_descriptors: usize,
    index_descriptors_mask: u32,
}

/// User-facing descriptor parameter.
pub struct Descriptor {
    pub payload: u64,
    pub start: u64,
    pub end: u64,
}

#[repr(C)]
struct Header {
    magic: [u32; 4],
    options: u32,
    count: u32,
}

struct Producer {
    head: AtomicU32,
}

/// Do not change without checking `Ring::descriptors`.
#[repr(C)]
struct DescriptorInner {
    /// One mark from the producer, one for the consumer if used.
    mark: [AtomicU32; 2],
    /// The user-chosen value.
    payload: [AtomicU32; 2],
    /// The `start` marker.
    start: [AtomicU32; 2],
    /// The `end` offset.
    end: [AtomicU32; 2],
}

impl Ring {
    pub fn new(mapper: Mapper, area: AreaFd, options: &RingOptions) -> Result<Self, MapError> {
        let layout = Self::layout_for(area.len(), options)?;
        let mapping = mapper.mmap_shared(area.fd.as_raw_fd(), area.len())?;

        Ok(Ring {
            inner: area,
            mapper,
            mapping,
            position: 0,
            layout,
        })
    }

    pub fn push(&self, descriptor: Descriptor) {
        fn split_u64(v: u64) -> [AtomicU32; 2] {
            [v as u32, (v >> 32) as u32].map(AtomicU32::new)
        }

        let index = self.position & self.layout.index_descriptors_mask;
        let target = &self.descriptors()[index as usize];

        let old_mark = target.mark[0].load(Ordering::Relaxed);
        // Maybe we add _two_ here, if the mark is still in 'used' state.
        // But surely the lowest bit is unset afterwards and old_mark < new_mark (in a wrapping
        // sense of this relation). This marks the buffer as owned by the producer.
        let new_mark = (old_mark | 1).wrapping_add(1);
        // Ensure the sequencing with regards to buffer modification.
        target.mark[0].store(new_mark, Ordering::Release);
        core::sync::atomic::fence(Ordering::Acquire);
        core::sync::atomic::compiler_fence(Ordering::Acquire);

        let inner = DescriptorInner {
            mark: [AtomicU32::new(new_mark), AtomicU32::new(0)],
            payload: split_u64(descriptor.payload),
            start: split_u64(descriptor.start),
            end: split_u64(descriptor.end),
        };

        for (t, v) in target.payload.iter().zip(inner.payload) {
            t.store(v.into_inner(), Ordering::Relaxed);
        }

        for (t, v) in target.start.iter().zip(inner.start) {
            t.store(v.into_inner(), Ordering::Relaxed);
        }

        for (t, v) in target.end.iter().zip(inner.end) {
            t.store(v.into_inner(), Ordering::Relaxed);
        }

        // Ensure the sequencing with regards to buffer modification.
        target.mark[0].store(new_mark | 1, Ordering::Release);
    }

    fn descriptors(&self) -> &[DescriptorInner] {
        let raw = &self.mapping[self.layout.index_descriptors..];

        unsafe {
            // Safety: the layout of `DescriptorInner` is just an array of 8 AtomicU32.
            &*core::ptr::slice_from_raw_parts(raw.as_ptr() as *const DescriptorInner, raw.len() / 8)
        }
    }

    fn layout_for(len: usize, options: &RingOptions) -> Result<Layout, MapError> {
        // Number of usable Atomic elements.
        let usable_elements = len / 4;
        let non_sharing_count = 256 / 4;

        if !options.nr_descriptors.is_power_of_two() {
            return Err(MapError(11));
        }

        let descriptor_elements = (options.nr_descriptors as usize)
            .checked_mul(8)
            .ok_or(MapError(11))?;

        // Place descriptors right after header.
        let index_descriptors = non_sharing_count;
        let usable_elements = usable_elements
            .checked_sub(non_sharing_count)
            .ok_or(MapError(11))?;
        let _tail = usable_elements
            .checked_sub(descriptor_elements)
            .ok_or(MapError(11))?;

        Ok(Layout {
            index_descriptors,
            index_descriptors_mask: options.nr_descriptors - 1,
        })
    }
}

impl Drop for Ring {
    fn drop(&mut self) {
        let mmap = core::mem::take(&mut self.mapping);
        // Safety: no more references to this region of memory.
        unsafe { self.mapper.munmap(mmap, self.inner.len()) };
    }
}
