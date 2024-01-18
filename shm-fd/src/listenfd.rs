//! Parse the LISTENFD environment variables.
//!
//! Create an associated optional target file descriptor number for the one we will be
//! initializing.
use crate::RawFd;
use alloc::{string::String, vec::Vec};
use alloc::borrow::ToOwned;

#[cfg(feature = "std")]
use std::os::unix::process::CommandExt;

pub struct ListenFd {
    pub fd_base: RawFd,
    pub fd_len: RawFd,
    pub names: Vec<String>,
}

pub struct ListenInit<F> {
    pub listen: ListenFd,
    pub file: Option<F>,
    pub target: RawFd,
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

impl<F> ListenInit<F> {
    pub fn named_or_try_create<R>(
        this: Option<ListenFd>,
        fd_name: &str,
        with: impl FnOnce() -> Result<F, R>,
    ) -> Result<Self, R> {
        match this {
            None => {
                let file = with()?;
                let target = 3;

                let listen = ListenFd {
                    fd_base: 3,
                    fd_len: 1,
                    names: Vec::from([fd_name.to_owned()]),
                };

                Ok(ListenInit {
                    listen,
                    file: Some(file),
                    target,
                })
            }
            Some(listen) => {
                let mut listen = listen;
                // Re-use the listenfd state passed to us.
                let position = listen.names
                    .iter()
                    .position(|n| n == fd_name);

                let (target, file);
                if let Some(position) = position {
                    target = listen.fd_base + position as RawFd;
                    // FIXME: verify that this is a memfile?
                    file = None;
                } else {
                    let _file = with()?;
                    file = Some(_file);

                    listen.names.push(fd_name.into());
                    target = listen.fd_base + listen.fd_len;
                    listen.fd_len += 1;
                }

                Ok(ListenInit {
                    listen,
                    file,
                    target,
                })
            }
        }
    }

    #[cfg(feature = "std")]
    pub unsafe fn wrap_proc(&self, proc: &mut std::process::Command)
        where F: std::os::fd::AsRawFd,
    {
        let rawfd = self.file.as_ref().map(|v| v.as_raw_fd());
        proc.env("LISTEN_FDS", self.listen.fd_len.to_string());
        proc.env("LISTEN_FDNAMES", self.listen.names.join(":"));
        let target = self.target;

        unsafe {
            proc.pre_exec(move || {
                let pid = format!("{}\0", libc::getpid());
                static LISTEN_PID: &[u8] = b"LISTEN_PID\0";
                if -1 == libc::setenv(LISTEN_PID.as_ptr() as *const _, pid.as_ptr() as *const _, 1) {
                    return Err(std::io::Error::last_os_error());
                }

                if let Some(rawfd) = rawfd {
                    if rawfd == target {
                        // We adjust the flags to not close-on-exec.
                        if -1 == libc::fcntl(rawfd, libc::F_SETFD, 0) {
                            return Err(std::io::Error::last_os_error());
                        }
                    } else {
                        if -1 == libc::dup2(rawfd, target) {
                            return Err(std::io::Error::last_os_error());
                        }
                    }
                }

                Ok(())
            });
        }
    }
}
