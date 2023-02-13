//! Owns a file descriptor with known size.
use shm_fd::{SharedFd, Shm, Stat};

use crate::MapError;

/// An owned file descriptor, with all information about the size of the object.
pub struct AreaFd {
    pub(crate) fd: SharedFd,
    /// The stat of the area.
    stat: Stat,
    /// the usable length in the address space representation.
    len: usize,
}

impl AreaFd {
    pub fn new(fd: SharedFd, shm: &Shm) -> Result<Self, MapError> {
        // FIXME: return the actual status code?
        let stat = shm.stat(&fd).map_err(|_| MapError(11))?;
        let len = usize::try_from(stat.st_size).map_err(|_| MapError(11))?;
        Ok(AreaFd { fd, stat, len })
    }

    pub fn len(&self) -> usize {
        self.len
    }
}
