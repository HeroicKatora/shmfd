//! Parse the LISTENFD environment variables.
//!
//! Create an associated optional target file descriptor number for the one we will be
//! initializing.
use std::os::fd::RawFd;

pub struct ListenFd {
    target: RawFd,
}

impl ListenFd {
    pub fn new() -> Result<Self, std::io::Error> {
        let passed_fd = std::env::var_os("LISTEN_FDNAMES");

        todo!()
    }
}
