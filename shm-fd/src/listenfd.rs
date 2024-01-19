//! Parse the LISTENFD environment variables.
//!
//! Create an associated optional target file descriptor number for the one we will be
//! initializing.
use crate::RawFd;
use alloc::{string::String, vec::Vec};
use alloc::borrow::ToOwned;

#[cfg(feature = "std")]
use std::os::unix::process::CommandExt;
#[cfg(feature = "std")]
use crate::NotifyFd;

/// Captures information on file descriptors passed through the environment.
///
/// When systemd sets pre-opened file descriptors in a service unit, it passes a description of
/// them via the environment variables.
pub struct ListenFd {
    /// The first file descriptor referred to by the environment.
    pub fd_base: RawFd,
    /// The count of file descriptors referred to by the environment.
    pub fd_len: RawFd,
    /// The names / descriptors of all file descriptors, if available.
    pub names: Vec<String>,
}

/// A `ListenFd` enriched with a relevant file descriptor for passing to a child process.
///
/// This either captures one of the file descriptors passed, or initialized a new owning file
/// descriptor, generally a file. It also computes the *target* file descriptor number. That is, if
/// the file was captured by finding its name in the `LISTN_FDNAMES` array then it is computed and
/// in-bounds of the passed array. If the file was not captured, it is _added_ to the `listen`
/// information and its new hypothetical descriptor is stored in `target`.
pub struct ListenInit<F> {
    /// The originally, potentially modified, passed `ListenFd`.
    pub listen: ListenFd,
    /// Owns the file if it had to be constructed due to not being found.
    pub file: Option<F>,
    /// The file descriptor the file would have in childs (or the next restart if registered).
    ///
    /// See struct description.
    pub target: RawFd,
    _inner: (),
}

#[derive(Debug)]
pub enum Error {
    BadPid,
    BadFd,
    BadNames,
}

// https://github.com/systemd/systemd/blob/414ae39821f0c103b076fc5f7432f827e0e79765/src/libsystemd/sd-daemon/sd-daemon.c#L92-L129
impl ListenFd {
    /// Capture and translate the systemd standard environment variables.
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
    /// Derive a new ListenFd setup, finds or adds a file descriptor.
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
                    _inner: (),
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
                    _inner: (),
                })
            }
        }
    }

    /// Notify systemd, if the file descriptor was not present.
    #[cfg(feature = "std")]
    pub fn maybe_notify(&self, notify: NotifyFd, fd_name: &str)
        -> Result<(), std::io::Error>
        where F: std::os::fd::AsRawFd
    {
        if let Some(newfile) = &self.file.as_ref() {
            let rawfd = newfile.as_raw_fd();
            let state = format!("FDSTORE=1\nFDNAME={fd_name}");
            notify.notify_with_fds(&state, core::slice::from_ref(&rawfd))
        } else {
            Ok(())
        }
    }

    /// Modify a command such that it copies the file descriptors at the appropriate location.
    ///
    /// # Safety
    ///
    /// This function is unsafe, since the caller must prove that copying the file descriptors is
    /// okay.
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

    // FIXME: it's not clear if we want to do this, fake the LISTEN_PID that is. Sure, the systemd
    // library uses it to check whether a file descriptor array passed as LISTEN_FDS is meant for
    // it but *also* for the notify socket. We can consciously pass-on the file descriptors but not
    // the socket, which would not work as expected for it and reject messages as unauthorized by
    // the assumed PID.
    #[doc(hidden)]
    #[cfg(feature = "std")]
    pub unsafe fn _set_pid(&self, proc: &mut std::process::Command) {
        proc.env_remove("LISTEN_PID");

        if std::env::var_os("LISTEN_PID").is_some() {
            unsafe {
                proc.pre_exec(|| {
                    let pid = format!("{}!!\0", libc::getpid());
                    static LISTEN_PID: &[u8] = b"LISTEN_PID\0";
                    libc::unsetenv(LISTEN_PID.as_ptr() as *const _);

                    if -1 == libc::setenv(
                        LISTEN_PID.as_ptr() as *const _,
                        pid.as_ptr() as *const _,
                        1 /* overwrite */,
                    ) {
                        return Err(std::io::Error::last_os_error());
                    }

                    Ok(())
                });
            }
        }
    }
}
