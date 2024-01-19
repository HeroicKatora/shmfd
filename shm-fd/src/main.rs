use shm_fd::{ListenFd, ListenInit, NotifyFd};
use memfile::MemFile;

use std::os::fd::AsRawFd;
use std::process::Command;

fn main() {
    // FIXME: allow customization.
    let fd_name = "SHM_SHARED_FD";

    let mut args = std::env::args_os().skip(1);
    let cmd = args.next().expect("no given");
    let args: Vec<_> = args.collect();

    let listen = ListenFd::new()
        .transpose()
        .expect("failed to parse LISTEN_FDS information");

    let notify_sd = NotifyFd::new()
        .transpose()
        .expect("failed to open notify socket");

    let init = ListenInit::<MemFile>::named_or_try_create(
        listen,
        fd_name,
        || MemFile::create_sealable("persistent"),
    ).expect("failed to initialized shm-file");

    // Just reserve a file descriptor...
    let rawfd = init.file.as_ref().map(|v| v.as_raw_fd());

    if let Some(notify) = notify_sd {
        if let Some(rawfd) = rawfd {
            eprintln!("Passing new file {rawfd}:{fd_name} to environment");
        }

        // If we created a new file descriptor, pass it to systemd.
        init.maybe_notify(notify, fd_name)
            .expect("failed to setup socket store");
    }

    if let Some(rawfd) = rawfd {
        eprintln!("Created new file at fd {}", rawfd);
    }

    let mut proc = Command::new(&cmd);
    proc.args(&args);
    // Safety: we promise the file descriptor is safe to clone and not-close-on-exec in the child.
    unsafe { init.wrap_proc(&mut proc) }

    let error = std::os::unix::process::CommandExt::exec(&mut proc);
    panic!("Failed to exec: {error}")
}
