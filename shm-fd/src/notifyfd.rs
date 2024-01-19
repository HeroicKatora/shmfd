//! Interact with the Systemd notify socket.
use std::env;
use std::ffi::{OsString, OsStr};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixDatagram;

pub struct NotifyFd {
    fd: OwnedFd,
    addr: Vec<libc::c_char>,
}

// https://github.com/systemd/systemd/blob/414ae39821f0c103b076fc5f7432f827e0e79765/src/libsystemd/sd-daemon/sd-daemon.c#L454-L598
impl NotifyFd {
    pub fn new() -> Result<Option<Self>, std::io::Error> {
        let Some(addr) = env::var_os("NOTIFY_SOCKET") else {
            return Ok(None);
        };

        Self::from_env(addr).map(Some)
    }

    pub fn from_env(name: OsString) -> Result<Self, std::io::Error> {
        let ty = name.as_encoded_bytes().get(0).cloned();

        let name_bytes = match ty {
            Some(b'/') => {
                name.as_encoded_bytes()
            }
            Some(b'@') => {
                &name.as_encoded_bytes()[1..]
            },
            _ => return Err(std::io::ErrorKind::Unsupported)?,
        };


        let name = OsStr::from_bytes(name_bytes);
        let dgram_socket = UnixDatagram::unbound()?;
        dgram_socket.connect(name)?;

        Ok(NotifyFd {
            fd: dgram_socket.into(),
            addr: name_bytes.iter().map(|&b| b as libc::c_char).collect(),
        })
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
        let mut hdr: libc::msghdr = unsafe { core::mem::zeroed::<libc::msghdr>() };
        let mut iov: libc::iovec = unsafe { core::mem::zeroed::<libc::iovec>() };
        let mut addr: libc::sockaddr_un = unsafe { core::mem::zeroed::<libc::sockaddr_un>() };

        iov.iov_base = state.as_ptr() as *mut libc::c_void;
        iov.iov_len = state.len();

        addr.sun_family = libc::AF_UNIX as libc::c_ushort;
        let addr_len = addr.sun_path.len().min(self.addr.len());
        addr.sun_path[..addr_len].copy_from_slice(&self.addr[..addr_len]);

        hdr.msg_iov = &mut iov;
        hdr.msg_iovlen = 1;
        hdr.msg_namelen = core::mem::size_of_val(&addr) as libc::c_uint;
        hdr.msg_name = &mut addr as *mut _ as *mut libc::c_void;

        // No send_ucred yet, hence
        let len = u32::try_from(core::mem::size_of_val(fds))
            .expect("user error");
        let len = if len > 0 {
            (unsafe { libc::CMSG_SPACE(len) } as usize)
        } else { 0 };

        let mut buf = vec![0; len];

        hdr.msg_controllen = len;
        hdr.msg_control = buf.as_mut_ptr() as *mut libc::c_void;

        if len > 0 {
            let cmsg = unsafe { libc::CMSG_FIRSTHDR(&hdr) };
            let cmsg = unsafe { &mut *cmsg };
            let msg_len = core::mem::size_of_val(fds);

            cmsg.cmsg_level = libc::SOL_SOCKET;
            cmsg.cmsg_type = libc::SCM_RIGHTS;
            cmsg.cmsg_len = unsafe { libc::CMSG_LEN(msg_len as u32) } as usize;

            assert!(cmsg.cmsg_len >= msg_len);
            let data = unsafe { libc::CMSG_DATA(cmsg) };

            // Safety: Pointer `data` is part of the buffer, by libc::CMSG_DATA.
            // Then fds is a pointer to an integer slice, always initialized.
            // Then the message length is the number of bytes in the slice.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    fds.as_ptr() as *const _ as *const u8,
                    data,
                    msg_len,
                );
            }
        }

        let sent = unsafe {
            libc::sendmsg(self.fd.as_raw_fd(), &hdr, libc::MSG_NOSIGNAL)
        };

        if -1 == sent {
            return Err(std::io::Error::last_os_error());
        }

        if sent as usize != state.len() {
            return Err(std::io::ErrorKind::InvalidData)?;
        }

        Ok(())
    }
}
