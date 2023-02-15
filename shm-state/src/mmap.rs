use alloc::sync::Arc;
use core::ffi::{c_int, c_void};
use core::sync::atomic::AtomicU32;

pub struct VTable {
    /// Simplified `mmap`.
    pub mmap: fn(len: usize, prot: c_int, file: c_int) -> *mut c_void,
    pub munmap: fn(*mut c_void, usize) -> c_int,
    pub errno: fn() -> c_int,

    pub prot_read: c_int,
    pub prot_write: c_int,
    pub map_failed: *mut c_void,
}

#[derive(Clone)]
pub struct Mapper {
    inner: Arc<Inner>,
}

#[derive(Clone, Debug)]
pub struct MapError(pub(crate) c_int);

struct Inner {
    vtable: VTable,
}

impl Mapper {
    /// Create a `Mapper` from a customized vtable.
    ///
    /// # Safety
    ///
    /// The VTable must contain a correct pair of functions that implement the `mmap` interface.
    pub unsafe fn new_unchecked(vtable: VTable) -> Self {
        Mapper {
            inner: Arc::new(Inner { vtable }),
        }
    }

    #[cfg(feature = "libc")]
    pub fn new() -> Self {
        fn _mmap_inner(len: usize, prot: c_int, file: c_int) -> *mut c_void {
            unsafe { libc::mmap(core::ptr::null_mut(), len, prot, libc::MAP_SHARED, file, 0) }
        }

        fn _munmap(addr: *mut c_void, len: usize) -> c_int {
            unsafe { libc::munmap(addr, len) }
        }

        fn _errno() -> c_int {
            unsafe { *libc::__errno_location() }
        }

        unsafe {
            Self::new_unchecked(VTable {
                mmap: _mmap_inner,
                munmap: _munmap,
                errno: _errno,
                prot_read: libc::PROT_READ,
                prot_write: libc::PROT_WRITE,
                map_failed: libc::MAP_FAILED,
            })
        }
    }

    pub fn mmap_shared(&self, file: c_int, len: usize) -> Result<&'static [AtomicU32], MapError> {
        let prot = self.inner.vtable.prot_read | self.inner.vtable.prot_write;
        let ptr = (self.inner.vtable.mmap)(len, prot, file);

        if ptr == self.inner.vtable.map_failed {
            return Err(MapError((self.inner.vtable.errno)()));
        }

        assert!((ptr as usize) % 4 == 0, "Unaligned mmap address chosen");
        let count = len / 4;

        // Safety:
        // * mmap returns valid memory
        // * memory _may_ be aliased, which is why we have atomics (external interior mutability).
        //   This is the best we can do and probably reasonable across processes.
        // * checked alignment requirement above.
        // * the mapping is leaked initially, i.e. has `'static` lifetime.
        Ok(unsafe { &*core::ptr::slice_from_raw_parts(ptr as *const AtomicU32, count) })
    }

    /// Deallocate a mapping created with `mmap_shared`.
    ///
    /// # Safety
    ///
    /// The memory denoted by `region` must not be aliased by any live reference. The same length
    /// must be passed that was used to map the region. It must be the same parameter as used in
    /// the `mmap_shared` call that previously returned the valid region.
    pub unsafe fn munmap(&self, region: *const [AtomicU32], len: usize) {
        (self.inner.vtable.munmap)(region as *mut _, len);
    }
}

impl core::ops::Deref for Mapper {
    type Target = VTable;

    fn deref(&self) -> &Self::Target {
        &self.inner.vtable
    }
}
