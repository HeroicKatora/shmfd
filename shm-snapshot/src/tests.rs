use crate::writer::{HeadCache, HeadPage, SequencePage, DataPage, WriteHead};
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
        entry.copy_from_slice(b"Hello, world!");
        entry.commit();

        head.iter_valid(&mut valids, Ordering::Relaxed);
        assert_eq!(valids.len(), 1);
    });
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
    })
}
