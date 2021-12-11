use std::process::Command;
use std::os::unix::{io::AsRawFd, process::CommandExt};
use memfile::MemFile;

fn main() {
    let file = MemFile::create_sealable("persistent")
        .expect("failed to initialized shm-file");
    // Just reserve a file descriptor...
    let placeholder = MemFile::create_default("placeholder")
        .expect("failed to initialized shm-file");
    let rawfd = file.as_raw_fd();
    let target = placeholder.as_raw_fd();

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
