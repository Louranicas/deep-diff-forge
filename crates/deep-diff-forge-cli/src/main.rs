use deep_diff_forge_core::ReviewDocument;

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None | Some("--help") | Some("-h") | Some("help") => print_help(),
        Some("--version") | Some("-V") | Some("version") => print_version(),
        Some("--self-test") | Some("self-test") => self_test(),
        Some("doctor") => doctor(),
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

fn print_help() {
    println!(
        "\
deep-diff-forge {version}

USAGE:
  deep-diff-forge --help
  deep-diff-forge --version
  deep-diff-forge --self-test
  deep-diff-forge doctor
  deep-diff-forge claude-code-contract
  deep-diff-forge chain-contract
  deep-diff-forge cluster-contract
  deep-diff-forge loom-contract

BOOTSTRAP STATUS:
  The current binary exposes deployability smoke commands while the full
  patch, semantic, TUI, daemon, and agent surfaces are implemented.

FUTURE PRIMARY MODES:
  deep-diff-forge <old> <new>
  deep-diff-forge --stdin-patch
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
    let cache_dir = std::env::var("XDG_CACHE_HOME")
        .map(|base| format!("{base}/deep-diff-forge"))
        .unwrap_or_else(|_| "~/.cache/deep-diff-forge".into());
    let state_dir = std::env::var("XDG_STATE_HOME")
        .map(|base| format!("{base}/deep-diff-forge"))
        .unwrap_or_else(|_| "~/.local/state/deep-diff-forge".into());

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
