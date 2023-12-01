#![cfg(target_family = "unix")]
use shm_snapshot::{ConfigureFile, File};
use memfile::CreateOptions;

#[test]
fn after_no_writes() {
    let file = CreateOptions::new().create(env!("CARGO_PKG_NAME"))
        .expect("to create a memory file");
    file.set_len(0x1_0000_0000).unwrap();
    let _restore_from = file.try_clone().unwrap();

    let mut file = File::new(file).unwrap();
    let mut cfg = ConfigureFile::default();

    file.discover(&mut cfg);
    cfg.or_insert_with(|cfg| {
        cfg.entries = 0x80;
        cfg.data = 0x100;
    });

    let mut writer = file.configure(&cfg);
    const GREETING: &[u8] = b"Hello, world";
    writer.write(GREETING).unwrap();

    drop(writer);

    let file = _restore_from;
    let mut file = File::new(file).unwrap();
    let mut cfg = ConfigureFile::default();
    file.discover(&mut cfg);
    cfg.or_insert_with(|cfg| {
        panic!("Failed to restore configuration {cfg:?}");
    });

    let mut valid_priors = vec![];
    file.valid(&mut valid_priors);
    assert_eq!(valid_priors.len(), 1, "{:?}", &valid_priors);

    let _writer = file.configure(&cfg);
}
