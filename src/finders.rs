use std::{
    error::Error,
    ffi::{
        c_int,
        c_void,
    },
    fs::{
        self,
        DirEntry,
    },
    io,
    path::{
        Path,
        PathBuf,
    },
    sync::atomic::{
        AtomicUsize,
        Ordering,
    },
};

use libc;

fn visit_dirs(
        dir: &Path,
        follow_symlinks: bool,
        cb: &dyn Fn(&DirEntry)) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path: PathBuf = entry.path();
        if !follow_symlinks && path.is_symlink() { continue; }
        if path.is_dir() {
            visit_dirs(&path, follow_symlinks, cb)?;
        } else if path.is_file() {
            cb(&entry);
        }
    }
    Ok(())
}

fn write_message(fd: c_int, message: &[u8]) -> io::Result<()> {
    let amount = unsafe { libc::write(
        fd,
        message.as_ptr() as *const c_void,
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

pub trait Reader: Sync {
    fn read_message(&self) -> Result<Option<String>, Box<dyn Error>>;
}

struct FdHolder {
    fd: c_int,
}

impl FdHolder {
    fn new(fd: c_int) -> Self {
        Self { fd: fd }
    }

}

impl std::ops::Deref for FdHolder {
    type Target = c_int;

    fn deref(&self) -> &c_int { &self.fd }
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

struct SocketReader {
    fd: FdHolder,
}

impl SocketReader {
    fn new(follow_symlinks: bool, paths: Vec<PathBuf>) -> io::Result<Self> {
        let mut fds: [c_int; 2] = [0; 2];

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

        std::thread::spawn(move || {
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
    fn read_message(&self) -> Result<Option<String>, Box<dyn Error>> {
        let mut buf = vec![0u8; 4096];
        let result = unsafe {
            libc::read(
                *self.fd,
                buf.as_mut_ptr() as *mut c_void,
                buf.len())
        };

        if result < 0 { return Err(Box::new(io::Error::last_os_error())); }
        if result == 0 { return Ok(None); }

        buf.resize(result.try_into().unwrap(), 0u8);

        let parsed = String::from_utf8(buf)?;
        Ok(Some(parsed))
    }
}

struct ListReader {
    cur: AtomicUsize,
    paths: Vec<PathBuf>,
}

impl ListReader {
    fn new(paths: Vec<PathBuf>) -> io::Result<Self> {
        Ok(Self {
            cur: AtomicUsize::new(0usize),
            paths: paths,
        })
    }

    fn get_next(&self) -> Option<usize> {
        let mut val = self.cur.load(Ordering::Relaxed);
        loop {
            if val >= self.paths.len() { return None; }
            let result = self.cur.compare_exchange_weak(
                val,
                val + 1,
                Ordering::Relaxed, Ordering::Relaxed);
            if result.is_ok() { return Some(val); }
            val = result.err().unwrap();
        }
    }
}

impl Reader for ListReader {
    fn read_message(&self) -> Result<Option<String>, Box<dyn Error>> {
        let idx = self.get_next();
        if idx.is_none() { return Ok(None); }
        let path = self.paths[idx.unwrap()]
            .to_str()
            .expect("decode failed")
            .to_owned();
        return Ok(Some(path));
    }
}

pub fn create(
    recursive: bool,
    follow_symlinks: bool,
    paths: Vec<PathBuf>) -> io::Result<Box<dyn Reader>> {
    if recursive {
        return Ok(Box::new(SocketReader::new(follow_symlinks, paths)?));
    }
    return Ok(Box::new(ListReader::new(paths)?));
}
