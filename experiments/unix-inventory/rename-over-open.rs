// Does std::fs::rename replace a file that a reader holds open with std's
// default File::open? (gate step 3, atomic-replace claim)
use std::io::Read;
fn main() {
    let d = std::env::args().nth(1).unwrap();
    let target = format!("{d}/state.json");
    let tmp = format!("{d}/state.json.tmp");
    std::fs::write(&target, b"old").unwrap();
    let mut reader = std::fs::File::open(&target).unwrap(); // default share mode
    std::fs::write(&tmp, b"new").unwrap();
    match std::fs::rename(&tmp, &target) {
        Ok(()) => println!("rename over an open file: OK"),
        Err(e) => println!("rename over an open file: FAILED: {e}"),
    }
    let mut s = String::new();
    reader.read_to_string(&mut s).unwrap();
    println!("open reader still reads: {s}");
    drop(reader);
    println!("path now holds: {}", std::fs::read_to_string(&target).unwrap());
}
