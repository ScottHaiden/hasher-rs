mod finders;

use std::{
    fs,
    path,
};

use finders::Reader;

fn main() {
    let producer = finders::SocketReader::new(
        false,
        vec![
            path::PathBuf::from("."),
            path::PathBuf::from("/home/"),
        ],
    ).expect("Failed to create producer");

    loop {
        match producer.read_message() {
            Ok(Some(msg)) => println!("found {msg}"),
            Ok(None) => break,
            Err(e) => panic!("Failed: {e}"),
        }
    }
}
