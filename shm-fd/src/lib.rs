#![cfg_attr(not(feature = "std"), no_std)]

use core::ffi::c_int as RawFd;
extern crate alloc;

pub mod op;
mod listenfd;

pub use listenfd::{ListenFd, ListenInit};

/// A raw file descriptor, opened for us by the environment.
///
/// The code does assume to own it, but it won't close the file descriptor.
pub struct SharedFd {
    fd: RawFd,
}

impl SharedFd {
    /// Import a shared file descriptor based on environment variable `SHM_SHARED_FDS`.
    ///
    /// # Safety
    /// Caller asserts that the environment variable has been set to a file descriptor that is not
    /// owned by any other resource.
    #[cfg(all(feature = "std", feature = "libc"))]
    pub unsafe fn from_env() -> Option<Self> {
        let listen = ListenFd::new()?.ok()?;
        Self::from_listen(&listen)
    }

    /// Import a shared file descriptor based on the contents that would be in the environment variable `SHM_SHARED_FDS`.
    #[cfg(all(feature = "libc"))]
    pub unsafe fn from_listen(var: &ListenFd) -> Option<Self> {
        let num = var.names.iter().position(|v|v == "SHM_SHARED_FD")?;
        let fd: RawFd = var.fd_base + num as RawFd;

        let mut statbuf = unsafe { core::mem::zeroed::<libc::stat>() };
        if -1 == unsafe { libc::fstat(fd, &mut statbuf) } {
            // FIXME: Report that error?
            return None;
        }

        Some(SharedFd { fd })
    }

    /// Open the file descriptor.
    ///
    /// This can fail if for some reason the file descriptor does not refer to an anonymous memory
    /// file.
    #[cfg(all(feature = "memfile", feature = "std"))]
    pub fn into_file(self) -> Result<memfile::MemFile, std::io::Error> {
        let fd = self.into_raw_fd();
        // It's not necessary to preserve the file descriptor here.
        // It can be restored in any case.
        memfile::MemFile::from_file(fd).map_err(|err| err.into_error())
    }

    pub fn as_raw_fd(&self) -> RawFd {
        self.fd
    }

    pub fn into_raw_fd(self) -> RawFd {
        let _this = core::mem::ManuallyDrop::new(self);
        _this.fd
    }
}

#[cfg(feature = "std")]
impl std::os::unix::io::AsRawFd for SharedFd {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}
