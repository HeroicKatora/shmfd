//! Interact with the Systemd notify socket.
use std::env;
use std::os::fd::RawFd;

pub struct NotifyFd {
    fd: RawFd,
}

impl NotifyFd {
    pub fn from_env(name: &str) -> Result<Option<Self>, std::io::Error> {
        let Some(addr) = env::var_os(name) else {
            return Ok(None);
        };

        todo!()
    }

    // Consume the notify fd to send a FD notification.
    //
    // FIXME: That's what the c function is doing.
    // <https://github.com/systemd/systemd/blob/414ae39821f0c103b076fc5f7432f827e0e79765/src/libsystemd/sd-daemon/sd-daemon.c#L454C12-L454C40>
    //
    // It's utterly confusing why we'd open a full file descriptor for every single message but oh
    // well, here we are. The code sends the ucredentials and file descriptors as part of the
    // *control* data, not the message data, of course, that's how you pass file descriptors, but
    // it only sends control data once (even for streams). Thus we will only attempt at most one
    // message with file descriptors and thus this method must consume the NotifyFd.
    pub fn notify_with_fds(
        self,
        state: &str,
        fds: &[RawFd]
    ) -> Result<(), std::io::Error> {
        todo!()
    }
}

impl Drop for NotifyFd {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) }
    }
}
