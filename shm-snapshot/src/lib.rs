#[cfg(test)]
mod tests;
mod writer;

pub use writer::{ConfigureFile, DataPage, File, PreparedTransaction, Snapshot, Writer};
use writer::Head;

use memmap2::MmapRaw;

#[derive(Debug)]
pub struct Commit {
    /// The entry index at which we have in fact committed. Not clear what use it would be to make
    /// this number available but we want to debug the struct anyways.
    #[allow(dead_code)]
    entry: u64,
}

pub struct CommitError {
    _inner: (),
}

impl File {
    pub fn new<T: std::os::unix::io::AsRawFd>(fd: T) -> Result<Self, std::io::Error> {
        let file = MmapRaw::map_raw(&fd)?;
        let head = Head::from_map(file);
        Ok(File { head })
    }

    #[inline(always)]
    pub fn valid(&self, into: &mut impl Extend<Snapshot>) {
        self.head.valid(into)
    }

    pub fn discover(&mut self, cfg: &mut ConfigureFile) {
        self.head.discover(cfg)
    }

    pub fn configure(mut self, cfg: &ConfigureFile) -> Writer {
        self.head.configure(cfg);
        self.into_writer_unguarded()
    }

    /// Convert this into a writer, without minding data consistency.
    pub fn into_writer_unguarded(self) -> Writer {
        Writer { head: self.head }
    }
}

/// Public interface of the writer.
impl Writer {
    /// Insert some data into the atomic log of the shared memory.
    pub fn write(&mut self, data: &[u8]) -> Result<Commit, CommitError> {
        match self.head.write_with(data, &mut |_tx| true)  {
            Ok(entry) => Ok(Commit { entry }),
            Err(_) => Err(CommitError { _inner: () })
        }
    }

    /// Insert some data into the atomic log of the shared memory.
    ///
    /// This also invokes a function before committing the data.
    pub fn write_with(
        &mut self,
        data: &[u8],
        intermediate: impl FnOnce(PreparedTransaction) -> bool
    ) -> Result<Commit, CommitError> {
        let mut dropped = Some(intermediate);
        let mut intermediate = move |tx: PreparedTransaction<'_>| {
            dropped.take().map_or(false, |fn_| fn_(tx))
        };

        match self.head.write_with(data, &mut intermediate)  {
            Ok(entry) => Ok(Commit { entry }),
            Err(_) => Err(CommitError { _inner: () })
        }
    }

    #[inline(always)]
    pub fn valid(&self, into: &mut impl Extend<Snapshot>) {
        self.head.valid(into)
    }
}

impl ConfigureFile {
    pub fn or_insert_with(&mut self, replace: impl FnOnce(&mut Self)) {
        if !self.is_initialized() {
            replace(self)
        }
    }
}

impl core::fmt::Debug for CommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommitError").finish()
    }
}

impl core::fmt::Display for CommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Failed to commit snapshot data")
    }
}
