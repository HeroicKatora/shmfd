use std::collections::HashSet;
use std::ffi::{OsString, OsStr};
use std::{fs::OpenOptions, process, path::Path};
use std::os::unix::{
    fs::OpenOptionsExt,
    io::AsRawFd,
    io::RawFd,
    io::IntoRawFd,
};

use clap::{Parser, ValueEnum};
use memfile::MemFile;
use memmap2::MmapRaw;
use shm_fd::{ListenFd, ListenInit, SharedFd};

fn main() {
    let RestoreCommand {
        snapshot,
        file,
        command,
        args,
    } = RestoreCommand::parse();

    // FIXME: allow customization.
    let fd_name = "SHM_SHARED_FD";

    let listen = ListenFd::new()
        .transpose()
        .expect("failed to initialize LISTEN_FDS env");

    let init = ListenInit::<MemFile>::named_or_try_create::<std::io::Error>(
        listen,
        fd_name,
        || MemFile::create_sealable("persistent"),
    ).expect("failed to initialized shm-file");

    let shmfd = unsafe {
        SharedFd::from_listen(&init.listen).expect("failed to map shmfd")
    };

    let duped_shmfd = {
        match unsafe { libc::dup(shmfd.as_raw_fd()) } {
            -1 => Err(std::io::Error::last_os_error()).expect("failed to dup"),
            safe => safe,
        }
    };

    // Open the output file now, ensure we have it as a file descriptor before proceeding.
    let backup_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .custom_flags(libc::O_DSYNC)
        .open(&file)
        .expect("Failed to open backup file");

    let mut proc = process::Command::new(command);
    proc.args(&args);

    unsafe { init._set_pid(&mut proc) };

    unsafe { fcntl_cloexec(duped_shmfd.as_raw_fd()).expect("failed to set close-on-exec") };
    unsafe { fcntl_cloexec(backup_file.as_raw_fd()).expect("failed to set close-on-exec") };

    // Ignore SIGTERM and SIGCHLD as we always wait for our child to exit first.
    unsafe { posixly_ignore_signals() };

    // FIXME: if we unwind right away, it's bad. We will overwrite the backing file with this
    // currently raw, potentially bad, state causing data loss. Fu..
    let protector = unsafe {
        writeback_protector(WriteBack {
            shm: duped_shmfd,
            bck: backup_file.as_raw_fd(),
        })
    }.expect("Can protect with write back");

    // Before we start, let's prepare whatever backup already exists.
    //
    // FIXME: Only, if we had something to restore.
    //     if init.file.is_some()
    // But that isn't correct if the environment setup the memory map for us without initializing
    // it from any persistent source. We might instead want to introduce modify-time values to the
    // header to decide, or base it off the latest live offset?
    {
        (protector.how)(protector.write_back.bck, protector.write_back.shm);
    }

    match snapshot {
        None => {
            let protector: Dropped = protector;
            if let Some(code) = proc.status().expect("can receive status").code() {
                drop(protector);
                std::process::exit(code);
            }
        }
        Some(SnapshotMode::RestoreV1) => {
            let path = file_with_parent(&file).expect("backup file to have a containing directory");

            let mut protector = protector;
            let mut child = proc.spawn().expect("can receive status");

            let status = loop {
                if let Some(code) = child.try_wait().expect("can receive status") {
                    break code;
                };

                {
                    if let Err(err) = try_restore_v1(&mut protector, path) {
                        eprintln!("Error making backup: {err}");
                    }
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
            let _ = libc::lseek(dest, 0, libc::SEEK_SET);
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

    fn copy_file_all(source: RawFd, dest: RawFd) -> libc::ssize_t {
        unsafe {
            let length = libc::lseek(source, 0, libc::SEEK_END);
            let _ = libc::lseek(dest, 0, libc::SEEK_SET);
            libc::ftruncate(dest, length);
        }

        let Ok(file) = MmapRaw::map_raw(&source) else {
            return -1;
        };

        let start_ptr = file.as_ptr() as *const libc::c_void;
        let start_len = file.len();

        let mut remaining = start_len;
        while remaining > 0 {
            let written = unsafe {
                libc::write(dest, start_ptr, start_len)
            };

            if written < 0 {
                return -1;
            }

            remaining = remaining.saturating_sub(written as usize);
        }

        start_len as libc::ssize_t
    }

    /* First copy existing data to the shared memory.
     * We choose this to discover what is supported.
     */
    let how: fn(RawFd, RawFd) = match copy_file_range(bck, shm) {
        // This can be hit, if the file systems target does not support copy_file_range from a
        // memory-mapped file. Which is realistically pretty much all of them?
        diff if matches!(diff as libc::c_int, -1)
            && matches!(
                unsafe { *libc::__errno_location() },
                libc::EXDEV | libc::EFBIG
            ) =>
        {
            |source, dest| {
                copy_file_all(source, dest);
            }
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

#[derive(Clone, Copy)]
struct FileWithParent<'lt>(&'lt Path, &'lt Path);

fn file_with_parent(file: &OsStr) -> Option<FileWithParent<'_>> {
    let path = Path::new(file);
    let parent = path.parent()?;
    Some(FileWithParent(path, parent))
}

fn try_restore_v1(dropped: &mut Dropped, backup: FileWithParent) -> Result<(), std::io::Error> {
    let FileWithParent(backup_path, parent) = backup;
    let snapshot = shm_snapshot::File::new(dropped.write_back.shm)?;

    let mut pre_valid = HashSet::new();
    let mut pre_cfg = shm_snapshot::ConfigureFile::default();
    if let Some(recovery) = snapshot.recover(&mut pre_cfg) {
        recovery.valid(&mut pre_valid);
    }

    // Detect which portions stayed immutable by collecting the assertions twice. Once before we
    // write the file, and once afterwards. The entries which were active before certify that their
    // data was written before the range copy, the entries which were active afterwards certify
    // that their data range was not modified before the end of the range copy.

    // Write everything into a temporary file first.
    let pending = tempfile::NamedTempFile::new_in(parent)?;
    (dropped.how)(dropped.write_back.shm, pending.as_raw_fd());

    // And now we must mask from the backup file all entries that we can not prove are valid. If
    // there are any remaining entries, this backup was successful.
    //
    // We then check if the backup file contains any successful data transaction.
    let mut post_valid = HashSet::new();
    let post_snapshot = shm_snapshot::File::new(pending.as_raw_fd())?;
    if let Some(recovery) = post_snapshot.recover(&mut pre_cfg) {
        // First mark all change entries invalid.
        recovery.retain(&pre_valid);

        // Then collect all remaining live entries.
        recovery.valid(&mut post_valid);
    }

    if post_valid.is_empty() {
        // No progress was made, no entry successfully persisted.
        return Ok(());
    }

    // FIXME: this is not yet implemented, i.e. we have wrong backup files with entries that have
    // not correctly sandwiched the immutable time interval of their data.

    // Success! We now swap out our file handles.
    let pending = pending.persist(backup_path)?;
    let mut pending_fd = pending.into_raw_fd();
    core::mem::swap(&mut dropped.write_back.bck, &mut pending_fd);
    unsafe { libc::close(pending_fd) };

    Ok(())
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
