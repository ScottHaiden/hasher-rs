use std::{
    error,
    fs,
    io,
    path,
    thread,
};

use libc;

struct FdHolder {
    fd: libc::c_int,
}

impl FdHolder {
    fn new(fd: libc::c_int) -> Self {
        Self { fd: fd }
    }

}

impl std::ops::Deref for FdHolder {
    type Target = libc::c_int;

    fn deref(&self) -> &libc::c_int { &self.fd }
}

impl Drop for FdHolder {
    fn drop(&mut self) {
        let result = unsafe { libc::close(self.fd) };
        if result < 0 {
            let err = io::Error::last_os_error();
            panic!("close({}) failed ({})", self.fd, err);
        }
    }
}

fn visit_dirs(
        dir: &path::Path,
        follow_symlinks: bool,
        cb: &dyn Fn(&fs::DirEntry)) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path: path::PathBuf = entry.path();
        if !follow_symlinks && path.is_symlink() { continue; }
        if path.is_dir() {
            visit_dirs(&path, follow_symlinks, cb)?;
        } else if path.is_file() {
            cb(&entry);
        }
    }
    Ok(())
}

fn write_message(fd: libc::c_int, message: &[u8]) -> io::Result<()> {
    let amount = unsafe { libc::write(
        fd,
        message.as_ptr() as *const libc::c_void,
        message.len())
    };
    if amount < 0 {
        Err(io::Error::last_os_error())
    } else if message.len() != usize::try_from(amount).ok().unwrap() {
        panic!("Unhandled incomplete write");
    } else {
        Ok(())
    }
}

pub trait Reader {
    fn read_message(&self) -> Result<Option<String>, Box<dyn error::Error>>;
}

pub struct SocketReader {
    fd: FdHolder,
}

impl SocketReader {
    pub fn new(follow_symlinks: bool, paths: Vec<path::PathBuf>) -> io::Result<Self> {
        let mut fds: [libc::c_int; 2] = [0; 2];

        let result = unsafe {
            libc::socketpair(
                libc::AF_UNIX,
                libc::SOCK_SEQPACKET,
                0,
                fds.as_mut_ptr())
        };

        if result < 0 { return Err(io::Error::last_os_error()) }

        let rfd = FdHolder::new(fds[0]);
        let wfd = FdHolder::new(fds[1]);

        thread::spawn(move || {
            let wfd: FdHolder = wfd;
            for path in paths {
                visit_dirs(&path, follow_symlinks, &|e| {
                    let message = e.path()
                        .to_str()
                        .expect("failed to convert to unicode")
                        .to_owned()
                        .into_bytes();
                    write_message(*wfd, &message)
                        .expect("Failed to write message into fd");
                }).expect("visit_dirs failed");
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
                *self.fd,
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
