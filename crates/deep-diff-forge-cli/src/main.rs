use deep_diff_forge_core::ReviewDocument;

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None | Some("--help" | "-h" | "help") => print_help(),
        Some("--version" | "-V" | "version") => print_version(),
        Some("--self-test" | "self-test") => self_test(),
        Some("doctor") => doctor(),
        Some("deploy") => {
            let rest: Vec<String> = args.collect();
            deploy_cmd(&rest);
        }
        Some("semantic") => {
            let rest: Vec<String> = args.collect();
            semantic_cmd(&rest);
        }
        Some("review") => {
            let rest: Vec<String> = args.collect();
            review_cmd(&rest);
        }
        Some("--stdin-patch") => {
            let rest: Vec<String> = args.collect();
            stdin_patch(&rest);
        }
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

/// Read a unified/Git patch from stdin and emit one of: the
/// `deep-diff-forge.review.v0` JSON document (`--json`), an inline or
/// side-by-side projection (`--layout inline|side-by-side`), or a human review
/// summary (default).
///
/// Exit codes follow the CLI contract: 2 = usage error, 3 = input read failure,
/// 4 = patch parse failure.
fn stdin_patch(opts: &[String]) {
    use std::io::Read as _;
    let mut input = String::new();
    if let Err(err) = std::io::stdin().read_to_string(&mut input) {
        eprintln!("error: could not read stdin: {err}");
        std::process::exit(3);
    }
    // --jsonl streams one event per file through the real pipeline runner.
    if opts.iter().any(|a| a == "--jsonl") {
        run_jsonl_pipeline(input);
        return;
    }

    let files = match deep_diff_forge_patch::parse(&input) {
        Ok(files) => files,
        Err(err) => {
            eprintln!("error: patch parse failed: {err}");
            std::process::exit(4);
        }
    };

    if opts.iter().any(|a| a == "--rank") {
        let ranked = deep_diff_forge_graph::rank(&files);
        if opts.iter().any(|a| a == "--json") {
            print_rank_json(&ranked);
        } else {
            print_rank_human(&ranked);
        }
    } else if opts.iter().any(|a| a == "--json") {
        print!("{}", deep_diff_forge_patch::to_json(&files));
    } else if let Some(name) = flag_value(opts, "--layout") {
        if let Some(layout) = deep_diff_forge_projection::layout_from_str(&name) {
            let options = deep_diff_forge_projection::ProjectionOptions {
                layout,
                side_width: deep_diff_forge_projection::DEFAULT_SIDE_WIDTH,
            };
            print!("{}", deep_diff_forge_projection::render(&files, options));
        } else {
            eprintln!("error: unknown layout: {name} (expected inline|side-by-side)");
            std::process::exit(2);
        }
    } else {
        print_patch_summary(&files);
    }
}

/// The current declared maturity level (kept in sync with the deployment
/// framework; bumped as each ladder rung ships).
const CURRENT_MATURITY: deep_diff_forge_core::MaturityLevel =
    deep_diff_forge_core::MaturityLevel::L5;

/// `review [--probe]`: read a patch from stdin and open the review TUI.
///
/// `--probe` renders one frame headlessly (no TTY needed) for CI/agents; bare
/// `review` launches the interactive loop and needs a real terminal.
fn review_cmd(opts: &[String]) {
    use std::io::Read as _;
    let mut input = String::new();
    if let Err(err) = std::io::stdin().read_to_string(&mut input) {
        eprintln!("error: could not read stdin: {err}");
        std::process::exit(3);
    }
    let files = match deep_diff_forge_patch::parse(&input) {
        Ok(files) => files,
        Err(err) => {
            eprintln!("error: patch parse failed: {err}");
            std::process::exit(4);
        }
    };
    let app = deep_diff_forge_tui::ReviewApp::from_review(&files);
    if opts.iter().any(|a| a == "--probe") {
        for line in deep_diff_forge_tui::render_to_lines(&app, 100, 30) {
            println!("{line}");
        }
    } else if let Err(err) = deep_diff_forge_tui::run(app) {
        eprintln!("error: review requires an interactive terminal: {err}");
        std::process::exit(6);
    }
}

/// `semantic <path> [--json]`: parse a source file and report its symbols.
fn semantic_cmd(opts: &[String]) {
    let Some(path) = opts.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("usage: deep-diff-forge semantic <path> [--json]");
        std::process::exit(2);
    };
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("error: could not read {path}: {err}");
            std::process::exit(3);
        }
    };
    let analysis = deep_diff_forge_syntax::analyze(
        path,
        &source,
        deep_diff_forge_syntax::SyntaxOptions::default(),
    );
    if opts.iter().any(|a| a == "--json") {
        print_semantic_json(path, &analysis);
    } else {
        print_semantic_human(path, &analysis);
    }
}

fn parse_status_str(status: &deep_diff_forge_core::ParseStatus) -> String {
    use deep_diff_forge_core::ParseStatus;
    match status {
        ParseStatus::Parsed => "parsed".to_string(),
        ParseStatus::ParsedWithErrors { errors } => format!("parsed_with_errors:{errors}"),
        ParseStatus::Fallback { reason } => format!("fallback:{reason:?}"),
    }
}

fn print_semantic_human(path: &str, analysis: &deep_diff_forge_syntax::SemanticAnalysis) {
    println!(
        "semantic: {path} ({}, {})",
        analysis.language.name(),
        parse_status_str(&analysis.parse_status)
    );
    for sym in &analysis.symbols {
        println!(
            "  {:<9} {}  L{}-L{}",
            sym.kind, sym.name, sym.start_line, sym.end_line
        );
    }
    println!("{} symbol(s)", analysis.symbols.len());
}

fn print_semantic_json(path: &str, analysis: &deep_diff_forge_syntax::SemanticAnalysis) {
    use deep_diff_forge_core::json_escape;
    use std::fmt::Write as _;
    let mut symbols = String::new();
    for (i, sym) in analysis.symbols.iter().enumerate() {
        if i > 0 {
            symbols.push_str(", ");
        }
        let _ = write!(
            symbols,
            "{{\"name\": {}, \"kind\": {}, \"start_line\": {}, \"end_line\": {}}}",
            json_escape(&sym.name),
            json_escape(&sym.kind),
            sym.start_line,
            sym.end_line
        );
    }
    println!(
        "{{\n  \"schema\": \"deep-diff-forge.semantic.v0\",\n  \"path\": {},\n  \"language\": {},\n  \"parse_status\": {},\n  \"symbols\": [{}]\n}}",
        json_escape(path),
        json_escape(analysis.language.name()),
        json_escape(&parse_status_str(&analysis.parse_status)),
        symbols
    );
}

/// Dispatch `deploy` subcommands.
fn deploy_cmd(opts: &[String]) {
    if opts.first().map(String::as_str) == Some("status") {
        deploy_status(opts.iter().any(|a| a == "--json"));
    } else {
        eprintln!("usage: deep-diff-forge deploy status [--json]");
        std::process::exit(2);
    }
}

/// Emit a machine- or human-readable deployment status snapshot.
///
/// Gates are reported `not-run`: this is a status snapshot, not a gate run.
/// Execute the gates with `just gate-feature`.
fn deploy_status(json: bool) {
    use deep_diff_forge_core::{DeploymentStatus, GateState};
    const GATES: [&str; 7] = [
        "identity", "format", "compile", "lint", "test", "fixture", "contract",
    ];
    let mut status = DeploymentStatus::new("deep-diff-forge", CURRENT_MATURITY);
    for gate in GATES {
        status = status.with_gate(gate, GateState::NotRun);
    }

    if json {
        use std::fmt::Write as _;
        let mut gates = String::new();
        for (i, g) in status.gates.iter().enumerate() {
            if i > 0 {
                gates.push_str(", ");
            }
            let _ = write!(
                gates,
                "{{\"name\": \"{}\", \"state\": \"{}\"}}",
                g.name,
                g.state.as_str()
            );
        }
        println!(
            "{{\n  \"schema\": \"deep-diff-forge.deployment-status.v0\",\n  \"repo\": \"{}\",\n  \"maturity\": \"{}\",\n  \"maturity_name\": \"{}\",\n  \"gates\": [{}],\n  \"external_observers\": {{\"zellij\": \"observed\", \"habitat\": \"optional\"}}\n}}",
            status.repo,
            status.maturity.as_str(),
            status.maturity.name(),
            gates
        );
    } else {
        println!("deep-diff-forge deployment status");
        println!("repo:     {}", status.repo);
        println!(
            "maturity: {} ({})",
            status.maturity.as_str(),
            status.maturity.name()
        );
        let names: Vec<&str> = status.gates.iter().map(|g| g.name.as_str()).collect();
        println!(
            "gates:    {} (run via: just gate-feature)",
            names.join(", ")
        );
    }
}

/// Drive `--jsonl` through the real pipeline runner (ingest → render JSONL).
fn run_jsonl_pipeline(input: String) {
    use deep_diff_forge_pipeline::{IngestStage, Pipeline, PipelineData, RenderStage};
    let pipeline = Pipeline::new()
        .with(Box::new(IngestStage))
        .with(Box::new(RenderStage::jsonl()));
    match pipeline.run(PipelineData::Patch(input)) {
        Ok(PipelineData::Rendered(text)) => print!("{text}"),
        Ok(_) => {}
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(4);
        }
    }
}

fn print_rank_human(ranked: &[deep_diff_forge_graph::RankedFile]) {
    for rf in ranked {
        let signals: Vec<&str> = rf.signals.iter().map(|s| s.label()).collect();
        println!(
            "{:>4}  {:<14} {}  [{}]",
            rf.score,
            rf.status.label(),
            rf.path,
            signals.join(",")
        );
    }
    println!("{} file(s) ranked", ranked.len());
}

fn print_rank_json(ranked: &[deep_diff_forge_graph::RankedFile]) {
    use deep_diff_forge_core::json_escape;
    use std::fmt::Write as _;
    let mut items = String::new();
    for (i, rf) in ranked.iter().enumerate() {
        if i > 0 {
            items.push_str(",\n");
        }
        let signals: Vec<String> = rf.signals.iter().map(|s| json_escape(s.label())).collect();
        let _ = write!(
            items,
            "    {{\"path\": {}, \"status\": {}, \"score\": {}, \"signals\": [{}]}}",
            json_escape(&rf.path),
            json_escape(rf.status.label()),
            rf.score,
            signals.join(", ")
        );
    }
    let body = if items.is_empty() {
        String::new()
    } else {
        format!("\n{items}\n  ")
    };
    println!("{{\n  \"schema\": \"deep-diff-forge.rank.v0\",\n  \"ranked\": [{body}]\n}}");
}

/// Return the value following `name` in `opts`, if present.
fn flag_value(opts: &[String], name: &str) -> Option<String> {
    opts.iter()
        .position(|a| a == name)
        .and_then(|i| opts.get(i + 1))
        .cloned()
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
  deep-diff-forge deploy status [--json]
  deep-diff-forge semantic <path> [--json]
  deep-diff-forge review [--probe]
  deep-diff-forge --stdin-patch [--json | --jsonl | --rank | --layout inline|side-by-side]
  deep-diff-forge claude-code-contract
  deep-diff-forge chain-contract
  deep-diff-forge cluster-contract
  deep-diff-forge loom-contract

MATURITY:
  L5 Review. The binary parses unified/Git patches (--stdin-patch), projects
  them (--layout, --json, --jsonl), ranks the review (--rank), extracts
  tree-sitter symbols (semantic <path>), opens the review TUI (review), and
  reports deployment status (deploy status --json). The daemon surface is
  designed but not yet implemented.

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
