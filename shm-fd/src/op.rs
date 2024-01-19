use super::SharedFd;
use core::ffi::c_int;
use alloc::sync::Arc;

/// Interact with `shm*` and related calls.
#[allow(dead_code)]
pub struct Shm {
    inner: Arc<ShmInner>,
}

#[allow(dead_code)]
struct ShmInner {
    vtable: ShmVTable,
}

/// An error returned when interaction with a shared memory file.
#[allow(dead_code)]
pub struct ShmError(c_int);

/// *Fixed* type, not platform dependent.
type OffT = i64;
type BlkSizeT = i64;
type BlkCntT = i64;
type TimeT = i64;

#[non_exhaustive]
#[derive(Default)]
pub struct Stat {
    pub st_mode: u32,
    pub st_uid: u32,
    pub st_gid: u32,
    pub st_size: OffT,
    pub st_blksize: BlkSizeT,
    pub st_blocks: BlkCntT,
    pub st_atime: TimeT,
    pub st_atime_nsec: i64,
    pub st_mtime: TimeT,
    pub st_mtime_nsec: i64,
    pub st_ctime: TimeT,
    pub st_ctime_nsec: i64,
}

/// A table of OS functions.
///
/// This enumerates the functions required to interact with the `SharedFd` object. A vtable must
/// contain functions that behave according the POSIX/libc's specification of the correspondingly
/// named functions, see Safety precondition of [`Shm::new_unchecked`]. Note that a default
/// table can be initialized when linking against `libc`.
///
/// You're encouraged to provide your own objects here instead of hooking the functions themselves
/// with override/linker tricks.
#[non_exhaustive]
pub struct ShmVTable {
    pub fstat: fn(c_int, Option<&mut Stat>) -> c_int,
    pub close: fn(c_int) -> c_int,
    pub errno: fn() -> c_int,
}

#[allow(dead_code)]
impl Shm {
    /// Create an `Shm` from a customized vtable.
    ///
    /// # Safety
    ///
    /// The VTable must contain a correct pair of functions that implement the `shm*` interface.
    pub unsafe fn new_unchecked(vtable: ShmVTable) -> Self {
        Shm {
            inner: Arc::new(ShmInner { vtable }),
        }
    }
    #[cfg(feature = "libc")]
    pub fn new() -> Self {
        unsafe {
            Self::new_unchecked(ShmVTable::new_libc())
        }
    }

    pub fn stat(&self, shared: &SharedFd) -> Result<Stat, ShmError> {
        let mut stat = Stat::default();
        let inner = (self.inner.vtable.fstat)(shared.fd, Some(&mut stat));

        if inner < 0 {
            return Err(ShmError((self.inner.vtable.errno)()));
        } else {
            Ok(stat)
        }
    }
}

impl ShmVTable {
    #[cfg(feature = "libc")]
    pub fn new_libc() -> Self {
        fn _fstat(fd: c_int, stat: Option<&mut Stat>) -> c_int {
            let mut uninit = core::mem::MaybeUninit::<libc::stat>::zeroed();
            // Safety: passing the correct pointer to a struct of libc::stat.
            let ret = unsafe { libc::fstat(fd, uninit.as_mut_ptr()) };

            if ret == 0 {
                // Safety: always initialized on return with success.
                let lstat = unsafe { uninit.assume_init() };
                if let Some(stat) = stat {
                    *stat = Stat {
                        st_mode: lstat.st_mode,
                        st_uid: lstat.st_uid,
                        st_gid: lstat.st_gid,
                        st_size: lstat.st_size,
                        st_blksize: lstat.st_blksize,
                        st_blocks: lstat.st_blocks,
                        st_atime: lstat.st_atime,
                        st_atime_nsec: lstat.st_atime_nsec,
                        st_mtime: lstat.st_mtime,
                        st_mtime_nsec: lstat.st_mtime_nsec,
                        st_ctime: lstat.st_ctime,
                        st_ctime_nsec: lstat.st_ctime_nsec,
                    };
                };
            }

            ret
        }

        fn _close_inner(fd: c_int) -> c_int {
            unsafe { libc::close(fd) }
        }

        fn _errno() -> c_int {
            unsafe { *libc::__errno_location() }
        }

        ShmVTable {
            fstat: _fstat,
            close: _close_inner,
            errno: _errno,
        }
    }
}
