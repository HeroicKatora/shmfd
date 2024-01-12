mod listenfd;
mod sd_fd;

use memfile::MemFile;
use std::os::unix::{io::AsRawFd, process::CommandExt};
use std::process::Command;

fn main() {
    let notify_sd = sd_fd::NotifyFd::from_env("NOTIFY_SOCKET")
        .expect("failed to open notify socket");

    let file = MemFile::create_sealable("persistent").expect("failed to initialized shm-file");
    // Just reserve a file descriptor...
    let placeholder =
        MemFile::create_default("placeholder").expect("failed to initialized shm-file");
    let rawfd = file.as_raw_fd();
    let target = placeholder.as_raw_fd();

    if let Some(notify) = notify_sd {
        let state = format!("FDSTORE=1\nFDNAME=shmfd");
        notify.notify_with_fds(&state, core::slice::from_ref(&rawfd))
            .expect("failed to setup socket store");
    }

    let mut args = std::env::args_os().skip(1);
    let cmd = args.next().expect("no given");
    let args: Vec<_> = args.collect();

    let mut proc = Command::new(&cmd);
    proc.args(&args);
    proc.env("SHM_SHARED_FDS", format!("{}", target));

    unsafe {
        proc.pre_exec(move || {
            if -1 == libc::dup2(rawfd, target) {
                panic!("Failed to dup file descriptor pre-exec");
            } else {
                Ok(())
            }
        });
    }

    let _ = proc.exec();
}
