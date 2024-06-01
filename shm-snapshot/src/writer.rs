use core::iter::Extend;
use core::sync::atomic::{AtomicU64, Ordering};
use memmap2::MmapRaw;

/// A memory-mapped file into which this writer adds new snapshot.
pub struct Writer {
    pub(crate) head: Head,
}

/// A read view of a file.
///
/// Can be used to recover data, or convert into a `Writer`.
pub struct File {
    pub(crate) head: Head,
}

/// A view onto a memory-mapped file, which has a configured layout.
pub struct FileDiscovery<'lt> {
    pub(crate) file: &'lt File,
    pub(crate) configuration: ConfigureFile,
}

/// Describes the layout of a shared memory in a [`Writer`].
#[derive(Default, Debug)]
pub struct ConfigureFile {
    /// The number of entries in the sequence ring buffer.
    pub entries: u64,
    /// The number of bytes in the data ring buffer.
    pub data: u64,
    /// The offset of the next-to-write entry.
    pub initial_offset: u64,
    /// The indicate version in the file, or an explicit invalid number.
    ///
    /// Can't allow it to be public, it's not supposed to be arbitrarily set.
    pub(crate) layout_version: u64,
}

pub struct Head {
    head: WriteHead,
    /// The memory map protecting the validity of the write head. This is purely for safety, not
    /// accessing the field besides `Drop`.
    #[allow(dead_code)]
    file: MmapRaw,
}

/// The descriptor of a singular snapshot.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Snapshot {
    /// The offset of data in the data ring.
    pub offset: u64,
    /// The length of data in the data ring. A non-zero length marks a valid entry, a zero length
    /// an invalid entry.
    pub length: u64,
}

pub(crate) trait Collect<T> {
    fn insert_one(&mut self, _: T) -> bool;
}

impl<T> Collect<T> for Vec<T> {
    fn insert_one(&mut self, val: T) -> bool {
        self.push(val);
        true
    }
}

pub(crate) struct Entry<'lt> {
    index: u64,
    offset: u64,
    length: u64,
    head: &'lt mut WriteHead,
}

/// An unfinished entry in a writer's ring, which can be atomically committed.
pub struct PreparedTransaction<'lt> {
    offset: u64,
    length: u64,
    head: &'lt mut WriteHead,
    tail: &'lt [DataPage],
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
    pub(crate) cache: HeadCache,
    pub(crate) meta: &'static HeadPage,
    pub(crate) sequence: &'static [SequencePage],
    pub(crate) data: &'static [DataPage],
    /// Data pages from the shared memory which we do not touch ourselves, i.e. user reserved.
    pub(crate) tail: &'static [DataPage],
}

struct HeadMapRaw {
    meta: *const HeadPage,
    sequence: *const [SequencePage],
    data: *const [DataPage],
}

impl Head {
    fn fitting_power_of_two(value: u64) -> u64 {
        const HIGEST_BIT_SET: u64 = !((!0) >> 1);
        // Must be a power of two, use the next lower one.
        HIGEST_BIT_SET >> value.leading_zeros()
    }

    pub(crate) fn discover(&self, cfg: &mut ConfigureFile) {
        let entry_mask = self.head.meta.entry_mask.load(Ordering::Relaxed);
        let data_mask = self.head.meta.page_mask.load(Ordering::Relaxed);
        let page_write_offset = self.head.meta.page_write_offset.load(Ordering::Relaxed);

        let layout_version = self.head.meta.version.load(Ordering::Relaxed);
        assert!(entry_mask < usize::MAX as u64);
        assert!(data_mask < usize::MAX as u64);

        let sequence = (entry_mask + 1) as usize;
        // Assume this refers to the whole tail at this point?
        let pages = self.head.data.len();
        let psequence = sequence / SequencePage::DATA_COUNT
            + usize::from(sequence % SequencePage::DATA_COUNT != 0);

        let data_space = (pages - psequence) as u64 * core::mem::size_of::<DataPage>() as u64;
        let available_entries = Self::fitting_power_of_two(entry_mask + 1);
        let available_data = Self::fitting_power_of_two(data_space);

        cfg.entries = available_entries;
        cfg.data = available_data.min(data_mask + 1);
        cfg.initial_offset = page_write_offset;
        cfg.layout_version = layout_version;
    }

    pub(crate) fn configure(&mut self, cfg: &ConfigureFile) {
        Self::configure_head(&mut self.head, cfg)
    }

    fn configure_head(head: &mut WriteHead, cfg: &ConfigureFile) {
        assert!(cfg.entries.next_power_of_two() == cfg.entries);
        assert!(cfg.data.next_power_of_two() == cfg.data);
        assert!(cfg.is_initialized());

        head.pre_configure_entries(cfg.entries);
        head.pre_configure_pages(cfg.data);
        head.pre_configure_write(cfg.initial_offset);
        head.configure_pages();
    }

    #[inline(always)]
    pub(crate) fn valid(&self, into: &mut impl Extend<Snapshot>) {
        Self::valid_in_head(&self.head, into)
    }

    pub(crate) fn valid_at(&self, into: &mut impl Extend<Snapshot>, cfg: &ConfigureFile) {
        let mut alternate_head = WriteHead {
            cache: HeadCache { ..self.head.cache },
            ..self.head
        };

        Self::configure_head(&mut alternate_head, cfg);
        Self::valid_in_head(&alternate_head, into);
    }

    pub(crate) fn retain_at(&self, retain: &dyn super::RetainSnapshot, cfg: &ConfigureFile) {
        let mut alternate_head = WriteHead {
            cache: HeadCache { ..self.head.cache },
            ..self.head
        };

        Self::configure_head(&mut alternate_head, cfg);
        Self::retain_in_head(&alternate_head, retain);
    }

    pub(crate) fn entry_at(&self, idx: super::SnapshotIndex) -> Snapshot {
        let snapshot = self.head.entry_at_relaxed(idx.entry);
        core::sync::atomic::fence(Ordering::Acquire);
        snapshot
    }

    fn valid_in_head(head: &WriteHead, into: &mut impl Extend<Snapshot>) {
        struct Collector<T>(T);

        impl<T, V> Collect<T> for Collector<&'_ mut V>
        where
            V: Extend<T>,
        {
            fn insert_one(&mut self, val: T) -> bool {
                self.0.extend(core::iter::once(val));
                true
            }
        }

        // Relaxed ordering is enough since we're the only reader still.
        head.iter_valid(&mut Collector(into), Ordering::Relaxed);
    }

    fn retain_in_head(head: &WriteHead, into: &dyn super::RetainSnapshot) {
        struct Retain<'lt>(&'lt dyn super::RetainSnapshot);

        impl Collect<Snapshot> for Retain<'_> {
            fn insert_one(&mut self, val: Snapshot) -> bool {
                self.0.contains(&val)
            }
        }

        head.iter_valid(&mut Retain(into), Ordering::Relaxed);
    }

    pub(crate) fn read(&self, snapshot: &Snapshot, into: &mut [u8]) {
        self.head.read(snapshot, into);
    }

    pub(crate) fn read_at(&self, snapshot: &Snapshot, into: &mut [u8], cfg: &ConfigureFile) {
        let mut alternate_head = WriteHead {
            cache: HeadCache { ..self.head.cache },
            ..self.head
        };

        Self::configure_head(&mut alternate_head, cfg);
        alternate_head.read(snapshot, into);
    }

    /// Construct this wrapper
    pub(crate) fn from_map(file: MmapRaw) -> Self {
        /// The head page we simulate if the file is too small to contain anything.
        ///
        /// The user will just notice that we can't write, but the construction itself won't fail.
        /// That happens later when the head is converted to a writer and the caller selected some
        /// minimum requirements. Here we just fulfill validity.
        static FALLBACK_HEAD: HeadPage = HeadPage {
            version: AtomicU64::new(ConfigureFile::MAGIC_VERSION),
            entry_mask: AtomicU64::new(0),
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
                    tail: &[],
                }
            }
        } else {
            WriteHead {
                cache: HeadCache::new(),
                meta: &FALLBACK_HEAD,
                data: &[],
                sequence: &[],
                tail: &[],
            }
        };

        Head { head, file }
    }

    pub(crate) fn tail(&self) -> &'_ [AtomicU64] {
        DataPage::as_slice_of_u64(self.head.tail)
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

impl ConfigureFile {
    pub(crate) const MAGIC_VERSION: u64 = 0x96c2_a6f4b68519b3;

    /// Is the configuration data complete?
    pub fn is_initialized(&self) -> bool {
        self.layout_version == Self::MAGIC_VERSION
    }

    /// Complete this configuration, if it is not already.
    pub fn or_insert_with(&mut self, replace: impl FnOnce(&mut Self)) {
        if !self.is_initialized() {
            replace(self);
            self.layout_version = ConfigureFile::MAGIC_VERSION;
        }
    }
}

impl Head {
    pub(crate) fn write_with(
        &mut self,
        data: &[u8],
        intermediate: &mut dyn FnMut(PreparedTransaction) -> bool,
    ) -> Result<u64, ()> {
        let mut entry = self.head.entry();
        let Some(end_ptr) = entry.new_write_offset(data.len()) else {
            return Err(());
        };

        entry.invalidate_heads(end_ptr);
        entry.copy_from_slice(data);

        if intermediate(PreparedTransaction {
            offset: entry.offset,
            length: entry.length,
            tail: entry.head.tail,
            head: entry.head,
        }) {
            Ok(entry.commit())
        } else {
            Err(())
        }
    }
}

impl WriteHead {
    pub(crate) fn pre_configure_entries(&mut self, num: u64) {
        assert!(num.next_power_of_two() == num);
        self.cache.entry_mask = num - 1;
    }

    pub(crate) fn pre_configure_pages(&mut self, num: u64) {
        assert!(num.next_power_of_two() == num);
        self.cache.page_mask = num - 1;
    }

    pub(crate) fn pre_configure_write(&mut self, offset: u64) {
        self.cache.page_write_offset = offset;
    }

    pub(crate) fn configure_pages(&mut self) {
        assert_eq!(
            core::mem::size_of::<DataPage>(),
            core::mem::size_of::<SequencePage>()
        );

        let sequence: usize = (self.cache.entry_mask + 1)
            .try_into()
            .expect("Invalid configured entry mask");
        let sequence = sequence.next_power_of_two();

        let data: usize = (self.cache.page_mask + 1)
            .try_into()
            .expect("Invalid configured page mask");
        let data = data.next_power_of_two();

        let psequence = sequence / SequencePage::DATA_COUNT
            + usize::from(sequence % SequencePage::DATA_COUNT != 0);
        let pdata = data / core::mem::size_of::<DataPage>()
            + usize::from(data % core::mem::size_of::<DataPage>() != 0);

        self.sequence = &self.sequence[..psequence];
        let (data, tail) = self.data[psequence..].split_at(pdata);
        self.data = data;
        self.tail = tail;

        self.meta
            .entry_mask
            .store(self.cache.entry_mask, Ordering::Relaxed);
        self.meta
            .page_mask
            .store(self.cache.page_mask, Ordering::Relaxed);
        self.meta
            .page_write_offset
            .store(self.cache.page_write_offset, Ordering::Relaxed);

        self.meta
            .version
            .store(ConfigureFile::MAGIC_VERSION, Ordering::Release);
    }

    pub(crate) fn entry(&mut self) -> Entry<'_> {
        let index = self.cache.entry_write_offset;
        let offset = self.cache.page_write_offset;
        Entry {
            head: self,
            length: 0,
            index,
            offset,
        }
    }

    pub(crate) fn iter_valid(&self, extend: &mut dyn Collect<Snapshot>, ordering: Ordering) {
        // Always use the stored one. If we're iterating a pre-loaded file then this is the one
        // stored from the previous run, or zeroed if new. If we're iterating over our current
        // writer then we've previously written it, i.e. the ordering here is always good too, no
        // matter which one is used precisely.
        let max = self.meta.entry_mask.load(ordering);
        let seqs = self.sequence.iter().flat_map(|seq| &seq.data);

        for (idx, seq) in seqs.enumerate() {
            if idx as u64 > max {
                break;
            }

            let length = seq.length.load(ordering);

            if length == 0 {
                continue;
            }

            if !extend.insert_one(Snapshot {
                length,
                offset: seq.offset.load(ordering),
            }) {
                seq.length.store(0, ordering);
            }
        }
    }

    pub(crate) fn new_write_offset(&self, n: usize) -> Option<u64> {
        let len = u64::try_from(n);
        if let Some(len) = len.ok().filter(|&l| l <= self.cache.page_mask) {
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

    pub(crate) fn copy_from_slice(&mut self, data: &[u8]) -> u64 {
        let mut n = self.cache.page_write_offset;

        for (&b, idx) in data.iter().zip(n..) {
            self.write_at(idx, b);
            n = n.wrapping_add(1);
        }

        let count = n.wrapping_sub(self.cache.page_write_offset);
        self.cache.page_write_offset = n;
        count
    }

    pub(crate) fn read(&self, snapshot: &Snapshot, into: &mut [u8]) {
        for (b, offset) in into.iter_mut().zip(0..snapshot.length) {
            let idx = snapshot.offset.wrapping_add(offset);
            *b = self.read_at(idx);
        }
    }

    fn get_entry_atomic(&self, idx: u64) -> &SequenceEntry {
        let idx = (idx & self.cache.entry_mask) as usize;

        let page = idx / SequencePage::DATA_COUNT;
        let entry = idx % SequencePage::DATA_COUNT;

        &self.sequence[page].data[entry]
    }

    fn invalidate_at(&mut self, idx: u64) -> u64 {
        let entry = self.get_entry_atomic(idx);
        entry.length.swap(0, Ordering::Relaxed)
    }

    fn insert_at(&mut self, idx: u64, snap: Snapshot) {
        let entry = self.get_entry_atomic(idx);

        entry.offset.store(snap.offset, Ordering::Release);
        entry.length.store(snap.length, Ordering::Release);
    }

    fn entry_at_relaxed(&self, idx: u64) -> Snapshot {
        let entry = self.get_entry_atomic(idx);

        Snapshot {
            offset: entry.offset.load(Ordering::Relaxed),
            length: entry.length.load(Ordering::Relaxed),
        }
    }

    fn idx_at(&self, idx: u64) -> (usize, usize, u32) {
        let idx = idx & self.cache.page_mask;

        let offset = idx % 8;
        let idx = idx / 8;
        let shift = 8 * offset;

        let data_idx = idx as usize % DataPage::DATA_COUNT;
        let page_idx = idx as usize / DataPage::DATA_COUNT;
        (page_idx, data_idx, shift as u32)
    }

    fn write_at(&self, idx: u64, byte: u8) {
        let (page_idx, data_idx, shift) = self.idx_at(idx);
        let word = &self.data[page_idx].data[data_idx];
        let mask = 0xffu64 << shift;

        let old = word.load(Ordering::Relaxed) & !mask;
        let new = old | (u64::from(byte) << shift);
        word.store(new, Ordering::Relaxed);
    }

    fn read_at(&self, idx: u64) -> u8 {
        let (page_idx, data_idx, shift) = self.idx_at(idx);

        let word = &self.data[page_idx].data[data_idx];
        let old = word.load(Ordering::Relaxed);

        ((old >> shift) & 0xff) as u8
    }
}

impl Entry<'_> {
    /// Consume the entry, putting it into the sequence buffer.
    pub(crate) fn commit(self) -> u64 {
        let end = self.head.cache.page_write_offset;
        self.head
            .meta
            .page_write_offset
            .store(end, Ordering::Relaxed);

        debug_assert!(
            end.wrapping_sub(self.offset) >= self.length,
            "Failed to reserve enough space in the data section for the entry, risking corrupted data with following writes"
        );

        self.head.insert_at(
            self.index,
            Snapshot {
                length: self.length,
                offset: self.offset,
            },
        );

        self.index
    }

    pub(crate) fn new_write_offset(&self, n: usize) -> Option<u64> {
        self.head.new_write_offset(n)
    }

    pub(crate) fn invalidate_heads(&mut self, end: u64) {
        self.head.invalidate_heads_to(end);
    }

    pub(crate) fn copy_from_slice(&mut self, data: &[u8]) {
        self.length += self.head.copy_from_slice(data);
    }
}

impl<'lt> PreparedTransaction<'lt> {
    pub fn replace(&mut self, data: &[u8]) {
        assert!(
            data.len() as u64 <= self.length,
            "{} > {}",
            data.len(),
            self.length
        );
        let mut n = self.offset;

        for (&b, idx) in data.iter().zip(n..) {
            self.head.write_at(idx, b);
            n = n.wrapping_add(1);
        }
    }

    pub fn tail(&self) -> &'lt [AtomicU64] {
        DataPage::as_slice_of_u64(self.tail)
    }
}

pub(crate) struct HeadCache {
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

#[derive(Default)]
pub(crate) struct HeadPage {
    /// Magic 8-byte sequence, denoting the layout of this file and identifying it as shm-snapshot.
    version: AtomicU64,
    /// The mask to translate stream index to a specific descriptor offset.
    entry_mask: AtomicU64,
    /// The mask to translate stream offset to a data page offset.
    page_mask: AtomicU64,
    /// The stream offset of the next byte to write.
    page_write_offset: AtomicU64,
}

impl HeadPage {
    const PAGE_SZ: usize = 4096;
}

pub(crate) struct SequencePage {
    data: [SequenceEntry; Self::DATA_COUNT],
}

struct SequenceEntry {
    offset: AtomicU64,
    length: AtomicU64,
}

impl Default for SequencePage {
    fn default() -> Self {
        SequencePage {
            data: [0; Self::DATA_COUNT].map(|_i| SequenceEntry {
                offset: AtomicU64::new(0),
                length: AtomicU64::new(0),
            }),
        }
    }
}

impl SequencePage {
    // FIXME: I currently don't target 32-bit atomic targets. But if then this should depend on
    // such a target choice. The code written should then also get another implementation, and
    // `Writer` only access this by indirection.
    const DATA_COUNT: usize = 4096 / 16;
}

pub struct DataPage {
    pub data: [AtomicU64; Self::DATA_COUNT],
}

impl DataPage {
    // One AtomicU64 per entry dividing the page.
    const DATA_COUNT: usize = 4096 / 8;

    pub fn as_slice_of_u64(this: &[DataPage]) -> &[AtomicU64] {
        let count = Self::DATA_COUNT * this.len();
        unsafe { &*core::ptr::slice_from_raw_parts(this.as_ptr() as *const AtomicU64, count) }
    }
}

impl Default for DataPage {
    fn default() -> Self {
        DataPage {
            data: [0; Self::DATA_COUNT].map(|_i| AtomicU64::new(0)),
        }
    }
}
