//! Interact with a memory-mapped file in the systemd File Descriptor store, for snapshot-restore of some state.
#[cfg(test)]
mod tests;
mod writer;

pub use writer::{ConfigureFile, File, FileDiscovery, PreparedTransaction, Snapshot, Writer};
use writer::Head;

use core::sync::atomic::AtomicU64;
use memmap2::MmapRaw;

/// The index of a snapshot in a file wrapped with a [`Writer`].
///
/// Requires the file metadata (the size of the entry ring) to determine a precise memory offset in
/// the file. This index does not guarantee that a snapshot is, or will stay, valid.
#[derive(Debug)]
pub struct SnapshotIndex {
    /// The entry index at which we have in fact committed. Not clear what use it would be to make
    /// this number available but we want to debug the struct anyways.
    #[allow(dead_code)]
    entry: u64,
}

/// A value that can decide whether a snapshot should be considered valid.
///
/// This is commonly used when it is necessary to identify snapshots which have not been changed,
/// in `FileDiscovery::retain`. Any modification to the data range covered by a snapshot is
/// preceded by an invalidation of said snapshot. If the same snapshot is observed as valid
/// multiple times, then the observer is guaranteed its data was not modified between the
/// observations.
///
/// The strategy need not be sensitive, false negatives should be considered correct, but can lead
/// to suboptimal behavior (i.e. discard of snapshots, higher error rate of the strategy used to
/// make snapshots).
pub trait RetainSnapshot {
    fn contains(&self, snapshot: &Snapshot) -> bool;
}

impl RetainSnapshot for std::collections::HashSet<Snapshot> {
    fn contains(&self, snapshot: &Snapshot) -> bool {
        self.contains(snapshot)
    }
}

impl RetainSnapshot for Vec<Snapshot> {
    fn contains(&self, snapshot: &Snapshot) -> bool {
        self.iter().position(|x| x == snapshot).is_some()
    }
}

/// An error, trying to commit a snapshot with [`Writer::commit`].
pub struct WriterCommitError {
    _inner: (),
}

impl File {
    pub fn new<T: std::os::unix::io::AsRawFd>(fd: T) -> Result<Self, std::io::Error> {
        let file = MmapRaw::map_raw(&fd)?;
        let head = Head::from_map(file);
        Ok(File { head })
    }

    /// Attempt to recover the configuration from existing data.
    ///
    /// This method writes the read information into the output argument `cfg` and returns a proxy
    /// with the recovered configuration. The proxy can be used to partially access the contained
    /// entries as well, if the discovery succeeded.
    pub fn recover(&self, cfg: &mut ConfigureFile) -> Option<FileDiscovery<'_>> {
        self.head.discover(cfg);

        if !cfg.is_initialized() {
            return None;
        }

        Some(FileDiscovery {
            file: self,
            configuration: ConfigureFile { ..*cfg },
        })
    }

    /// Change the metadata of the file, to the one described in the configuration.
    pub fn configure(mut self, cfg: &ConfigureFile) -> Writer {
        self.head.configure(cfg);
        self.into_writer_unguarded()
    }

    /// Convert this into a writer, without minding data consistency.
    pub fn into_writer_unguarded(self) -> Writer {
        Writer { head: self.head }
    }
}

impl FileDiscovery<'_> {
    /// Read data described by a snapshot, with discovered metadata in the file.
    pub fn read(&self, snapshot: &Snapshot, buffer: &mut [u8]) {
        self.file.head.read_at(snapshot, buffer, &self.configuration)
    }

    /// Iteratively read all valid entries from the file.
    ///
    /// The order of reads is not guaranteed. Internally we have a structure equivalent to a ring
    /// buffer (similar to `VecDeque`) and likely are iterating in the order of the underlying raw
    /// slice, not the order of the actual logical data layout.
    ///
    /// More specific interfaces for external iteration with an iterator may be added. Send changes
    /// if you have an implementation.
    #[inline(always)]
    pub fn valid(&self, into: &mut impl Extend<Snapshot>) {
        self.file.head.valid_at(into, &self.configuration)
    }

    /// Invalidate some entries, as determined by the retained configuration.
    ///
    /// For instance, delete snapshots which are known to have been potentially invalidated by
    /// modifications into the covered memory.
    pub fn retain(&self, retain: &dyn RetainSnapshot) {
        self.file.head.retain_at(retain, &self.configuration);
    }
}

/// Public interface of the writer.
impl Writer {
    /// Insert some data into the atomic log of the shared memory.
    pub fn commit(&mut self, data: &[u8]) -> Result<SnapshotIndex, WriterCommitError> {
        match self.head.write_with(data, &mut |_tx| true)  {
            Ok(entry) => Ok(SnapshotIndex { entry }),
            Err(_) => Err(WriterCommitError { _inner: () })
        }
    }

    /// Insert some data into the atomic log of the shared memory.
    ///
    /// This also invokes a function such that it's effects are sequenced after the reservation of
    /// the new slot but before committing the data. The function can also introduce changes that
    /// appear correctly from the semantics view of the ring. Changes to the tail can be made via
    /// the passed `PreparedTransaction` object.
    pub fn commit_with<T>(
        &mut self,
        data: &[u8],
        intermediate: impl FnOnce(PreparedTransaction) -> Option<T>
    ) -> Result<(SnapshotIndex, T), WriterCommitError> {
        let mut dropped = Some(intermediate);
        let mut result = None;
        let ref mut result_ref = result;

        let mut intermediate = move |tx: PreparedTransaction<'_>| {
            dropped.take().map_or(false, |fn_| {
                if let Some(val) = fn_(tx) {
                    *result_ref = Some(val);
                    true
                } else {
                    false
                }
            })
        };

        match self.head.write_with(data, &mut intermediate)  {
            Ok(entry) => {
                let val = result.expect("written when returning `true`");
                Ok((SnapshotIndex { entry }, val))
            },
            Err(_) => Err(WriterCommitError { _inner: () })
        }
    }

    /// Read data described by a snapshot, with discovered metadata in the file.
    pub fn read(&self, snapshot: &Snapshot, buffer: &mut [u8]) {
        self.head.read(snapshot, buffer);
    }

    /// Collect all currently valid snapshot entries.
    #[inline(always)]
    pub fn valid(&self, into: &mut impl Extend<Snapshot>) {
        self.head.valid(into)
    }

    /// Access the tail of the underlying shared memory file.
    ///
    /// This refers to the portion of the file after the header, the entry ring, and the data ring
    /// buffer. This data can not be referenced by an entry directly and belongs to arbitrary use
    /// by the caller.
    pub fn tail(&self) -> &[AtomicU64] {
        self.head.tail()
    }
}

impl core::fmt::Debug for WriterCommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriterCommitError").finish()
    }
}

impl core::fmt::Display for WriterCommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Failed to commit snapshot data")
    }
}
