use deep_diff_forge_core::ReviewDocument;

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None | Some("--help" | "-h" | "help") => print_help(),
        Some("--version" | "-V" | "version") => print_version(),
        Some("--self-test" | "self-test") => self_test(),
        Some("doctor") => doctor(),
        Some("--stdin-patch") => stdin_patch(args.any(|a| a == "--json")),
        Some("claude-code-contract") => claude_code_contract(),
        Some("chain-contract") => chain_contract(),
        Some("cluster-contract") => cluster_contract(),
        Some("loom-contract") => loom_contract(),
        Some(command) => {
            eprintln!("unknown command: {command}");
            eprintln!("run `deep-diff-forge --help` for supported bootstrap commands");
            std::process::exit(2);
        }
    }
}

/// Read a unified/Git patch from stdin and emit either a human review summary
/// (default) or the `deep-diff-forge.review.v0` JSON document (`--json`).
///
/// Exit codes follow the CLI contract: 3 = input read failure,
/// 4 = patch parse failure.
fn stdin_patch(json: bool) {
    use std::io::Read as _;
    let mut input = String::new();
    if let Err(err) = std::io::stdin().read_to_string(&mut input) {
        eprintln!("error: could not read stdin: {err}");
        std::process::exit(3);
    }
    match deep_diff_forge_patch::parse(&input) {
        Ok(files) => {
            if json {
                print!("{}", deep_diff_forge_patch::to_json(&files));
            } else {
                print_patch_summary(&files);
            }
        }
        Err(err) => {
            eprintln!("error: patch parse failed: {err}");
            std::process::exit(4);
        }
    }
}

fn print_patch_summary(files: &[deep_diff_forge_core::ReviewFile]) {
    use deep_diff_forge_core::PatchLineKind;
    for file in files {
        let mut adds = 0usize;
        let mut dels = 0usize;
        for hunk in &file.patch_twin.hunks {
            for line in &hunk.lines {
                match line.kind {
                    PatchLineKind::Added => adds += 1,
                    PatchLineKind::Removed => dels += 1,
                    PatchLineKind::Context => {}
                }
            }
        }
        let hunks = file.patch_twin.hunks.len();
        let status = format!("{:?}", file.status).to_lowercase();
        println!(
            "{status:>14}  {}  (+{adds} -{dels}, {hunks} hunks)",
            file.path
        );
    }
    println!("{} file(s) changed", files.len());
}

fn print_help() {
    println!(
        "\
deep-diff-forge {version}

USAGE:
  deep-diff-forge --help
  deep-diff-forge --version
  deep-diff-forge --self-test
  deep-diff-forge doctor
  deep-diff-forge --stdin-patch [--json]
  deep-diff-forge claude-code-contract
  deep-diff-forge chain-contract
  deep-diff-forge cluster-contract
  deep-diff-forge loom-contract

BOOTSTRAP STATUS:
  The current binary parses unified/Git patches (--stdin-patch, L1 patch
  maturity) and exposes deployability smoke commands, while the semantic, TUI,
  daemon, and agent surfaces are designed but not yet implemented.

FUTURE PRIMARY MODES:
  deep-diff-forge <old> <new>
  deep-diff-forge --git
  deep-diff-forge review
  deep-diff-forge chain
  deep-diff-forge cluster
  deep-diff-forge loom
  deep-diff-forge daemon start
",
        version = env!("CARGO_PKG_VERSION")
    );
}

fn print_version() {
    println!("deep-diff-forge {}", env!("CARGO_PKG_VERSION"));
}

fn self_test() {
    let document = ReviewDocument::empty();
    assert!(document.files.is_empty());
    println!("self-test ok: core model initialized");
}

fn doctor() {
    let runtime_dir =
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp/deep-diff-forge-runtime".into());
    let cache_dir = std::env::var("XDG_CACHE_HOME").map_or_else(
        |_| "~/.cache/deep-diff-forge".into(),
        |base| format!("{base}/deep-diff-forge"),
    );
    let state_dir = std::env::var("XDG_STATE_HOME").map_or_else(
        |_| "~/.local/state/deep-diff-forge".into(),
        |base| format!("{base}/deep-diff-forge"),
    );

    println!("doctor ok: bootstrap binary is executable");
    println!("runtime_dir={runtime_dir}");
    println!("cache_dir={cache_dir}");
    println!("state_dir={state_dir}");
    println!("daemon_socket={runtime_dir}/deep-diff-forge/deep-diff-forge.sock");
}

fn claude_code_contract() {
    println!(
        "\
deep-diff-forge claude-code-contract v0

GUARANTEES:
  - stdout is machine-readable for contract commands unless --human is added later.
  - non-zero exit code means the requested contract failed.
  - patch truth is never replaced by agent annotations.
  - future JSON/JSONL modes will use stable file, hunk, span, and annotation ids.

BOOTSTRAP COMMANDS:
  deep-diff-forge --self-test
  deep-diff-forge doctor
  deep-diff-forge claude-code-contract
"
    );
}

fn chain_contract() {
    println!(
        "\
deep-diff-forge chain-contract v0

GUARANTEES:
  - chainable commands read stdin only when an explicit stdin flag is present.
  - stdout carries primary records; stderr carries diagnostics and progress.
  - JSON output is one complete document; JSONL output is one event per line.
  - every streamed record has stable ids and a schema version.
  - pipe failures exit non-zero without truncating a valid final JSON document.

PLANNED MODES:
  deep-diff-forge --git --json
  deep-diff-forge rank --stdin --json
  deep-diff-forge annotate --stdin --jsonl
  deep-diff-forge render --stdin --plain
"
    );
}

fn cluster_contract() {
    println!(
        "\
deep-diff-forge cluster-contract v0

GUARANTEES:
  - cluster execution preserves patch truth and stable ids across lanes.
  - parallel lanes join by deterministic input order unless ranking is requested.
  - each lane has explicit dimensions, budgets, inputs, outputs, and receipts.
  - daemon acceleration is optional; one-shot CLI execution remains correct.

PLANNED DIMENSIONS:
  patch semantic risk agent runtime storage history presentation
"
    );
}

fn loom_contract() {
    println!(
        "\
deep-diff-forge loom-contract v0

GUARANTEES:
  - loom assimilation produces plans, fixtures, gates, and receipts before merge.
  - generated Rust crate stubs are explicit outputs, never hidden mutation.
  - exemplar lessons are recorded with source, boundary, and adoption decision.
  - unsafe code, network access, and destructive file actions are denied by default.

PLANNED PHASES:
  intake boundary-map weave-plan fixture-synthesis rust-crate-stub gate receipt assimilation
"
    );
}
