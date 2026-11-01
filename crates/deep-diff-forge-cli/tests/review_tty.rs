//! Bare `review` in a real pseudo-terminal: the live loop as a user meets it. The terminal is put
//! in the alternate screen with mouse capture, the cockpit is drawn, a `q` typed at the keyboard
//! quits with exit 0, and the terminal is restored. The loop's other paths (bursts, resizes,
//! failing input, panics) are driven headlessly in `deep-diff-forge-tui`'s `run.rs` tests.

#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes"
)]

use std::io::Write as _;
use std::process::{Command, Stdio};

const PATCH: &str = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,2 @@
 fn keep() {}
-let old = 1;
+let new = 2;
";

/// Runs the binary under `pty.fork()` (a new session whose controlling terminal is the pty, so
/// crossterm reads the keyboard there whatever terminal runs the tests), with the patch on stdin
/// and a 100x30 window. Once the alternate screen is entered and the menu bar drawn, it types `q`.
/// It prints `exit=<code> typed=<bool> timed_out=<bool>`, then everything the program wrote to the
/// terminal. A 30 s deadline kills a program that never exits.
const DRIVER: &str = r#"
import fcntl, os, pty, select, struct, sys, termios, time
binary = sys.argv[1]
patch = sys.stdin.buffer.read()
r, w = os.pipe()
pid, master = pty.fork()
if pid == 0:
    os.close(w)
    os.dup2(r, 0)
    fcntl.ioctl(1, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    os.execv(binary, [binary, "review"])
os.close(r)
os.write(w, patch)
os.close(w)
out, typed, timed_out = b"", False, False
deadline = time.monotonic() + 30
while True:
    if not typed and b"\x1b[?1049h" in out and b"deep-diff-forge" in out:
        os.write(master, b"q")
        typed = True
    if time.monotonic() > deadline:
        timed_out = True
        os.kill(pid, 9)
        break
    ready, _, _ = select.select([master], [], [], 0.1)
    if ready:
        try:
            chunk = os.read(master, 65536)
        except OSError:
            break
        if not chunk:
            break
        out += chunk
_, status = os.waitpid(pid, 0)
sys.stdout.write(f"exit={os.waitstatus_to_exitcode(status)} typed={typed} timed_out={timed_out}\n")
sys.stdout.flush()
sys.stdout.buffer.write(out)
"#;

#[test]
fn review_in_a_terminal_draws_the_cockpit_and_quits_on_q_restoring_the_terminal() {
    let mut child = Command::new("python3")
        .args(["-I", "-c", DRIVER, env!("CARGO_BIN_EXE_deep-diff-forge")])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("python3 runs the pty driver");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(PATCH.as_bytes())
        .expect("write the patch");
    let out = child.wait_with_output().expect("wait");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "driver failed: {stderr}");
    let text = String::from_utf8_lossy(&out.stdout);
    let (status, screen) = text.split_once('\n').expect("a status line");
    assert_eq!(
        status, "exit=0 typed=True timed_out=False",
        "terminal output: {screen:?}\nstderr: {stderr}"
    );
    let enter = screen
        .find("\x1b[?1049h")
        .expect("the alternate screen is entered");
    let drawn = screen
        .find("deep-diff-forge")
        .expect("the cockpit is drawn");
    let leave = screen
        .rfind("\x1b[?1049l")
        .expect("the alternate screen is left");
    assert!(enter < drawn && drawn < leave, "{screen:?}");
    assert!(screen.contains("\x1b[?1000h"), "mouse capture is enabled");
    assert!(
        screen[drawn..].contains("\x1b[?1000l"),
        "and disabled again"
    );
    assert!(
        screen[drawn..leave].contains("Files (ranked)"),
        "the file tree is drawn"
    );
    assert!(
        screen[drawn..leave].contains("src/lib.rs"),
        "with the patch's file"
    );
}
