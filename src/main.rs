mod finders;

use finders::Reader;

fn main() {
    let rbox = Box::new(finders::SocketReader::new().expect("failed"));
    let r = rbox.as_ref();
    loop {
        match r.read_message() {
            Ok(Some(msg)) => println!("got {}", msg),
            Ok(None) => break,
            Err(e) => panic!("error: {}", e),
        }
    }
}
