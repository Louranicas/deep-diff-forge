//! A stand-in ranker for scripts/contract_matrix.sh (row fw-unknown-labels-hold): it runs the
//! deep-diff-forge binary named when it was built (`DDF_UNDER`) with its own arguments and stdin,
//! and adds the label `a_made_up_label` to every file whose signals are exactly `stub_added`.
//! forge-warden runs its ranker from a verified file descriptor, so the stand-in must be a
//! compiled program, not a script. Built by the row with `rustc` in its scratch directory.
use std::io::Write as _;
use std::process::{Command, Stdio};

fn main() {
    let out = Command::new(env!("DDF_UNDER"))
        .args(std::env::args_os().skip(1))
        .stdin(Stdio::inherit())
        .output()
        .expect("the ranker under the stand-in did not run");
    let text = String::from_utf8_lossy(&out.stdout).replace(
        "\"signals\": [\"stub_added\"]",
        "\"signals\": [\"stub_added\", \"a_made_up_label\"]",
    );
    std::io::stdout().write_all(text.as_bytes()).expect("stdout");
    std::io::stderr().write_all(&out.stderr).expect("stderr");
    std::process::exit(out.status.code().unwrap_or(1));
}
