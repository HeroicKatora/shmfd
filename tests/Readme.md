Integration testing for the shm crates.

```bash
cargo test --release
```

The main concern are coverage of race conditions, i.e. violations of our
consistency requirements. Some easy to verify data is created with hashing.

## Structure

The structure of implementation is a little strange at first. The dependencies
on other crates are effectively binary or artifact dependencies, which is both
unstable and we want to allow the patch / override with other binaries to
enable out-of-tree testing as well as compliance testing of alternate
implementations.

Hence, this is a workspace building *mainly* the tooling for tests. One of the
packages here will then build and import the binaries from the main crates in
its `build.rs` file.
