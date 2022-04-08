# shm-fd

This crate is part of [a group][shmfd] that enables using shared-memory files
conveniently for persisting state across program restarts. Programs can access
this file via language and runtime independent means from the environment. This
crate contains a simple binary to configure such an environment, and a library
to consume it.

[shmfd]: https://github.com/HeroicKatora/shmfd

## Usage

```rust
use shm_fd::SharedFd;

// Trust the environment..
let fd = unsafe { SharedFd::from_env() }?;
let memfile = fd.into_file()?;

// Example: utilize the shared memory via memmap2 crate
use memmap2::MmapMut;
let mapping = unsafe { MmapMut::map_mut(file.as_raw_fd()) }?;
let memory = &mut mapping[..];
```
