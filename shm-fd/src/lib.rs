use memfile::MemFile;
use std::os::unix::io::RawFd;

pub struct SharedFd {
    fd: RawFd,
}

impl SharedFd {
    /// Import a shared file descriptor based on environment variable `SHM_SHARED_FDS`.
    ///
    /// # Safety
    /// Caller asserts that the environment variable has been set to a file descriptor that is not
    /// owned by any other resource.
    pub unsafe fn from_env() -> Option<Self> {
        let var = std::env::var_os("SHM_SHARED_FDS")?;
        let num = var.to_str()?.split(',').next()?;
        let fd: i32 = num.parse().ok()?;
        Some(SharedFd { fd: RawFd::from(fd) })
    }

    /// Open the file descriptor.
    ///
    /// This can fail if for some reason the file descriptor does not refer to an anonymous memory
    /// file.
    pub fn into_file(self) -> Result<MemFile, std::io::Error> {
        // It's not necessary to preserve the file descriptor here.
        // It can be restored in any case.
        MemFile::from_file(self.fd).map_err(|err| err.into_error())
    }
}

impl std::os::unix::io::AsRawFd for SharedFd {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}
