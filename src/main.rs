mod file;
mod finders;
mod transmitter;

use std::collections::HashMap;
use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};

use clap::{Parser};

type BoolResult = Result<bool, Box<dyn std::error::Error>>;

fn is_data_not_found_error(err: &std::io::Error) -> bool {
    if let Some(raw) = err.raw_os_error() {
        return raw == file::ERRNO_ENODATA;
    }
    return false;
}

fn not_found(what: &str) -> BoolResult {
    let message = format!("Not found: {what}");
    return Err(Box::new(Error::new(ErrorKind::NotFound, message)));
}

#[derive(clap::Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// List of hash algorithms to use.
    #[arg(short = 'C', default_values = ["blake2b512", "sha3-512", "sha512"])]
    hashes: Vec<String>,

    /// Set task: Identify files without hashes.
    #[arg(short = 'H', group = "job")]
    find_unhashed: bool,

    /// Set task: Check hashes.
    #[arg(short = 'c', group = "job")]
    check_hashes: bool,

    /// Set task: Print hashes.
    #[arg(short = 'p', group = "job")]
    print_hashes: bool,

    /// Set task: Reset (clear) hashes.
    #[arg(short = 'r', group = "job")]
    reset_hashes: bool,

    /// Set task: Set hashes (Find file's hash and set it in metadata).
    #[arg(short = 's', group = "job")]
    set_hashes: bool,

    /// Follow symbolic links.
    #[arg(short = 'L')]
    follow_symlinks: bool,

    /// Recurse: Run recursively over the directories given by paths.
    #[arg(short = 'R')]
    recurse: bool,

    /// Number of threads.
    #[arg(short = 't', group = "threads")]
    threads_count: Option<usize>,

    /// Use as many threads as there are CPUs.
    #[arg(short = 'T', group = "threads")]
    all_threads: bool,

    /// Enable verbose mode.
    #[arg(short = 'v')]
    verbose: bool,

    /// Paths on which to operate.
    paths: Vec<PathBuf>,

}

impl Args {
    fn get_job_name(&self) -> Option<String> {
        if self.check_hashes { return Some("checker".into()); }
        if self.find_unhashed { return Some("find_unhashed".into()); }
        if self.print_hashes { return Some("print_hashes".into()); }
        if self.reset_hashes { return Some("reset_hashes".into()); }
        if self.set_hashes { return Some("hasher".into()); }
        return None;
    }

    fn get_prog_name(&self) -> Option<String> {
        let arg0 = std::env::args().next()?;
        let path = Path::new(&arg0);
        let name = path.file_name()?.to_str()?.into();
        return Some(name);
    }

    fn get_worker_count(&self) -> usize {
        if let Some(count) = self.threads_count { return count; }
        if self.all_threads {
            return std::thread::available_parallelism()
                .expect("available_parallelism failed")
                .get();
        }
        return 1;
    }
}

struct HashUtil {
    hashes: Vec<String>,
    producer: Box<dyn finders::Reader>,
    verbose: bool,
}

impl HashUtil {
    fn new(hashes: Vec<String>, producer: Box<dyn finders::Reader>, verbose: bool) -> Self {
        Self {
            hashes: hashes,
            producer: producer,
            verbose: verbose,
        }
    }

    fn print_one(&self, hash: &[u8], hashtype: &str, path: &str) {
        let get_byte = |byte: &u8| {
            let nybbles = [
                '0', '1', '2', '3', '4', '5', '6', '7',
                '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
            ];
            return [
                nybbles[usize::from(byte >> 4 & 0x0f)],
                nybbles[usize::from(byte >> 0 & 0x0f)],
            ];
        };
        let hex: String = hash.iter()
            .map(get_byte)
            .flatten()
            .collect();
        if self.hashes.len() == 1 {
            println!("{}  {}", hex, path);
        } else {
            println!("{} [{:>10}] {}", hex, hashtype, path);
        }
    }

    fn print_hash(&self, path: &Path) -> BoolResult {
        if !path.is_file() { return not_found(path.to_str().unwrap()); }

        let file = file::File::new(path);

        for hash in self.hashes.iter() {
            let result = file.get_attr(hash);
            if let Ok(value) = result {
                self.print_one(&value, hash, path.to_str().unwrap());
                continue;
            }
            if let Err(e) = result {
                if is_data_not_found_error(&e) { continue; }
                return Err(Box::new(e));
            }
        }

        return Ok(true);
    }

    fn find_unhashed(&self, path: &Path) -> BoolResult {
        if !path.is_file() { return not_found(path.to_str().unwrap()); }

        let file = file::File::new(path);

        let hashes = self.hashes.iter().map(|hash| file.get_attr(hash));

        for hash in hashes {
            if hash.is_ok() { continue; }

            let err: Error = hash.err().unwrap();
            if is_data_not_found_error(&err) {
                println!("{}", path.display());
                break;
            }

            return Err(Box::new(err));
        }

        return Ok(true);
    }

    fn check_hash(&self, path: &Path) -> BoolResult {
        if !path.is_file() { return not_found(path.to_str().unwrap()); }

        let file = file::File::new(path);

        let expected_hashes: HashMap<String, Vec<u8>> = self.hashes.iter()
            .map(|hash| (hash, file.get_attr(hash)))
            .filter(|(_, val)| val.is_ok())
            .map(|(hash, val)| (hash.clone(), val.unwrap()))
            .collect();

        let hashes_to_check: Vec<&str> = expected_hashes.keys().map(String::as_str).collect();

        let actual_hashes: HashMap<String, Vec<u8>> = file.find_hashes(&hashes_to_check)?;

        let mut ret = true;
        for hash in hashes_to_check {
            let key: String = hash.into();
            let expected: &Vec<u8> = expected_hashes.get(&key).unwrap();
            let actual: &Vec<u8> = actual_hashes.get(&key).unwrap();

            let was_ok = expected == actual;
            let status = if was_ok { "OK" } else { "Failed" };

            if self.hashes.len() > 1 {
                println!("{}: {} {}", path.to_str().unwrap(), hash, status);
            } else {
                println!("{}: {}", path.to_str().unwrap(), status);
            }

            ret &= was_ok;
        }

        return Ok(ret);
    }

    fn set_hashes(&self, path: &Path) -> BoolResult {
        if !path.is_file() { return not_found(path.to_str().unwrap()); }

        let file = file::File::new(path);

        fn missing_hash(result: &Result<Vec<u8>, Error>) -> bool {
            if result.is_ok() { return false; }
            let err = result.as_ref().err().unwrap();
            return is_data_not_found_error(&err);
        }

        let needed: Vec<&str> = self.hashes.iter()
            .map(|hash| (hash, file.get_attr(hash)))
            .filter(|(_hash, val)| missing_hash(val))
            .map(|(hash, _val)| hash.as_str())
            .collect();

        if needed.is_empty() { return Ok(true) }

        let hashes = file.find_hashes(&needed)?;

        for (hash, val) in hashes {
            file.set_attr(&hash, &val)?;
            self.print_one(&val, &hash, path.to_str().unwrap());
        }

        return Ok(true);
    }

    fn reset_hashes(&self, path: &Path) -> BoolResult {
        if !path.is_file() { return not_found(path.to_str().unwrap()); }

        let file = file::File::new(path);

        for hash in self.hashes.iter() {
            if let Err(e) = file.remove_attr(hash) {
                if is_data_not_found_error(&e) { continue; }
                return Err(Box::new(e));
            }
            if self.verbose { println!("{}: removed {}", path.display(), hash); }
        }

        return Ok(true);
    }

    fn run_loop<T: Fn(&Self, &Path) -> BoolResult>(&self, callback: T) -> bool {
        let mut ret = true;
        loop {
            let msg = match self.producer.read_message() {
                Err(e) => panic!("Failed to read message: {e}"),
                Ok(None) => break,
                Ok(Some(msg)) => msg,
            };

            match callback(&self, Path::new(&msg)) {
                Ok(result) => ret &= result,
                Err(e) => {
                    eprintln!("Worker encountered error: {e}");
                    self.producer.kill();
                    return false;
                },
            }
        }
        ret
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let task = args.get_job_name().or_else(|| args.get_prog_name());
    if task.is_none() { return Ok(()); }

    let job = match task.unwrap().as_str() {
        "checker"       => HashUtil::check_hash,
        "find_unhashed" => HashUtil::find_unhashed,
        "print_hashes"  => HashUtil::print_hash,
        "reset_hashes"  => HashUtil::reset_hashes,
        "hasher"        => HashUtil::set_hashes,
        _               => panic!("UNKNOWN TASK"),
    };

    let num_workers = args.get_worker_count();

    let hash_util = HashUtil::new(
        args.hashes,
        finders::create(args.recurse, args.follow_symlinks, args.paths)?,
        args.verbose,
    );

    let result = std::thread::scope(|s| {
        let mut workers = Vec::new();

        for _ in 1..num_workers {
            let hash_util = &hash_util;
            workers.push(s.spawn(move || hash_util.run_loop(&job)));
        }

        let mut ret = hash_util.run_loop(&job);
        for worker in workers {
            let result = worker.join().expect("Failed to join worker");
            ret &= result;
        }

        ret
    });

    if !result { std::process::exit(1); }

    Ok(())
}
