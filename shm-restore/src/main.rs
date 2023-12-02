use std::os::unix::{fs::OpenOptionsExt, io::AsRawFd, io::RawFd};
use std::ffi::{OsString, OsStr};
use std::{fs::OpenOptions, process};

use shm_fd::SharedFd;
use clap::{Parser, ValueEnum};

fn main() {
    let RestoreCommand {
        snapshot,
        file,
        command,
        args,
    } = RestoreCommand::parse();

    let duped_shmfd = if let Some(fd) = unsafe { SharedFd::from_env() } {
        match unsafe { libc::dup(fd.as_raw_fd()) } {
            -1 => Err(std::io::Error::last_os_error()).expect("failed to dup"),
            safe => safe,
        }
    } else {
        std::process::exit(1);
    };

    // Open the file now, ensure we have it as a file descriptor before proceeding.
    let backup_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .custom_flags(libc::O_DSYNC)
        .open(&file)
        .expect("Failed to open backup file");

    let mut proc = process::Command::new(command);
    proc.args(&args);

    unsafe { fcntl_cloexec(duped_shmfd.as_raw_fd()).expect("failed to set close-on-exec") };
    unsafe { fcntl_cloexec(backup_file.as_raw_fd()).expect("failed to set close-on-exec") };

    // Ignore SIGTERM and SIGCHLD as we always wait for our child to exit first.
    unsafe { posixly_ignore_signals() };
    let protector = unsafe {
        writeback_protector(WriteBack {
            shm: duped_shmfd,
            bck: backup_file.as_raw_fd(),
        })
    };

    match snapshot {
        None => {
            if let Some(code) = proc.status().expect("can receive status").code() {
                drop(protector);
                std::process::exit(code);
            }
        }
        Some(SnapshotMode::RestoreV1) => {
            let mut protector = protector;
            let mut child = proc.spawn().expect("can receive status");

            let status = loop {
                if let Some(code) = child.try_wait().expect("can receive status") {
                    break code;
                };

                if let Ok(protector) = &mut protector {
                    try_restore_v1(protector, &file);
                }
            };

            drop(protector);
            if let Some(code) = status.code() {
                std::process::exit(code);
            }
        }
    }
}

#[derive(Parser)]
struct RestoreCommand {
    /// Configure making continuous atomic snapshots of the memory while running.
    ///
    /// The strategy defines the reliability and/or synchronization mode of the snapshot by a
    /// strategy. They may require different degrees of coordinate with the client program but are
    /// in general designed to be lock-free.
    #[arg(value_enum, long)]
    snapshot: Option<SnapshotMode>,

    #[arg(help = "The backup file")]
    file: OsString,

    #[arg(help = "The command to execute with the SHM-FD set as environment variable")]
    command: OsString,

    args: Vec<OsString>,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum SnapshotMode {
    /// Use a lock-free, optimistic snapshot functionality.
    ///
    /// The reference implementation is in `shm-snapshot`.
    RestoreV1,
}

struct WriteBack {
    shm: RawFd,
    bck: RawFd,
}

struct Dropped {
    write_back: WriteBack,
    how: fn(RawFd, RawFd),
}

unsafe fn writeback_protector(
    WriteBack { shm, bck }: WriteBack,
) -> Result<Dropped, std::io::Error> {
    fn copy_file_range(source: RawFd, dest: RawFd) -> libc::ssize_t {
        unsafe {
            let length = libc::lseek(source, 0, libc::SEEK_END);
            let mut off_source = 0;
            let mut off_dest = 0;

            // TODO: should we care about this failing?
            libc::ftruncate(dest, length);
            libc::copy_file_range(
                source,
                &mut off_source,
                dest,
                &mut off_dest,
                length as usize,
                0,
            )
        }
    }

    /* First copy existing data to the shared memory.
     * We choose this to discover what is supported.
     */
    let how: fn(RawFd, RawFd) = match copy_file_range(bck, shm) {
        diff if matches!(diff as libc::c_int, libc::EXDEV | libc::EFBIG) => {
            todo!("Fallback to normal copy")
        }
        diff if diff < 0 => return Err(std::io::Error::last_os_error()),
        _ => |source, dest| {
            copy_file_range(source, dest);
        },
    };

    /* On drop, copy all data back to the backup file.
     */
    impl Drop for Dropped {
        fn drop(&mut self) {
            (self.how)(self.write_back.shm, self.write_back.bck);
        }
    }

    Ok(Dropped {
        write_back: WriteBack { shm, bck },
        how,
    })
}

fn try_restore_v1(dropped: &mut Dropped, backup: &OsStr) {
}

// Ignore SIGTERM..
unsafe fn posixly_ignore_signals() {
    let mut action: libc::sigaction = core::mem::zeroed();

    type Sigaction = fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void);
    action.sa_sigaction = (|_, _, _| ()) as Sigaction as usize;

    libc::sigaction(libc::SIGTERM, &mut action as *mut _, core::ptr::null_mut());
    libc::sigaction(libc::SIGINT, &mut action as *mut _, core::ptr::null_mut());
    libc::sigaction(libc::SIGCHLD, &mut action as *mut _, core::ptr::null_mut());
}

unsafe fn fcntl_cloexec(fd: RawFd) -> Result<(), std::io::Error> {
    // To large parts from <man 3p fcntl> (2017)
    let mut flags = libc::fcntl(fd, libc::F_GETFD);
    if -1 == flags {
        return Err(std::io::Error::last_os_error());
    }
    flags |= libc::FD_CLOEXEC;
    if -1 == libc::fcntl(fd, libc::F_SETFD, flags) {
        return Err(std::io::Error::last_os_error());
    }

    Ok(())
}
