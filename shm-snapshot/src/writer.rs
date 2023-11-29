use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use memmap2::MmapRaw;

pub struct Writer {
    pub(crate) head: Head,
}

/// A read view of a file.
///
/// Can be used to recover data, or convert into a `Writer`.
pub struct File {
    pub(crate) head: Head,
}

pub struct Head {
    head: WriteHead,
    /// The memory map protecting the validity of the write head. This is purely for safety, not
    /// accessing the field besides `Drop`.
    #[allow(dead_code)]
    file: MmapRaw,
}

pub struct Entry<'lt> {
    index: u64,
    head: &'lt mut WriteHead,
}

/// Resolved pointers _into_ a memory map.
///
/// # Safety
///
/// Do **NOT** under and circumstance return the references with unchanged lifetimes. The correct
/// lifetime is the `'self` of an encompassing `Head`.
///
/// It is vitally important that this struct is always paired with a backing file that keeps the
/// allocation. The members lifetime is a lie, the truth impossible to represent as it should have
/// a `'self` lifetime to the owner of the memory. The backing file allocation might be leaked to
/// turn these into true representations though (leaking the allocation with it). If the SharedFd
/// is used similar to an alternative heap then this is correct.
pub(crate) struct WriteHead {
    /// Our process / thread internal view of the head page mapped in the file.
    ///
    /// This exists solely for internal consistency.
    cache: HeadCache,
    meta: &'static HeadPage,
    sequence: &'static [SequencePage],
    data: &'static [DataPage],
}

struct HeadMapRaw {
    meta: *const HeadPage,
    sequence: *const [SequencePage],
    data: *const [DataPage],
}

impl Head {
    pub const RECENT_VERSION: u32 = 0;

    /// Construct this wrapper
    pub(crate) fn from_map(file: MmapRaw) -> Self {
        /// The head page we simulate if the file is too small to contain anything.
        ///
        /// The user will just notice that we can't write, but the construction itself won't fail.
        /// That happens later when the head is converted to a writer and the caller selected some
        /// minimum requirements. Here we just fulfill validity.
        static FALLBACK_HEAD: HeadPage = HeadPage {
            version: AtomicU32::new(Head::RECENT_VERSION),
            page_mask: AtomicU64::new(0),
            page_write_offset: AtomicU64::new(0),
        };

        let ptr = file.as_mut_ptr();
        let len = file.len();

        let head = if let Some(head) = unsafe { Self::map_all_raw(ptr, len) } {
            // Safety: pointers returned are still in-bounds. By keeping `file` we also ensure that
            // the mapping is kept in place. The types themselves are full atomics, meaning we do
            // not have any uniqueness requirements on the pointer.
            //
            // The one scary part is the safety requirement of the pointee being initialized
            // memory. We assume that this is the case for all memory mapped files, initializing
            // pages to zero on faulty access.
            unsafe {
                WriteHead {
                    cache: HeadCache::new(),
                    meta: &*head.meta,
                    sequence: &*head.sequence,
                    data: &*head.data,
                }
            }
        } else {
            WriteHead {
                cache: HeadCache::new(),
                meta: &FALLBACK_HEAD,
                data: &[],
                sequence: &[],
            }
        };

        Head { head, file }
    }

    /// Safety:
    ///
    /// Call promises that `ptr` points to an allocation valid for at least `len` bytes, that is
    /// adding the len to the pointer must be in-bounds.
    unsafe fn map_all_raw(ptr: *mut u8, len: usize) -> Option<HeadMapRaw> {
        let tail_len = len.checked_sub(HeadPage::PAGE_SZ)?;
        let tail = ptr.add(HeadPage::PAGE_SZ);

        let sequence_ptr = tail as *const SequencePage;
        let sequence_len = tail_len / core::mem::size_of::<SequencePage>();
        let data_ptr = tail as *const DataPage;
        let data_len = tail_len / core::mem::size_of::<DataPage>();

        Some(HeadMapRaw {
            meta: ptr as *const HeadPage,
            sequence: core::ptr::slice_from_raw_parts(sequence_ptr, sequence_len),
            data: core::ptr::slice_from_raw_parts(data_ptr, data_len),
        })
    }
}

impl Head {
    pub(crate) fn write(&mut self, data: &[u8]) -> Result<u64, ()> {
        let mut entry = self.head.entry();
        let Some(end_ptr) = entry.new_write_offset(data.len()) else {
            return Err(());
        };

        entry.invalidate_heads_to(end_ptr);
        entry.copy_from_slice(data);
        Ok(entry.commit())
    }
}

impl WriteHead {
    pub(crate) fn entry(&mut self) -> Entry<'_> {
        let index = self.cache.page_write_offset;
        Entry { head: self, index }
    }

    pub(crate) fn new_write_offset(&self, n: usize) -> Option<u64> {
        let len = u64::try_from(n);
        if let Some(len) = len.ok().filter(|&l| l <= self.cache.entry_mask) {
            Some(self.cache.page_write_offset.wrapping_add(len))
        } else {
            None
        }
    }

    /// Invalidate all heads so that `n` bytes can be written.
    pub(crate) fn invalidate_heads_to(&mut self, end: u64) {
        let mut entry = self.cache.entry_read_offset;
        let mut data = self.cache.page_read_offset;

        loop {
            if data >= end {
                break;
            }

            // The entry write offset is ahead of the entry read offset.
            if entry == self.cache.entry_write_offset {
                data = end;
                break;
            }

            let length = self.invalidate_at(entry);
            entry = entry.wrapping_add(1);
            data = data.wrapping_add(length);
        }

        self.cache.entry_read_offset = entry;
        self.cache.page_read_offset = data;
    }

    pub(crate) fn copy_from_slice(&mut self, data: &[u8]) {
        let mut n = self.cache.page_write_offset;

        for (&b, idx) in data.iter().zip(n..) {
            self.write_at(idx, b);
            n = n.wrapping_add(1);
        }

        self.cache.page_write_offset = n;
    }

    fn invalidate_at(&mut self, idx: u64) -> u64 {
        let idx = (idx & self.cache.entry_mask) as usize;

        let page = idx / SequencePage::DATA_COUNT;
        let entry = idx % SequencePage::DATA_COUNT;

        let entry = &self.sequence[page].data[entry];
        entry.length.swap(0, Ordering::Relaxed)
    }

    fn write_at(&self, idx: u64, byte: u8) {
        let idx = idx & self.cache.page_mask;

        let offset = idx % 8;
        let idx = idx / 8;
        let shift = 8 * offset;

        let data_idx = idx as usize % DataPage::DATA_COUNT;
        let page_idx = idx as usize / DataPage::DATA_COUNT;

        let word = &self.data[page_idx].data[data_idx];
        let mask = 0xffu64 << shift;

        let old = word.load(Ordering::Relaxed) & !mask;
        let new = old | (u64::from(byte) << shift);
        word.store(new, Ordering::Relaxed);
    }
}

impl Entry<'_> {
    /// Consume the entry, putting it into the sequence buffer.
    pub(crate) fn commit(self) -> u64 {
        self.index
    }

    pub(crate) fn new_write_offset(&self, n: usize) -> Option<u64> {
        self.head.new_write_offset(n)
    }

    pub(crate) fn invalidate_heads_to(&mut self, end: u64) {
        self.head.invalidate_heads_to(end);
    }

    pub(crate) fn copy_from_slice(&mut self, data: &[u8]) {
        self.head.copy_from_slice(data);
    }
}

struct HeadCache {
    entry_mask: u64,
    entry_read_offset: u64,
    entry_write_offset: u64,
    page_mask: u64,
    page_write_offset: u64,
    page_read_offset: u64,
}

impl HeadCache {
    pub(crate) fn new() -> Self {
        HeadCache {
            entry_mask: 0,
            entry_read_offset: 0,
            entry_write_offset: 0,
            page_mask: 0,
            page_write_offset: 0,
            page_read_offset: 0,
        }
    }
}

struct HeadPage {
    version: AtomicU32,
    /// The mask to translate stream offset to a specific page offset.
    page_mask: AtomicU64,
    /// The stream offset of the next byte to write.
    page_write_offset: AtomicU64,
}

impl HeadPage {
    const PAGE_SZ: usize = 4096;
}

struct SequencePage {
    data: [SequenceEntry; Self::DATA_COUNT],
}

struct SequenceEntry {
    offset: AtomicU64,
    length: AtomicU64,
}

impl SequencePage {
    // FIXME: I currently don't target 32-bit atomic targets. But if then this should depend on
    // such a target choice. The code written should then also get another implementation, and
    // `Writer` only access this by indirection.
    const DATA_COUNT: usize = 4096 / 16;
}

struct DataPage {
    data: [AtomicU64; Self::DATA_COUNT],
}

impl SequencePage {
}

impl DataPage {
    // One AtomicU64 per entry dividing the page.
    const DATA_COUNT: usize = 4096 / 8;
}
