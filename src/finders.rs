use std::collections::HashSet;
use std::error::Error;
use std::fs::{self, DirEntry};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::transmitter;

struct Visitor {
    seen: HashSet<PathBuf>,
    follow_symlinks: bool,
}

impl Visitor {
    fn new(follow_symlinks: bool) -> Self {
        Self {
            seen: HashSet::new(),
            follow_symlinks: follow_symlinks,
        }
    }

    fn visit<F>(&mut self, dir: &Path, cb: F) -> io::Result<()>
        where F: Fn(&DirEntry) -> bool + Copy {
        let canon = dir.canonicalize()?;
        if self.seen.contains(&canon) { return Ok(()); }
        self.seen.insert(canon);

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path: PathBuf = entry.path();
            if !self.follow_symlinks && path.is_symlink() { continue; }
            if path.is_dir() {
                self.visit(&path, cb)?;
            }
            if path.is_file() {
                if cb(&entry) { continue; }
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }
        }
        Ok(())
    }
}

pub trait Reader: Sync {
    fn read_message(&self) -> Result<Option<PathBuf>, Box<dyn Error>>;
    fn kill(&self);
}

struct ThreadReader {
    transmitter: Arc<transmitter::Transmitter<PathBuf>>,
}

impl ThreadReader {
    fn new(follow_symlinks: bool, paths: Vec<PathBuf>) -> io::Result<Self> {
        let paths = if !paths.is_empty() {
            paths
        } else {
            vec![PathBuf::from(".")]
        };

        let trx = Arc::new(transmitter::Transmitter::<PathBuf>::new(1024));

        let write_end = Arc::clone(&trx);
        std::thread::spawn(move || {
            let mut visitor = Visitor::new(follow_symlinks);
            let write_end = &*write_end;
            let _closer = write_end.closer();
            for path in paths {
                visitor.visit(&path, |entry| -> bool {
                    write_end.put(entry.path())
                }).expect("Visitor::visit() failed");
            }
        });

        Ok(Self { transmitter: trx })
    }
}

impl Reader for ThreadReader {
    fn read_message(&self) -> Result<Option<PathBuf>, Box<dyn Error>> {
        return Ok(self.transmitter.get());
    }

    fn kill(&self) { self.transmitter.kill(); }
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
    fn read_message(&self) -> Result<Option<PathBuf>, Box<dyn Error>> {
        let idx = self.get_next();
        if idx.is_none() { return Ok(None); }
        let path = self.paths[idx.unwrap()].clone();
        return Ok(Some(path));
    }

    fn kill(&self) {}
}

pub fn create(
    recursive: bool,
    follow_symlinks: bool,
    paths: Vec<PathBuf>) -> io::Result<Box<dyn Reader>> {
    if recursive {
        return Ok(Box::new(ThreadReader::new(follow_symlinks, paths)?));
    }
    return Ok(Box::new(ListReader::new(paths)?));
}
