use std::{
    collections::HashMap,
    ffi::{
        CString,
        c_int,
        c_void,
    },
    fs,
    io::{
        BufReader,
        Error,
        ErrorKind,
        Read,
    },
    path::{
        Path,
        PathBuf,
    },
};
use openssl::hash::{
    Hasher,
    MessageDigest,
};

pub struct File {
    path: PathBuf,
}

pub const ERRNO_ENODATA: i32 = 61;

fn xattr_name(hash: &str) -> String { return format!("user.hash.{}", hash); }

impl File {
    pub fn new(path: &Path) -> Self {
        return Self { path: PathBuf::from(path) };
    }

    pub fn path(&self) -> &Path { return self.path.as_path(); }

    fn call_getxattr(&self, attr_name: &str, result: &mut Vec<u8>) -> Result<usize, Error> {
        let path = CString::new(self.path().to_str().unwrap())?;
        let name = CString::new(xattr_name(attr_name).to_string()).unwrap();
        let result_ptr = result.as_mut_ptr() as *mut libc::c_void;

        let amount: isize = unsafe {
            libc::getxattr(path.as_ptr(), name.as_ptr(), result_ptr, result.len())
        };
        if amount < 0 {
            return Err(Error::last_os_error());
        }
        return Ok(amount.try_into().unwrap());
    }

    pub fn get_attr(&self, attrname: &str) -> Result<Vec<u8>, Error> {
        let len: usize = self.call_getxattr(attrname, &mut Vec::new())?;

        let mut result = vec![0u8; len];
        let got: usize = self.call_getxattr(attrname, &mut result)?;

        if len != got {
            return Err(Error::new(ErrorKind::Other, "Xattr size changed"));
        }

        return Ok(result);
    }

    pub fn get_attr_hex(&self, attrname: &str) -> Result<String, Error> {
        let bytes: Vec<u8> = self.get_attr(attrname)?;
        return Ok(bytes.iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join(""));
    }

    pub fn remove_attr(&self, attrname: &str) -> Result<(), Error> {
        let path = CString::new(self.path().to_str().unwrap())?;
        let name = CString::new(xattr_name(attrname).to_string()).unwrap();

        let result: i32 = unsafe { libc::removexattr(path.as_ptr(), name.as_ptr()) };

        if result < 0 { return Err(Error::last_os_error()); }

        return Ok(());
    }

    pub fn set_attr(&self, attr_name: &str, value: &[u8]) -> Result<(), Error> {
        let path = CString::new(self.path().to_str().unwrap())?;
        let name = CString::new(xattr_name(attr_name).to_string()).unwrap();
        let val = value.as_ptr() as *const libc::c_void;

        let result: i32 = unsafe {
            libc::setxattr(path.as_ptr(), name.as_ptr(), val, value.len(), 0)
        };
        if result < 0 { return Err(Error::last_os_error()); }
        return Ok(());
    }

    pub fn find_hashes(&self, hashes: &[&str]) -> Result<HashMap<String, Vec<u8>>, Error> {
        let mut hashers = hashes.iter()
            .map(|name| -> Result<MessageDigest, Error> {
                let digest = MessageDigest::from_name(name);
                if let Some(ret) = digest { return Ok(ret); }
                let msg = format!("unknown hash {}", name);
                return Err(Error::new(ErrorKind::NotFound, msg));
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|digest| Hasher::new(digest))
            .collect::<Result<Vec<_>, _>>()?;

        let mut reader = self.open()?;
        loop {
            let mut buf = [0u8; 4096];
            let amount = reader.read(&mut buf)?;
            if amount == 0 { break; }
            for hasher in hashers.iter_mut() { hasher.update(&buf[..amount])?; }
        }

        let mut ret = HashMap::new();
        for (i, mut hasher) in hashers.into_iter().enumerate() {
            let digest_bytes = hasher.finish()?.into_iter().copied().collect();
            ret.insert(hashes[i].to_owned(), digest_bytes);
        }
        return Ok(ret);
    }

    fn open(&self) -> Result<BufReader<fs::File>, Error> {
        let file = fs::File::open(&self.path)?;
        return Ok(BufReader::new(file));
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Write};
    use super::*;

    const FAKEHASH: &'static str = "notarealhash";
    const SHA512: &'static str = "sha512";
    const BLAKE2B: &'static str = "blake2b512";
    const DEFAULT_HASHES: [&'static str; 2] = [SHA512, BLAKE2B];

    #[test]
    fn test_new() {
        let tempfile = tempfile::NamedTempFile::new().unwrap();
        let path: &Path = tempfile.path();

        let file = File::new(path);
        assert_eq!(file.path(), path);
    }

    #[test]
    fn test_get_set_and_reset() {
        const HASH_NAME: &str = "sha512";
        const HASH_VAL: &[u8; 5] = b"dummy";
        const HASH_HEX: &str = "64756d6d79";

        let tempfile = tempfile::NamedTempFile::new().unwrap();
        let path: &Path = tempfile.path();

        let file: File = File::new(path);

        // Before...
        assert_eq!(file.get_attr(HASH_NAME).err().unwrap().raw_os_error(), Some(ERRNO_ENODATA));
        assert_eq!(file.get_attr_hex(HASH_NAME).err().unwrap().raw_os_error(), Some(ERRNO_ENODATA));
        assert_eq!(file.remove_attr(HASH_NAME).err().unwrap().raw_os_error(), Some(ERRNO_ENODATA));

        // set...
        assert!(file.set_attr(HASH_NAME, HASH_VAL).is_ok());

        // Get afterwards...
        assert_eq!(file.get_attr(HASH_NAME).unwrap(), Vec::from(HASH_VAL));
        assert_eq!(file.get_attr_hex(HASH_NAME).unwrap(), HASH_HEX);

        // Reset...
        assert!(file.remove_attr(HASH_NAME).is_ok());

        // After...
        assert_eq!(file.get_attr(HASH_NAME).err().unwrap().raw_os_error(), Some(ERRNO_ENODATA));
        assert_eq!(file.get_attr_hex(HASH_NAME).err().unwrap().raw_os_error(), Some(ERRNO_ENODATA));
        assert_eq!(file.remove_attr(HASH_NAME).err().unwrap().raw_os_error(), Some(ERRNO_ENODATA));
    }

    #[test]
    fn test_find_hashes_empty_happy_path() {
        let sha512_empty: Vec<u8> = vec![
            0xcf, 0x83, 0xe1, 0x35, 0x7e, 0xef, 0xb8, 0xbd, 0xf1, 0x54, 0x28, 0x50, 0xd6, 0x6d,
            0x80, 0x07, 0xd6, 0x20, 0xe4, 0x05, 0x0b, 0x57, 0x15, 0xdc, 0x83, 0xf4, 0xa9, 0x21,
            0xd3, 0x6c, 0xe9, 0xce, 0x47, 0xd0, 0xd1, 0x3c, 0x5d, 0x85, 0xf2, 0xb0, 0xff, 0x83,
            0x18, 0xd2, 0x87, 0x7e, 0xec, 0x2f, 0x63, 0xb9, 0x31, 0xbd, 0x47, 0x41, 0x7a, 0x81,
            0xa5, 0x38, 0x32, 0x7a, 0xf9, 0x27, 0xda, 0x3e];
        let blake2b_empty: Vec<u8> = vec![
            0x78, 0x6a, 0x02, 0xf7, 0x42, 0x01, 0x59, 0x03, 0xc6, 0xc6, 0xfd, 0x85, 0x25, 0x52,
            0xd2, 0x72, 0x91, 0x2f, 0x47, 0x40, 0xe1, 0x58, 0x47, 0x61, 0x8a, 0x86, 0xe2, 0x17,
            0xf7, 0x1f, 0x54, 0x19, 0xd2, 0x5e, 0x10, 0x31, 0xaf, 0xee, 0x58, 0x53, 0x13, 0x89,
            0x64, 0x44, 0x93, 0x4e, 0xb0, 0x4b, 0x90, 0x3a, 0x68, 0x5b, 0x14, 0x48, 0xb7, 0x55,
            0xd5, 0x6f, 0x70, 0x1a, 0xfe, 0x9b, 0xe2, 0xce];

        let tempfile = tempfile::NamedTempFile::new().unwrap();
        let file = File::new(tempfile.path());

        let maybe_result = file.find_hashes(&DEFAULT_HASHES);
        assert!(maybe_result.is_ok());

        let result = maybe_result.unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[SHA512], sha512_empty);
        assert_eq!(result[BLAKE2B], blake2b_empty);
    }

    #[test]
    fn test_find_hashes_happy_path() {
        let blake2b: Vec<u8> = vec![
            0x80, 0xfe, 0x13, 0x86, 0x08, 0x15, 0xf4, 0xa0, 0x18, 0xad, 0x50, 0x75, 0xbf, 0xb6,
            0x84, 0x4c, 0xa2, 0x4b, 0x59, 0x63, 0xb6, 0x06, 0x4a, 0x3b, 0x39, 0x12, 0x24, 0x0a,
            0x58, 0x24, 0xba, 0x34, 0xef, 0x71, 0xd2, 0xe3, 0x28, 0x70, 0xaf, 0x66, 0xb1, 0x05,
            0x4c, 0x94, 0xd6, 0x54, 0x36, 0x44, 0x6f, 0xff, 0x8c, 0xa8, 0x44, 0x66, 0x7d, 0xe5,
            0x0e, 0xf8, 0xf7, 0x00, 0xf9, 0x23, 0x43, 0x01];
        let sha512: Vec<u8> = vec![
            0x09, 0xe1, 0xe2, 0xa8, 0x4c, 0x92, 0xb5, 0x6c, 0x82, 0x80, 0xf4, 0xa1, 0x20, 0x3c,
            0x7c, 0xff, 0xd6, 0x1b, 0x16, 0x2c, 0xfe, 0x98, 0x72, 0x78, 0xd4, 0xd6, 0xbe, 0x9a,
            0xfb, 0xf3, 0x8c, 0x0e, 0x89, 0x34, 0xcd, 0xad, 0xf8, 0x37, 0x51, 0xf4, 0xe9, 0x9d,
            0x11, 0x13, 0x52, 0xbf, 0xfe, 0xfc, 0x95, 0x8e, 0x5a, 0x48, 0x52, 0xc8, 0xa7, 0xa2,
            0x9c, 0x95, 0x74, 0x2c, 0xe5, 0x92, 0x88, 0xa8];

        let mut tempfile = tempfile::NamedTempFile::new().unwrap();
        writeln!(tempfile, "Hello, world!").expect("write failed");
        let file = File::new(tempfile.path());

        let maybe_result = file.find_hashes(&DEFAULT_HASHES);
        assert!(maybe_result.is_ok());

        let result = maybe_result.unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[SHA512], sha512);
        assert_eq!(result[BLAKE2B], blake2b);
    }

    #[test]
    fn test_find_hashes_invalid_hash() {
        let tempfile = tempfile::NamedTempFile::new().unwrap();
        let file = File::new(tempfile.path());

        assert!(file.find_hashes(&[FAKEHASH]).is_err());
        assert!(file.find_hashes(&[SHA512, BLAKE2B, FAKEHASH]).is_err());
    }
}
