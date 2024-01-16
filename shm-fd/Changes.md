## Version 0.4

Integrated with systemd File Descriptor store, the environment variables are
now accordingly `$LISTEN_FDS`, `$LISTEN_FDNAMES`. The binary forwards the
systemd configuration or initializes it.

## Version 0.3

Merged the binary into the crate, i.e. `shm-fd` now serves as a wrapper binary
to setup a file descriptor into another which can consume it.
