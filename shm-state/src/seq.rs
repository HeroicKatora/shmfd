//! A primitive sequential log.
use crate::{
    area::MappedFd,
    ring::{DescriptorIdx, RingMapped},
    Descriptor, Ring,
};
use core::sync::atomic::Ordering;

pub struct Seq {
    inner: SeqInner,
    // Hmpf, if we used `Arc` for this and kept it within the `SeqInner.ring` then we wouldn't have
    // this problem. Also it would solve the safety complexity. But an allocation..
    #[allow(dead_code)]
    mapfd: MappedFd,
}

pub struct SeqOptions {
    /// The total buffer size to use.
    ///
    /// Must be a power-of-two, larger than 4.
    pub buffer: usize,
}

#[derive(Clone, Copy)]
struct Layout {
    data_offset: usize,
    buffer_mask: u32,
    tail: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeqError {
    /// The layout can not be used as it does not fulfill the invariants.
    InvalidLayout,
    /// The layout is okay, but the ring used is too small to fit the layout.
    UnfittingLayout,
    /// The layout can not be represented by this architecture / pointer size.
    BadArchitectureLayout,
    /// During `restore`, no snapshot was found to restore to.
    NoSnapshot,
    /// The capacity of the buffer could not fit the provided data.
    CapacityOverflow,
}

struct SeqInner {
    ring: RingMapped,
    layout: Layout,
    begin: u64,
    len: u32,
    descriptor: DescriptorIdx,
}

impl Seq {
    pub fn new(ring: Ring, options: &SeqOptions) -> Result<Self, SeqError> {
        // Safety: we drop the `ring` before `mapfd` in all paths. The path where it is passed to
        // `SeqInner` is critical but it won't be returned in the error so that `mapfd` surely
        // outlives this value. Otherwise they are returned and `mapfd` is finalized after the
        // `inner` attribute.
        let (ring, mapfd) = unsafe { ring.into_parts() };
        let inner = SeqInner::wrap(ring, options)?;
        Ok(Seq { inner, mapfd })
    }

    pub fn restore(&mut self) -> Result<u32, SeqError> {
        self.inner.restore()
    }

    pub fn set(&mut self, seq: &[u8]) -> Result<(), SeqError> {
        self.inner.set(seq)
    }

    pub fn get(&mut self, seq: &mut [u8]) -> Result<usize, SeqError> {
        self.inner.get(seq)
    }
}

impl SeqInner {
    pub(crate) fn wrap(ring: RingMapped, options: &SeqOptions) -> Result<Self, SeqError> {
        let layout = Self::layout_for(ring.tail().len(), options)?;
        Ok(SeqInner {
            ring,
            layout,
            begin: 0,
            len: 0,
            descriptor: DescriptorIdx(0),
        })
    }

    /// Try to initialized this store based on the shared memory state.
    ///
    /// If a prior state was found, `Some(_)` is returned with the number of bytes that the current
    /// state contains. Otherwise, `Err` is returned with the proper diagnostic. You may intend to
    /// match the variant `NoSnapshot` as a signal to initialize from scratch instead of an error.
    pub fn restore(&mut self) -> Result<u32, SeqError> {
        let last_descriptor = self.ring.restore().ok_or(SeqError::NoSnapshot)?;
        let offset_len = last_descriptor.payload;

        let begin = offset_len >> 32;
        let len = offset_len as u32;

        if len > self.layout.buffer_mask / 2 {
            return Err(SeqError::InvalidLayout);
        }

        self.begin = begin;
        self.len = len;

        Ok(self.len)
    }

    /// Change the current value.
    pub fn set(&mut self, seq: &[u8]) -> Result<(), SeqError> {
        let len = u32::try_from(seq.len()).map_err(|_| SeqError::InvalidLayout)?;

        // Guarantees we do not overwrite the previous value, which means one valid value is
        // preserved even when this update does not complete for any reason (crash, scheduled
        // away).
        if len > self.layout.buffer_mask / 2 {
            return Err(SeqError::InvalidLayout);
        }

        let begin = self.begin;
        let mut pos = self.begin;
        let mut iter = seq.chunks_exact(4);
        let data = &self.ring.tail()[self.layout.data_offset..];

        while let Some(ch) = iter.next() {
            let idx = pos & u64::from(self.layout.buffer_mask);
            let val = u32::from_ne_bytes(ch.try_into().unwrap());
            data[(idx >> 2) as usize].store(val, Ordering::Relaxed);
            pos += 4;
        }

        let tail = iter.remainder();

        if !tail.is_empty() {
            let idx = pos & u64::from(self.layout.buffer_mask);
            let mut bytes = [0; 4];
            bytes[..tail.len().min(4)].copy_from_slice(tail);
            let val = u32::from_ne_bytes(bytes);
            data[(idx >> 2) as usize].store(val, Ordering::Relaxed);
        }

        // Yes, we are shifting bits out but the buffer can not be larger than u32::MAX so these
        // bits are necessarily unused / masked away on access.
        let offset_len = (begin << 32) | u64::from(len);
        let new_idx = self.ring.push(Descriptor {
            start: 0,
            end: self.layout.tail as u64,
            payload: offset_len,
        });

        self.begin = begin;
        self.len = len;

        // This case should not be usually hit (we carefully do not overwrite the previous snapshot
        // which should still be alive). Except for the case where this is the _first_ write. In
        // this case, the descriptor may not actually point to a valid descriptor yet and this may
        // have been the one used for the push.
        if new_idx != self.descriptor {
            self.ring.invalidate(self.descriptor);
        }

        // Post-condition: the new descriptor is valid.
        self.descriptor = new_idx;

        Ok(())
    }

    /// Retrieve the current value.
    pub fn get(&mut self, seq: &mut [u8]) -> Result<usize, SeqError> {
        let mut iter = seq.chunks_exact_mut(4);
        let mut range = 0..self.len;
        let data = &self.ring.tail()[self.layout.data_offset..];

        while range.len() > 4 {
            if let Some(ch) = iter.next() {
                let idx =
                    (self.begin + u64::from(range.start)) & u64::from(self.layout.buffer_mask);
                let bytes = data[(idx >> 2) as usize]
                    .load(Ordering::Relaxed)
                    .to_ne_bytes();
                ch.copy_from_slice(&bytes);
            } else {
                break;
            }

            range.start = range.start + 4;
        }

        if !range.is_empty() {
            let idx = (self.begin + u64::from(range.start)) & u64::from(self.layout.buffer_mask);
            let bytes = data[(idx >> 2) as usize]
                .load(Ordering::Relaxed)
                .to_ne_bytes();

            let tail = iter.into_remainder();
            let tail_len = tail.len().min(4);
            tail.copy_from_slice(&bytes[..tail_len]);
        }

        Ok(self.len as usize)
    }

    fn layout_for(cnt: usize, options: &SeqOptions) -> Result<Layout, SeqError> {
        if !options.buffer.is_power_of_two() {
            return Err(SeqError::InvalidLayout);
        }

        if options.buffer < 4 {
            return Err(SeqError::InvalidLayout);
        }

        let buffer_mask =
            u32::try_from(options.buffer - 1).map_err(|_| SeqError::BadArchitectureLayout)?;

        let non_sharing_count = 256 / 4;

        let data_offset = cnt
            .checked_sub(non_sharing_count)
            .ok_or(SeqError::UnfittingLayout)?;

        let tail = data_offset
            .checked_sub(options.buffer / 4)
            .ok_or(SeqError::UnfittingLayout)?;

        Ok(Layout {
            data_offset,
            tail,
            buffer_mask,
        })
    }
}

#[test]
fn simple_seq() {
    use crate::ring::{RingMapped, RingOptions};
    use core::sync::atomic::AtomicU32;

    const INIT: AtomicU32 = AtomicU32::new(0);
    static REGION: [AtomicU32; 1 << 10] = [INIT; 1 << 10];

    let ropt = RingOptions { nr_descriptors: 2 };
    let sopt = SeqOptions { buffer: 1 << 7 };

    let ring = RingMapped::wrap(&REGION, &ropt).unwrap();
    let mut seq = SeqInner::wrap(ring, &sopt).unwrap();

    const HELLO: &[u8] = b"Hello, world!";

    seq.set(HELLO).unwrap();
    let mut buffer = [0; HELLO.len()];
    let retrieved = seq.get(&mut buffer);
    assert_eq!(retrieved, Ok(HELLO.len()));
    assert_eq!(buffer, HELLO);

    let ring = RingMapped::wrap(&REGION, &ropt).unwrap();
    let mut seq = SeqInner::wrap(ring, &sopt).unwrap();
    assert_eq!(seq.restore(), Ok(HELLO.len() as u32));

    let mut buffer = [0; HELLO.len()];
    assert_eq!(seq.get(&mut buffer), Ok(HELLO.len()));
    assert_eq!(buffer, HELLO);
}
