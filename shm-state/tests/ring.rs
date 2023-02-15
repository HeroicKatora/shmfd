use shm_state::Mapper;

#[test]
#[cfg(feature = "libc")]
fn setup() {
    let map = Mapper::new();
}
