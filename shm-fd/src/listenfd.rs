//! Parse the LISTENFD environment variables.
//!
//! Create an associated optional target file descriptor number for the one we will be
//! initializing.
use crate::RawFd;
use alloc::{string::String, vec::Vec};

pub struct ListenFd {
    pub fd_base: RawFd,
    pub fd_len: RawFd,
    pub names: Vec<String>,
}

#[derive(Debug)]
pub enum Error {
    BadPid,
    BadFd,
    BadNames,
}

// https://github.com/systemd/systemd/blob/414ae39821f0c103b076fc5f7432f827e0e79765/src/libsystemd/sd-daemon/sd-daemon.c#L92-L129
impl ListenFd {
    #[cfg(all(feature = "std", feature = "libc"))]
    pub fn new() -> Option<Result<Self, Error>> {
        eprintln!(
            "{:?} {:?} {:?}",
            std::env::var_os("LISTEN_FDS"),
            std::env::var_os("LISTEN_PID"),
            std::env::var_os("LISTEN_FDNAMES"),
        );

        let Some(count) = std::env::var_os("LISTEN_FDS") else {
            return None;
        };

        if let Some(pid) = std::env::var_os("LISTEN_PID") {
            let Some(pid) = pid.to_str() else {
                return Some(Err(Error::BadPid));
            };

            let Ok(pid): Result<libc::pid_t, _> = pid.parse() else {
                return Some(Err(Error::BadPid));
            };

            if pid != unsafe { libc::getpid() } {
                return Some(Err(Error::BadPid));
            }
        } else {};

        let Ok(count): Result<RawFd, _> = ({
            count
                .to_str()
                .map_or_else(
                    || Err(core::num::IntErrorKind::InvalidDigit),
                    |st| st.parse::<RawFd>().map_err(|e| e.kind().clone()),
                )
        }) else {
            return Some(Err(Error::BadFd));
        };

        let names;
        if let Some(passed_fd) = std::env::var_os("LISTEN_FDNAMES") {
            // Must be a subset of ASCII.
            let Some(passed_fd) = passed_fd.to_str() else {
                return Some(Err(Error::BadNames));
            };

            names = passed_fd.split(":").map(String::from).collect();
        } else {
            names = Vec::new();
        }

        let listen = ListenFd {
            fd_base: 3,
            fd_len: count,
            names,
        };

        Some(Ok(listen))
    }
}
