//! Owns a file descriptor with known size.
use crate::{MapError, Mapper};
use core::sync::atomic::AtomicU32;

use shm_fd::{SharedFd, Shm, Stat};

/// An owned file descriptor, with all information about the size of the object.
pub struct AreaFd {
    pub(crate) fd: SharedFd,
    /// The stat of the area.
    stat: Stat,
    /// the usable length in the address space representation.
    len: usize,
}

/// An owned file descriptor and its corresponding, memory-mapped region.
pub struct MappedFd {
    area: AreaFd,
    mapper: Mapper,
    mapping: &'static [AtomicU32],
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

impl MappedFd {
    /// Create by mapping _all_ memory of the file descriptor at an arbitrary new location.
    pub fn new(mapper: Mapper, area: AreaFd) -> Result<Self, MapError> {
        let mapping = mapper.mmap_shared(area.fd.as_raw_fd(), area.len())?;

        Ok(MappedFd {
            area,
            mapper,
            mapping,
        })
    }

    /// Get a copy of the inner mapping.
    ///
    /// # Safety
    ///
    /// The caller must not use this after the mapped fd has been dropped or otherwise closed.
    pub(crate) unsafe fn get_unchecked(&self) -> &'static [AtomicU32] {
        self.mapping
    }
}

impl Drop for MappedFd {
    fn drop(&mut self) {
        let mmap = core::mem::take(&mut self.mapping);
        // Safety: no more references to this region of memory.
        unsafe { self.mapper.munmap(mmap, self.area.len()) };
    }
}
