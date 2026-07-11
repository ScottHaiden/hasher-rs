use libc;

use std::{error, io, thread};

fn do_close(fd: libc::c_int) -> io::Result<()> {
    let result = unsafe { libc::close(fd) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub trait Reader {
    fn read_message(&self) -> Result<Option<String>, Box<dyn error::Error>>;
}

pub struct SocketReader {
    fd: libc::c_int,
}

impl SocketReader {
    pub fn new() -> io::Result<Self> {
        let mut fds: [libc::c_int; 2] = [0; 2];

        let result = unsafe {
            libc::socketpair(
                libc::AF_UNIX,
                libc::SOCK_SEQPACKET,
                0,
                fds.as_mut_ptr())
        };

        if result < 0 { return Err(io::Error::last_os_error()) }

        let (rfd, wfd) = (fds[0], fds[1]);

        thread::spawn(move || {
            if let Err(e) = do_close(wfd) {
                panic!("close({}) failed ({})", wfd, e);
            }
        });

        Ok(Self { fd: rfd })
    }
}

impl Reader for SocketReader {
    fn read_message(&self) -> Result<Option<String>, Box<dyn error::Error>> {
        let mut buf = vec![0u8; 4096];
        let result = unsafe {
            libc::read(
                self.fd,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len())
        };

        if result < 0 { return Err(Box::new(io::Error::last_os_error())); }
        if result == 0 { return Ok(None); }

        buf.resize(result.try_into().unwrap(), 0u8);

        let parsed = String::from_utf8(buf)?;
        Ok(Some(parsed))
    }
}

impl Drop for SocketReader {
    fn drop(&mut self) {
        if let Err(e) = do_close(self.fd) {
            panic!("close({}) failed ({})", self.fd, e);
        }
    }
}
