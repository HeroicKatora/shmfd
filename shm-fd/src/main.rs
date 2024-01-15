mod sd_fd;

use shm_fd::ListenFd;
use memfile::MemFile;
use std::os::unix::{io::AsRawFd, process::CommandExt};
use std::process::Command;

fn main() {
    let notify_sd = sd_fd::NotifyFd::new()
        .expect("failed to open notify socket");

    // FIXME: allow customization.
    let fd_name = "SHM_SHARED_FD";
    let target;
    let file;

    let listen: ListenFd = match ListenFd::new() {
        None => {
            eprintln!("Initializing new file and LISTEN_FD state");
            // Create our own listenfd state.
            file = Some(MemFile::create_sealable("persistent").expect("failed to initialized shm-file"));

            target = 3;
            ListenFd {
                fd_base: 3,
                fd_len: 1,
                names: vec![fd_name.to_owned()],
            }
        },
        Some(listen) => {
            let mut listen = listen.expect("failed to parse LISTEN_FD state");
            // Re-use the listenfd state passed to us.
            let position = listen.names
                .iter()
                .position(|n| n == fd_name);

            if let Some(position) = position {
                eprintln!("Using LISTEN_FD state passed by environment");
                target = listen.fd_base + position as std::os::fd::RawFd;
                // FIXME: verify that this is a memfile?
                file = None;
            } else {
                eprintln!("Adding file to LISTEN_FD state passed by environment");

                file = Some(MemFile::create_sealable("persistent").expect("failed to initialized shm-file"));
                listen.names.push(fd_name.into());
                target = listen.fd_base + listen.fd_len;
                listen.fd_len += 1;
            }

            listen
        },
    };

    // Just reserve a file descriptor...
    let rawfd = file.as_ref().map(|v| v.as_raw_fd());

    if let Some(rawfd) = rawfd {
        eprintln!("Created new file at fd {}", rawfd);
        if let Some(notify) = notify_sd {
            // If we created a new file descriptor, pass it to systemd.
            eprintln!("Passing new file {rawfd}:{fd_name} to environment");
            let state = format!("FDSTORE=1\nFDNAME={fd_name}");
            notify.notify_with_fds(&state, core::slice::from_ref(&rawfd))
                .expect("failed to setup socket store");
        }
    }

    let mut args = std::env::args_os().skip(1);
    let cmd = args.next().expect("no given");
    let args: Vec<_> = args.collect();

    let mut proc = Command::new(&cmd);
    proc.args(&args);

    proc.env("LISTEN_FDS", listen.fd_len.to_string());
    proc.env("LISTEN_FDNAMES", listen.names.join(":"));

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

    let error = proc.exec();
    panic!("Failed to exec: {error}")
}
