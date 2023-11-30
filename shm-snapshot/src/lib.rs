#[cfg(test)]
mod tests;
mod writer;

pub use writer::{File, Snapshot, Writer};
use writer::Head;

use shm_fd::SharedFd;
use memmap2::MmapRaw;

pub struct Commit(u64);

pub struct CommitError {
    _inner: (),
}

impl File {
    pub fn new(fd: SharedFd) -> Result<Self, std::io::Error> {
        let file = MmapRaw::map_raw(&fd)?;
        let head = Head::from_map(file);
        Ok(File { head })
    }

    #[inline(always)]
    pub fn valid(&self, into: impl Extend<Snapshot>) {
        self.head.valid(into)
    }

    pub fn into_writer(self) -> Writer {
        Writer { head: self.head }
    }
}

/// Public interface of the writer.
impl Writer {
    pub fn write(&mut self, data: &[u8]) -> Result<Commit, CommitError> {
        match self.head.write(data)  {
            Ok(n) => Ok(Commit(n)),
            Err(_) => Err(CommitError { _inner: () })
        }
    }

    #[inline(always)]
    pub fn valid(&self, into: impl Extend<Snapshot>) {
        self.head.valid(into)
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
