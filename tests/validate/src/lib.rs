use std::os::fd::{AsRawFd, IntoRawFd, RawFd};
use std::os::unix::process::CommandExt;

use assert_cmd::{assert::Assert, Command};
use memfile::MemFile;

pub struct Env {
    file: MemFile,
    placeholder: RawFd,
}

impl Env {
    pub fn new() -> Self {
        let file = MemFile::create_sealable("persistent").expect("failed to initialized shm-file");
        let placeholder =
            MemFile::create_default("placeholder").expect("failed to initialized shm-file");
        Env {
            file,
            placeholder: placeholder.into_raw_fd(),
        }
    }

    /// Run a process under a shared FD, referring to the memfile controlled by this struct.
    ///
    /// Note: for safety reasons we must at least spawn the process before returning.
    pub fn shared_fd(&self, mut cmd: std::process::Command) -> Assert {
        cmd.env("SHM_SHARED_FDS", format!("{}", self.placeholder));

        // We borrow from `self` but the process is started before we return, executing the
        // pre_exec hook as well.
        unsafe {
            let raw_fd = self.file.as_raw_fd();
            let placeholder = self.placeholder;

            cmd.pre_exec(move || {
                if -1 == libc::dup2(raw_fd, placeholder) {
                    panic!("Failed to dup file descriptor pre-exec");
                } else {
                    Ok(())
                }
            });
        }

        let mut cmd = Command::from_std(cmd);
        cmd.assert()
    }
}
