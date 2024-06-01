use crate::writer::{DataPage, HeadCache, HeadPage, SequencePage, WriteHead};
use core::sync::atomic::Ordering;

#[test]
fn initialize_inner_basic() {
    let mut valids = vec![];
    with_setup(|mut head| {
        head.iter_valid(&mut valids, Ordering::Relaxed);
        assert!(valids.is_empty());

        head.pre_configure_pages(0x80);
        head.pre_configure_entries(0x10);
        head.configure_pages();

        let mut entry = head.entry();
        const DATA: &[u8] = b"Hello, world!";
        let end_ptr = entry
            .new_write_offset(DATA.len())
            .expect("Invalid, can't determine end offset of data");
        entry.invalidate_heads(end_ptr);
        entry.copy_from_slice(DATA);
        entry.commit();

        head.iter_valid(&mut valids, Ordering::Relaxed);
        assert_eq!(valids.len(), 1);
    });
}

#[test]
fn commit_not() {
    let mut valids = vec![];
    with_setup(|mut head| {
        head.iter_valid(&mut valids, Ordering::Relaxed);
        assert!(valids.is_empty());

        head.pre_configure_pages(0x80);
        head.pre_configure_entries(0x10);
        head.configure_pages();

        let mut entry = head.entry();
        entry.copy_from_slice(b"Hello, world!");
        drop(entry);

        head.iter_valid(&mut valids, Ordering::Relaxed);
        assert_eq!(valids.len(), 0);

        let mut entry = head.entry();
        const DATA: &[u8] = b"Hello, world!";
        let end_ptr = entry
            .new_write_offset(DATA.len())
            .expect("Invalid, can't determine end offset of data");
        entry.invalidate_heads(end_ptr);
        entry.copy_from_slice(DATA);
        entry.commit();

        head.iter_valid(&mut valids, Ordering::Relaxed);
        assert_eq!(valids.len(), 1);
    })
}

#[derive(Default)]
struct TestSetup {
    head: HeadPage,
    sequence: [SequencePage; 2],
    data: [DataPage; 4],
}

fn with_setup(method: impl FnOnce(WriteHead)) {
    let test = Box::leak(Box::new(TestSetup::default()));

    method(WriteHead {
        cache: HeadCache::new(),
        meta: &mut test.head,
        sequence: &mut test.sequence,
        data: &mut test.data,
        tail: &[],
    })
}
