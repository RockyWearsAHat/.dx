//! What the binary does when the thing reading it walks away.
//!
//! `dx text long.dx | head` is an ordinary gesture, and the moment `head` has its lines it
//! closes the pipe. Rust's `print!` answers that by panicking, so the shell showed a
//! backtrace for a command that had done exactly what was asked. Only the real process can
//! show this — a unit test has no pipe to close — so it lives here.

use std::io::Write;
use std::process::{Command, Stdio};

/// A document large enough that writing it cannot fit in the pipe buffer, so the write
/// reaches a reader that is no longer there instead of a kernel buffer that absorbs it.
fn long_document() -> String {
    let mut text = String::new();
    for line in 0..4000 {
        text.push_str(&format!(
            "::paragraph id=p{line}\nA line of ordinary prose, number {line}.\n::end\n\n"
        ));
    }
    text
}

#[test]
fn a_reader_that_stops_reading_ends_the_command_rather_than_crashing_it() {
    let directory = std::env::temp_dir().join(format!("dx-pipes-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("a temporary directory");
    let document = directory.join("long.dx");
    let mut file = std::fs::File::create(&document).expect("a document to read");
    file.write_all(long_document().as_bytes()).expect("written");
    drop(file);

    let mut child = Command::new(env!("CARGO_BIN_EXE_dx"))
        .arg("text")
        .arg(&document)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("dx runs");

    // Closing the read end is what `head` does when it has enough.
    drop(child.stdout.take());
    let finished = child.wait_with_output().expect("dx finishes");

    let complaint = String::from_utf8_lossy(&finished.stderr);
    assert!(
        !complaint.contains("panicked"),
        "dx panicked writing to a closed pipe: {complaint}"
    );
    assert!(
        finished.status.success(),
        "a reader that stopped reading is not a failed command: {finished:?}"
    );

    std::fs::remove_dir_all(&directory).ok();
}
