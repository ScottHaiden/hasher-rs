mod file;
mod finders;

use std::{
    collections::HashMap,
    io::{
        Error,
        ErrorKind,
    },
    path::{
        Path,
        PathBuf,
    },
};

use clap::{Parser};

type BoxedError = Box<dyn std::error::Error>;
type ErrorOnly = Result<(), BoxedError>;

fn is_data_not_found_error(err: &std::io::Error) -> bool {
    if let Some(raw) = err.raw_os_error() {
        return raw == file::ERRNO_ENODATA;
    }
    return false;
}

fn not_found(what: &str) -> ErrorOnly {
    let message = format!("Not found: {what}");
    return Err(Box::new(Error::new(ErrorKind::NotFound, message)));
}

#[derive(clap::Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// List of hash algorithms to use
    #[arg(short = 'C', default_values = ["sha512", "blake2b512", "sha3-512"])]
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

    /// Set task: Set hashes (Find file's hash and set it in metadata)
    #[arg(short = 's', group = "job")]
    set_hashes: bool,

    /// Follow symbolic links
    #[arg(short = 'L')]
    follow_symlinks: bool,

    /// Recurse: Run recursively over the directories given by paths.
    #[arg(short = 'R')]
    recurse: bool,

    /// Number of threads
    #[arg(short = 't', group = "threads")]
    threads_count: Option<usize>,

    /// Use as many threads as there are CPUs
    #[arg(short = 'T', group = "threads")]
    all_threads: bool,

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

    fn print_one(&self, hash: &str, hashtype: &str, path: &str) {
        println!("{} [{:>10}] {}", hash, hashtype, path);
    }

    fn print_hash(&self, path: &Path) -> ErrorOnly {
        if !path.is_file() { return not_found(path.to_str().unwrap()); }

        let file = file::File::new(path);

        for hash in self.hashes.iter() {
            let result = file.get_attr_hex(hash);
            if let Ok(value) = result {
                self.print_one(&value, hash, path.to_str().unwrap());
                continue;
            }
            if let Err(e) = result {
                if is_data_not_found_error(&e) { continue; }
                return Err(Box::new(e));
            }
        }

        return Ok(());
    }

    fn find_unhashed(&self, path: &Path) -> ErrorOnly {
        if !path.is_file() { return not_found(path.to_str().unwrap()); }

        let file = file::File::new(path);

        let hashes = self.hashes.iter().map(|hash| file.get_attr(hash));

        for hash in hashes {
            if hash.is_ok() { continue; }

            let err: Error = hash.err().unwrap();
            if is_data_not_found_error(&err) {
                println!("{}", path.display());
                return Ok(());
            }

            return Err(Box::new(err));
        }

        return Ok(());
    }

    fn check_hash(&self, path: &Path) -> ErrorOnly {
        if !path.is_file() { return not_found(path.to_str().unwrap()); }

        let file = file::File::new(path);

        let expected_hashes: HashMap<String, Vec<u8>> = self.hashes.iter()
            .map(|hash| (hash, file.get_attr(hash)))
            .filter(|(_, val)| val.is_ok())
            .map(|(hash, val)| (hash.clone(), val.unwrap()))
            .collect();

        let hashes_to_check: Vec<&str> = expected_hashes.keys().map(String::as_str).collect();

        let actual_hashes: HashMap<String, Vec<u8>> = file.find_hashes(&hashes_to_check)?;

        for hash in hashes_to_check {
            let key: String = hash.into();
            let expected: &Vec<u8> = expected_hashes.get(&key).unwrap();
            let actual: &Vec<u8> = actual_hashes.get(&key).unwrap();
            if expected == actual {
                println!("{}: {} OK", path.to_str().unwrap(), hash);
                continue;
            }
            let message: String = format!("{}: incorrect {}", path.to_str().unwrap(), hash);
            return Err(Box::new(Error::new(ErrorKind::InvalidData, message)));
        }

        return Ok(());
    }

    fn set_hashes(&self, path: &Path) -> ErrorOnly {
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

        let hashes = file.find_hashes(&needed)?;

        for (hash, val) in hashes { file.set_attr(&hash, &val)?; }

        return Ok(());
    }

    fn reset_hashes(&self, path: &Path) -> ErrorOnly {
        if !path.is_file() { return not_found(path.to_str().unwrap()); }

        let file = file::File::new(path);

        for hash in self.hashes.iter() {
            if let Err(e) = file.remove_attr(hash) {
                if is_data_not_found_error(&e) { continue; }
                return Err(Box::new(e));
            }
        }

        return Ok(());
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let task = args.get_job_name().or_else(|| args.get_prog_name());
    if task.is_none() { return Ok(()); }

    let job = match task.unwrap().as_str() {
        "checker"       => Args::check_hash,
        "find_unhashed" => Args::find_unhashed,
        "print_hashes"  => Args::print_hash,
        "reset_hashes"  => Args::reset_hashes,
        "hasher"        => Args::set_hashes,
        _               => panic!("UNKNOWN TASK"),
    };

    let num_workers = args.get_worker_count();
    let producer = finders::create(args.recurse, args.follow_symlinks, args.paths.clone())?;

    let result = std::thread::scope(|s| {
        let mut workers = Vec::new();

        for _ in 0..num_workers {
            let args = &args;
            let producer = &producer;

            workers.push(s.spawn(move || {
                loop {
                    let msg = producer.read_message();

                    if let Err(e) = msg {
                        eprintln!("Failed to read message: {}", e);
                        return false;
                    }

                    let msg = msg.ok().unwrap();

                    if msg.is_none() { return true; }

                    let result = job(&args, Path::new(&msg.unwrap()));
                    match result {
                        Ok(_) => continue,
                        Err(e) => {
                            eprintln!("Worker encountered error: {}", e);
                            return false;
                        }
                    }
                }
            }));
        }

        let mut ret = true;
        for worker in workers {
            let result = worker.join().expect("Failed to join worker");
            ret &= result;
        }

        ret
    });

    if !result { std::process::exit(1); }

    Ok(())
}
