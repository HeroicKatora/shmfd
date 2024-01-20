#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]
/// Interact with the systemd File Descriptor store (or equivalent).
///
/// This crate implements pure Rust wrappers to access a set of environment variables describing
/// file descriptors pre-opened by the parent process. This is used to pass the kernel state
/// maintained by a service manager to the service. Commonly, this may be a network socket to avoid
/// connection loss on restarts, or a shared memory area to retain in-memory state across restarts.
///
/// The crate only captures mainly the primary file-descriptor mechanism. Higher level semantics
/// such as interpreting the contents of a memory file or socket are left to other library
/// components.
///
/// ## Binary Target
///
/// The crate also defines a binary target. This program serves as example and can be utilized as a
/// wrapper binary to bring up the File Descriptor store on a cold restart. It makes sure a shm
/// file exists as a named file descriptor in the store.
use core::ffi::c_int as RawFd;

extern crate alloc;

mod listenfd;
// FIXME: tried, but not as useful as intended. There are a few types we use in interfaces and
// representations which would have to be modelled, too (for the std::env::var_os and for
// libc::AF_UNIX / libc::sendmsg mostly).
//
// Hence, this module is private for now until that representation is figured out.
mod op;
#[cfg(all(feature = "std", feature = "libc"))]
mod notifyfd;

pub use listenfd::{ListenFd, ListenInit};
#[cfg(all(feature = "std", feature = "libc"))]
pub use notifyfd::NotifyFd;

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

        if -1 == (op::ShmVTable::new_libc().fstat)(fd, None) {
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

    /// Grab the raw fie descriptor.
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
