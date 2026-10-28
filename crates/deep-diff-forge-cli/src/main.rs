// DDF integration: R02, R09, R19, R21. Validate that a patch transforms the captured base into the captured candidate before attaching semantics; export explicit coverage and snapshot scope.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#shared-service
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-cli--main
mod analysis;
mod args;
mod contract;
mod git_input;
mod mcp;
mod rpc;
use deep_diff_forge_core::exit;
use deep_diff_forge_core::{InputSeal, ReviewDocument, display_safe};

/// Write `s` to stdout, treating a reader that closed the pipe (`… | head`) as
/// a clean exit. Rust ignores SIGPIPE, so a bare `print!`/`println!` panics with
/// `BrokenPipe` (exit 101) the moment the downstream consumer goes away; routing
/// bulk output through here keeps `deep-diff-forge … | head` a normal success.
/// std-only, no `unsafe`.
fn emit(s: &str) {
    use std::io::Write as _;
    if let Err(err) = std::io::stdout().write_all(s.as_bytes()) {
        if err.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(exit::SUCCESS);
        }
        eprintln!("error: write failed: {err}");
        std::process::exit(exit::IO);
    }
}

/// `println!`-shaped wrapper over [`emit`] for broken-pipe-tolerant line output.
macro_rules! emitln {
    () => { emit("\n") };
    ($($arg:tt)*) => { emit(&format!("{}\n", format_args!($($arg)*))) };
}

/// Read all of `reader` into a `String`, but never buffer more than the patch
/// byte budget. `what` names the source for diagnostics.
///
/// Untrusted input (stdin, a source file) is read with a hard `budget + 1` cap,
/// so a pathological or unbounded stream is a graceful `exit 4` rather than a
/// memory-exhaustion denial of service — the process never allocates past the
/// budget. The
/// parser re-checks the budget, but this is the read-side enforcement that
/// closes the gap between "input arrives" and "parser sees it". Exits the
/// process on any failure (3 = read/encoding, 4 = over budget).
fn read_capped_or_exit(reader: impl std::io::Read, what: &str) -> String {
    let buf = read_capped_bytes_or_exit(reader, what);
    let Ok(text) = String::from_utf8(buf) else {
        eprintln!("error: {what} is not valid UTF-8");
        std::process::exit(exit::IO);
    };
    text
}

/// [`read_capped_or_exit`] without the UTF-8 requirement: a patch's non-UTF-8 lines are decoded
/// per file instead (anchor C-12), and its seal covers these exact bytes.
fn read_capped_bytes_or_exit(reader: impl std::io::Read, what: &str) -> Vec<u8> {
    use std::io::Read as _;
    let cap = deep_diff_forge_patch::DEFAULT_BYTE_BUDGET;
    let mut buf = Vec::new();
    // take(cap + 1): one byte past the budget is enough to detect overflow
    // without ever buffering an unbounded amount.
    if let Err(err) = reader.take(cap as u64 + 1).read_to_end(&mut buf) {
        eprintln!("error: could not read {what}: {err}");
        std::process::exit(exit::IO);
    }
    if buf.len() > cap {
        eprintln!("error: {what} exceeds the {cap}-byte input budget");
        std::process::exit(exit::INVALID_INPUT);
    }
    buf
}

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let command = args.next();
    if command
        .as_deref()
        .is_some_and(|c| command_help(c, args.peek()))
    {
        return;
    }
    refuse_arguments_to_bare_commands(command.as_deref(), args.peek());
    match command.as_deref() {
        None | Some("--help" | "-h" | "help") => print_help(),
        Some("--version" | "-V" | "version") => print_version(),
        Some("--self-test" | "self-test") => self_test(),
        Some("doctor") => doctor(),
        Some("deploy") => {
            let rest: Vec<String> = args.collect();
            deploy_cmd(&rest);
        }
        Some("council") => match deep_diff_forge_review::cli::dispatch(&args.collect::<Vec<_>>()) {
            Ok((output, code)) => {
                emit(&output);
                if code != 0 {
                    std::process::exit(code);
                }
            }
            Err(error) => {
                eprintln!("error: {error}");
                std::process::exit(exit::COUNCIL_REFUSED);
            }
        },
        Some("analyze") => {
            let rest: Vec<String> = args.collect();
            analyze_paths_or_exit(&rest);
            match analysis::run(&rest) {
                Ok(json) => emit(&json),
                Err(error) => {
                    eprintln!("error: {error}");
                    std::process::exit(exit::INVALID_INPUT);
                }
            }
        }
        Some("semantic") => {
            let rest: Vec<String> = args.collect();
            semantic_cmd(&rest);
        }
        Some("highlight") => {
            let rest: Vec<String> = args.collect();
            highlight_cmd(&rest);
        }
        Some("structural") => {
            let rest: Vec<String> = args.collect();
            structural_cmd(&rest);
        }
        Some("review") => {
            let rest: Vec<String> = args.collect();
            review_cmd(&rest);
        }
        Some("daemon") => {
            let rest: Vec<String> = args.collect();
            daemon_cmd(&rest);
        }
        Some("learn") => {
            let rest: Vec<String> = args.collect();
            learn_cmd(&rest);
        }
        Some("--stdin-patch") => {
            let rest: Vec<String> = args.collect();
            stdin_patch(&rest);
        }
        Some(selector @ ("--git-working-tree" | "--git-staged" | "--git-range")) => {
            let selector = selector.to_owned();
            let rest: Vec<String> = args.collect();
            git_input::run(&selector, &rest);
        }
        Some("schema") => schema_cmd(&args.collect::<Vec<_>>()),
        Some("explain") => contract_output(contract::explain(&args.collect::<Vec<_>>())),
        Some("envelope") => run_envelope(&args.collect::<Vec<_>>()),
        Some("mcp") => run_mcp(&args.collect::<Vec<_>>()),
        Some("rpc") => run_rpc(&args.collect::<Vec<_>>()),
        Some("claude-code-contract") => claude_code_contract(),
        Some("chain-contract") => chain_contract(),
        Some("cluster-contract") => cluster_contract(),
        Some("loom-contract") => loom_contract(),
        Some(command) => {
            eprintln!("unknown command: {command}");
            eprintln!("run `deep-diff-forge --help` for supported bootstrap commands");
            std::process::exit(exit::USAGE);
        }
    }
}

/// The commands that take no arguments: an extra one is refused, not ignored (anchor A-01).
fn refuse_arguments_to_bare_commands(command: Option<&str>, extra: Option<&String>) {
    if let (
        Some(
            name @ ("--help"
            | "-h"
            | "help"
            | "--version"
            | "-V"
            | "version"
            | "--self-test"
            | "self-test"
            | "doctor"
            | "claude-code-contract"
            | "chain-contract"
            | "cluster-contract"
            | "loom-contract"),
        ),
        Some(extra),
    ) = (command, extra)
    {
        args::admit_or_exit(
            std::slice::from_ref(extra),
            &args::NONE,
            &format!("deep-diff-forge {name}"),
        );
    }
}

/// `schema [result|exit-codes]`: print a machine contract's JSON Schema.
fn schema_cmd(opts: &[String]) {
    args::admit_or_exit(opts, &SCHEMA, "deep-diff-forge schema [result|exit-codes]");
    contract_output(contract::schema(opts));
}

/// The output and guard options shared by `--stdin-patch` and the structured Git selections.
const REVIEW_FLAGS: [&str; 6] = [
    "--json",
    "--jsonl",
    "--rank",
    "--cluster",
    "--require-files",
    "--require-hunks",
];
const STDIN_PATCH: args::Spec = args::Spec {
    flags: &REVIEW_FLAGS,
    valued: &["--layout", "--parallel"],
    positionals: 0,
};
const STDIN_PATCH_USAGE: &str = "deep-diff-forge --stdin-patch [--json | --jsonl | --rank | --cluster [--parallel N] | --layout inline|side-by-side] [--require-files] [--require-hunks]";
const SCHEMA: args::Spec = args::Spec {
    flags: &[],
    valued: &[],
    positionals: 1,
};

/// Exit code for an input-contract refusal: the caller passed `--require-files`
/// or `--require-hunks` and the (well-formed) patch has nothing to review.
/// Distinct from 4 (the input could not be parsed) so a gate can tell "nothing
/// to review" from "garbage". This is the "contract violation" slot of the
/// documented exit-code table.
const EXIT_REFUSED: i32 = exit::INPUT_CONTRACT_REFUSED;

/// The `--require-files` / `--require-hunks` input contracts, parsed from the CLI flags; the
/// predicate itself is [`deep_diff_forge_patch::InputGuards`], shared with the daemon.
#[derive(Clone, Copy)]
struct InputGuards(deep_diff_forge_patch::InputGuards);

impl InputGuards {
    fn from_opts(opts: &[String]) -> Self {
        Self(deep_diff_forge_patch::InputGuards {
            files: opts.iter().any(|a| a == "--require-files"),
            hunks: opts.iter().any(|a| a == "--require-hunks"),
        })
    }

    fn any(self) -> bool {
        self.0.files || self.0.hunks
    }

    /// Exit [`EXIT_REFUSED`] with a one-line stderr reason and an empty stdout on the first
    /// requested contract the input breaks. A gate that feeds a diff into the engine should pass
    /// both, so neither an empty or mis-piped input nor a header-only or rename-only diff can be
    /// mistaken for a clean, zero-risk review.
    fn enforce(self, files: &[deep_diff_forge_core::ReviewFile]) {
        if let Some(reason) = self.0.refusal(files) {
            eprintln!("refused: {reason}");
            std::process::exit(EXIT_REFUSED);
        }
    }
}

/// [`parse_or_exit`] for decoded bytes: a file with non-UTF-8 lines is parsed lossily and marked.
fn parse_decoded_or_exit(
    decoded: &deep_diff_forge_patch::DecodedPatch<'_>,
) -> Vec<deep_diff_forge_core::ReviewFile> {
    match deep_diff_forge_patch::parse_decoded(
        decoded,
        deep_diff_forge_patch::ParseOptions::default(),
    ) {
        Ok(files) => files,
        Err(err) => {
            eprintln!("error: patch parse failed: {err}");
            std::process::exit(exit::INVALID_INPUT);
        }
    }
}

/// Parse a patch or exit 4 with a stderr diagnostic (the CLI contract for a
/// patch parse failure). Shared by every stdin-patch entry point.
fn parse_or_exit(input: &str) -> Vec<deep_diff_forge_core::ReviewFile> {
    match deep_diff_forge_patch::parse(input) {
        Ok(files) => files,
        Err(err) => {
            eprintln!("error: patch parse failed: {err}");
            std::process::exit(exit::INVALID_INPUT);
        }
    }
}

/// Seal the exact input bytes for a machine-readable document. UTF-8
/// validation in [`read_capped_or_exit`] is lossless, so the `String`'s bytes
/// are precisely the bytes read — hashed before any parsing or normalisation.
fn seal_of(input: &[u8]) -> InputSeal {
    InputSeal::of(input, env!("CARGO_PKG_VERSION"))
}

/// Read a unified/Git patch from stdin and emit one of: the
/// `deep-diff-forge.review.v0` JSON document (`--json`), an inline or
/// side-by-side projection (`--layout inline|side-by-side`), or a human review
/// summary (default).
///
/// Exit codes follow the CLI contract: 2 = usage error, 3 = input read failure,
/// 4 = patch parse failure, 7 = `--require-files` / `--require-hunks` refused
/// a patch with nothing to review.
fn stdin_patch(opts: &[String]) {
    git_input::reject_with_stdin(opts);
    args::admit_or_exit(opts, &STDIN_PATCH, STDIN_PATCH_USAGE);
    let bytes = read_capped_bytes_or_exit(std::io::stdin().lock(), "stdin");
    let decoded = deep_diff_forge_patch::decode(&bytes);
    let guards = InputGuards::from_opts(opts);
    // --jsonl streams one event per file through the real pipeline runner.
    if opts.iter().any(|a| a == "--jsonl") {
        if !decoded.is_exact() {
            // The runner parses text and would drop the lossy marks: emit the same events from
            // the marked files instead.
            let files = parse_decoded_or_exit(&decoded);
            guards.enforce(&files);
            emit(&deep_diff_forge_pipeline::jsonl_events(&files));
            return;
        }
        if guards.any() {
            // The pipeline parses internally; the gate needs the counts
            // before anything is streamed, so pre-flight the parse here.
            guards.enforce(&parse_or_exit(decoded.text()));
        }
        run_jsonl_pipeline(decoded.text().to_owned());
        return;
    }

    let review = sealed_review_or_exit(&bytes, guards);
    emit_review(&review, opts);
}

/// The one parse → guard → seal path ([`deep_diff_forge_patch::SealedReview`], anchor C-05), with
/// the CLI's exits: 4 for a parse failure, 7 for a refused contract.
fn sealed_review_or_exit(bytes: &[u8], guards: InputGuards) -> deep_diff_forge_patch::SealedReview {
    use deep_diff_forge_patch::ReviewError;
    match deep_diff_forge_patch::SealedReview::new(bytes, guards.0, env!("CARGO_PKG_VERSION")) {
        Ok(review) => review,
        Err(ReviewError::Parse(error)) => {
            eprintln!("error: patch parse failed: {error}");
            std::process::exit(exit::INVALID_INPUT);
        }
        Err(refused @ ReviewError::Refused(_)) => {
            eprintln!("{refused}");
            std::process::exit(EXIT_REFUSED);
        }
    }
}

/// Emit the selected review output for `review`, whose seal covers the exact patch bytes its
/// files were parsed from and goes into machine-readable documents. Shared by `--stdin-patch`
/// and the structured Git selections.
fn emit_review(review: &deep_diff_forge_patch::SealedReview, opts: &[String]) {
    let (files, seal) = (review.files(), review.seal());
    if opts.iter().any(|a| a == "--cluster") {
        run_cluster(files, opts, seal);
    } else if opts.iter().any(|a| a == "--rank") {
        let ranked = deep_diff_forge_graph::rank(files);
        if opts.iter().any(|a| a == "--json") {
            print_rank_json(&ranked, seal);
        } else {
            print_rank_human(&ranked);
        }
    } else if opts.iter().any(|a| a == "--json") {
        emit(&review.to_json());
    } else if let Some(name) = flag_value(opts, "--layout") {
        if let Some(layout) = deep_diff_forge_projection::layout_from_str(&name) {
            let options = deep_diff_forge_projection::ProjectionOptions {
                layout,
                side_width: deep_diff_forge_projection::DEFAULT_SIDE_WIDTH,
            };
            emit(&deep_diff_forge_projection::render(files, options));
        } else {
            eprintln!(
                "error: unknown layout: {} (expected inline|side-by-side)",
                display_safe(&name)
            );
            std::process::exit(exit::USAGE);
        }
    } else {
        print_patch_summary(files);
    }
}

/// The current declared maturity level (kept in sync with the deployment
/// framework; bumped as each ladder rung ships).
const CURRENT_MATURITY: deep_diff_forge_core::MaturityLevel =
    deep_diff_forge_core::MaturityLevel::L9;

/// `daemon {path|start|health|status|stop} [--socket PATH]`: drive the optional
/// UDS JSON-RPC review daemon.
fn daemon_cmd(opts: &[String]) {
    use deep_diff_forge_daemon::SocketLocation;
    let positionals = args::admit_or_exit(
        opts,
        &args::Spec {
            flags: &["--foreground"],
            valued: &["--socket"],
            positionals: 1,
        },
        "deep-diff-forge daemon {path|start [--foreground]|health|status|stop} [--socket PATH]",
    );
    let sub = positionals.first().copied();
    // Validate the subcommand first, so an unknown one reports usage whatever the environment.
    // `start` always serves in the foreground (a supervisor such as systemd owns the process);
    // `--foreground` says so and belongs to `start` alone.
    if sub != Some("start") && opts.iter().any(|a| a == "--foreground") {
        eprintln!("error: --foreground applies only to `daemon start`");
        eprintln!(
            "usage: deep-diff-forge daemon {{path|start [--foreground]|health|status|stop}} [--socket PATH]"
        );
        std::process::exit(exit::USAGE);
    }
    if !matches!(sub, Some("path" | "start" | "health" | "status" | "stop")) {
        eprintln!(
            "usage: deep-diff-forge daemon {{path|start [--foreground]|health|status|stop}} [--socket PATH]"
        );
        std::process::exit(exit::USAGE);
    }
    // One resolution point: explicit `--socket` override, else $XDG_RUNTIME_DIR,
    // else fail closed (no world-writable /tmp fallback). The result is a
    // valid-by-construction location, so no path `Option` is threaded below.
    let override_path = flag_value(opts, "--socket");
    let Ok(location) = SocketLocation::resolve(override_path.as_deref().map(std::path::Path::new))
    else {
        eprintln!(
            "error: $XDG_RUNTIME_DIR is not set; refusing an insecure world-writable /tmp socket."
        );
        eprintln!("       set XDG_RUNTIME_DIR, or pass an explicit --socket PATH.");
        std::process::exit(exit::USAGE);
    };
    match sub {
        Some("path") => emitln!("{}", location.path().display()),
        Some("start") => {
            if let Err(err) = deep_diff_forge_daemon::run_server(&location) {
                eprintln!("error: daemon failed: {err}");
                std::process::exit(exit::DAEMON_OR_TERMINAL);
            }
        }
        Some("health") => daemon_client(&location, r#"{"id":1,"method":"daemon.health"}"#),
        Some("status") => daemon_client(&location, r#"{"id":1,"method":"daemon.status"}"#),
        Some("stop") => daemon_client(&location, r#"{"id":1,"method":"daemon.shutdown"}"#),
        _ => {
            eprintln!(
                "usage: deep-diff-forge daemon {{path|start [--foreground]|health|status|stop}} [--socket PATH]"
            );
            std::process::exit(exit::USAGE);
        }
    }
}

fn daemon_client(location: &deep_diff_forge_daemon::SocketLocation, line: &str) {
    match deep_diff_forge_daemon::request(location, line) {
        Ok(response) => emitln!("{response}"),
        Err(err) => {
            eprintln!(
                "error: no daemon at {}: {err}",
                display_safe(&location.path().to_string_lossy())
            );
            std::process::exit(exit::DAEMON_OR_TERMINAL);
        }
    }
}

/// `learn {status|record}`: inspect or feed the L9 local-only learning store.
///
/// The learning loop is local-only and fail-soft: a fresh machine with no store
/// reports zero receipts and no trusted default, never an error.
fn learn_cmd(opts: &[String]) {
    let positionals = args::admit_or_exit(
        opts,
        &args::Spec {
            flags: &["--json", "--stdin"],
            valued: &[],
            positionals: 1,
        },
        "deep-diff-forge learn {status|record --stdin} [--json]",
    );
    let json = opts.iter().any(|a| a == "--json");
    match positionals.first().copied() {
        None | Some("status") => learn_status(json),
        Some("record") => learn_record(opts),
        _ => {
            eprintln!("usage: deep-diff-forge learn {{status|record --stdin}} [--json]");
            std::process::exit(exit::USAGE);
        }
    }
}

/// Report the learning state: store location, receipt count, per-strategy
/// scores, and the trusted-default verdict.
fn learn_status(json: bool) {
    use deep_diff_forge_learning::{LearningReport, store};
    let dir = match store::learning_dir() {
        Ok(d) => d,
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(exit::IO);
        }
    };
    let report = match LearningReport::from_dir(&dir) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("error: could not read learning store: {err}");
            std::process::exit(exit::IO);
        }
    };
    let dir_str = dir.display().to_string();
    let trusted = report
        .trusted_default
        .map(deep_diff_forge_learning::Strategy::label);

    if json {
        use deep_diff_forge_core::json_escape;
        use std::fmt::Write as _;
        let mut scores = String::new();
        for (i, s) in report.scores.iter().enumerate() {
            if i > 0 {
                scores.push_str(", ");
            }
            let _ = write!(
                scores,
                "{{\"strategy\": {}, \"samples\": {}, \"acceptance_rate\": {:.3}, \"helpful_rate\": {:.3}, \"fallback_rate\": {:.3}, \"revisit_rate\": {:.3}, \"mean_elapsed_ms\": {:.1}, \"cache_hit_rate\": {:.3}, \"trusted\": {}}}",
                json_escape(s.strategy.label()),
                s.samples,
                s.acceptance_rate,
                s.helpful_rate,
                s.fallback_rate,
                s.revisit_rate,
                s.mean_elapsed_ms,
                s.cache_hit_rate,
                s.earns_trust(&report.policy)
            );
        }
        let trusted_json = trusted.map_or_else(|| "null".to_string(), json_escape);
        emit(&format!(
            "{{\n  \"schema\": \"deep-diff-forge.learning.v0\",\n  \"store\": {},\n  \"total_receipts\": {},\n  \"trusted_default\": {},\n  \"scores\": [{}]\n}}\n",
            json_escape(&dir_str),
            report.total_receipts,
            trusted_json,
            scores
        ));
    } else {
        emitln!("deep-diff-forge learning (L9)");
        emitln!("store:    {dir_str}");
        emitln!("receipts: {}", report.total_receipts);
        emitln!(
            "trusted:  {}",
            trusted.unwrap_or("none (insufficient evidence)")
        );
        for s in &report.scores {
            emitln!(
                "  {:<7} n={:<4} accept={:.2} helpful={:.2} fallback={:.2} revisit={:.2} {:>6.1}ms  {}",
                s.strategy.label(),
                s.samples,
                s.acceptance_rate,
                s.helpful_rate,
                s.fallback_rate,
                s.revisit_rate,
                s.mean_elapsed_ms,
                if s.earns_trust(&report.policy) {
                    "[trusted]"
                } else {
                    ""
                }
            );
        }
    }
}

/// `learn record --stdin`: read one JSON
/// [`StrategyReceipt`](deep_diff_forge_learning::StrategyReceipt) from stdin and
/// append it to the local store. The agent/automation entry point for feeding
/// the loop; rejects malformed input rather than silently dropping it.
fn learn_record(opts: &[String]) {
    use deep_diff_forge_learning::{LearningError as E, StrategyReceipt, record_receipt};
    if !opts.iter().any(|a| a == "--stdin") {
        eprintln!("usage: deep-diff-forge learn record --stdin   (reads one JSON receipt)");
        std::process::exit(exit::USAGE);
    }
    let input = read_capped_or_exit(std::io::stdin().lock(), "stdin");
    let receipt = match StrategyReceipt::from_json(input.trim()) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("error: invalid receipt: {err}");
            std::process::exit(exit::INVALID_INPUT);
        }
    };
    if let Err(err) = record_receipt(&receipt) {
        eprintln!("error: could not record receipt: {err}");
        // A refused receipt is the input's fault and repeats on retry; only the store is I/O.
        // Exhaustive, so a new error kind has to choose its exit code.
        let code = match err {
            E::InvalidReceipt(_) => exit::INVALID_INPUT,
            E::NoStateDir | E::Io(_) | E::Serialize(_) | E::Deserialize(_) => exit::IO,
        };
        std::process::exit(code);
    }
    emitln!(
        "recorded: {} receipt for {} ({})",
        receipt.strategy.label(),
        receipt.language,
        receipt.outcome.label()
    );
}

/// `review [--probe] [--side] [--palette | --cmd NAME]`: read a patch from stdin
/// and open the review TUI.
///
/// `--probe` renders one frame headlessly (no TTY needed) for CI/agents; bare
/// `review` launches the interactive loop and needs a real terminal. `--side`
/// opens in the two-column side-by-side layout (default is inline). `--palette`
/// opens the command palette; `--cmd NAME` runs a palette command (rank,
/// outline, cluster, summary, notes, review, daemon, learning, maturity) and
/// shows its panel.
fn review_cmd(opts: &[String]) {
    if opts
        .iter()
        .any(|a| matches!(a.as_str(), "--help" | "-h" | "help"))
    {
        print_review_help();
        return;
    }
    args::admit_or_exit(
        opts,
        &args::Spec {
            flags: &["--probe", "--side", "--palette"],
            valued: &["--cmd", "--cols", "--rows"],
            positionals: 0,
        },
        "deep-diff-forge review [--probe [--cols N] [--rows N]] [--side] [--palette | --cmd NAME]",
    );
    // An unknown command name or a size that is not a number is refused before stdin is read.
    // Before, the name fell through to `rank` and the size to its default, both with exit 0.
    let command = flag_value(opts, "--cmd").map(|name| palette_index_or_exit(&name));
    let cols = probe_size_or_exit(opts, "--cols", 100).max(40);
    let rows = probe_size_or_exit(opts, "--rows", 30).max(8);

    let input = read_capped_or_exit(std::io::stdin().lock(), "stdin");
    let files = parse_or_exit(&input);
    // Surface the engine's own findings (risk signals, semantic changes) as
    // grounded, anchored inline notes so the review explains *why* a file
    // matters in-place.
    let ranked = deep_diff_forge_graph::rank(&files);
    let notes = deep_diff_forge_tui::engine_annotations(&files, &ranked);
    let mut app = deep_diff_forge_tui::ReviewApp::from_review_with_annotations(&files, notes);
    // `--side` opens directly in the two-column layout (default is inline).
    if opts.iter().any(|a| a == "--side") {
        app.handle(deep_diff_forge_tui::AppEvent::ToggleLayout);
    }
    // `--cmd <name>` runs a palette command and shows its panel; `--palette`
    // opens the palette. Both give headless access to the in-cockpit commands.
    if let Some(index) = command {
        open_palette_command(&mut app, index);
    } else if opts.iter().any(|a| a == "--palette") {
        app.handle(deep_diff_forge_tui::AppEvent::OpenPalette);
    }
    if opts.iter().any(|a| a == "--probe") {
        // Probe size is configurable so callers can render a roomy frame; the
        // floor keeps the layout from degenerating on absurd inputs.
        for line in deep_diff_forge_tui::render_to_lines(&app, cols, rows) {
            emitln!("{line}");
        }
    } else if let Err(err) = deep_diff_forge_tui::run(app) {
        eprintln!("error: review requires an interactive terminal: {err}");
        std::process::exit(exit::DAEMON_OR_TERMINAL);
    }
}

fn print_review_help() {
    emit(
        "deep-diff-forge review

USAGE:
  deep-diff-forge review [--probe [--cols N] [--rows N]] [--side] [--palette | --cmd NAME]

Reads a unified/Git patch from stdin and opens the review TUI.

OPTIONS:
  --probe        Render one headless frame; does not require a TTY
  --cols N       Probe width, minimum 40 columns
  --rows N       Probe height, minimum 8 rows
  --side         Start in side-by-side layout
  --palette      Open the command palette before rendering/running
  --cmd NAME     Run a palette command and show its result panel
  -h, --help     Show this help

COMMAND NAMES:
  rank, outline, cluster, summary, notes, review, daemon, learning, maturity

",
    );
}

/// The palette position of the command whose label matches `name` (by full label or first
/// word), or a usage error naming the commands there are.
fn palette_index_or_exit(name: &str) -> usize {
    let commands = deep_diff_forge_tui::Command::all();
    commands
        .iter()
        .position(|c| c.label() == name || c.label().split(' ').next() == Some(name))
        .unwrap_or_else(|| {
            let names: Vec<&str> = commands
                .iter()
                .filter_map(|c| c.label().split(' ').next())
                .collect();
            eprintln!(
                "error: unknown review command {}; expected one of: {}",
                display_safe(name),
                names.join(", ")
            );
            std::process::exit(exit::USAGE);
        })
}

/// The value of a probe size flag, `default` when it is absent, or a usage error when it is
/// not a number from 0 to 65535.
fn probe_size_or_exit(opts: &[String], flag: &str, default: u16) -> u16 {
    flag_value(opts, flag).map_or(default, |value| {
        value.parse().unwrap_or_else(|_| {
            eprintln!(
                "error: {flag} takes a number of terminal cells, not {}",
                display_safe(&value)
            );
            std::process::exit(exit::USAGE);
        })
    })
}

/// Open the palette, move to the command at `index`, and run it — leaving the app on that
/// command's panel.
fn open_palette_command(app: &mut deep_diff_forge_tui::ReviewApp, index: usize) {
    use deep_diff_forge_tui::AppEvent;
    app.handle(AppEvent::OpenPalette);
    for _ in 0..index {
        app.handle(AppEvent::Next);
    }
    app.handle(AppEvent::Select);
}

/// Open and read a source file under the byte cap, exiting on failure. The
/// shared helper behind the file-reading subcommands.
fn read_source_file_or_exit(path: &str) -> String {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("error: could not read {}: {err}", display_safe(path));
            std::process::exit(exit::IO);
        }
    };
    read_capped_or_exit(file, "source file")
}

/// `structural <old> <new> [--json]`: token-level (AST-leaf) structural diff —
/// reformat-aware, never touching patch truth.
fn structural_cmd(opts: &[String]) {
    let paths = args::admit_or_exit(
        opts,
        &args::Spec {
            flags: &["--json"],
            valued: &[],
            positionals: 2,
        },
        "deep-diff-forge structural <old> <new> [--json]",
    );
    let (Some(old_path), Some(new_path)) = (paths.first(), paths.get(1)) else {
        eprintln!("usage: deep-diff-forge structural <old> <new> [--json]");
        std::process::exit(exit::USAGE);
    };
    let old_source = read_source_file_or_exit(old_path);
    let new_source = read_source_file_or_exit(new_path);
    let language = deep_diff_forge_syntax::detect_language(new_path);
    // An unsupported language tokenizes to nothing on both sides, which would
    // otherwise read as a misleading "no token changes (formatting only)". Be
    // explicit instead: structural diffing is Rust-only today.
    if language == deep_diff_forge_syntax::Language::Unsupported {
        eprintln!(
            "error: structural diff is unsupported for {} — only Rust is tokenized today",
            display_safe(new_path)
        );
        std::process::exit(exit::USAGE);
    }
    let diff = deep_diff_forge_syntax::structural_diff(language, &old_source, &new_source);
    if opts.iter().any(|a| a == "--json") {
        print_structural_json(&diff);
    } else {
        print_structural_human(&diff);
    }
}

fn print_structural_human(diff: &deep_diff_forge_syntax::StructuralDiff) {
    use deep_diff_forge_syntax::ChangeKind;
    if let Some(reason) = diff.unavailable() {
        emitln!(
            "structural: unavailable ({}); use patch review",
            reason.wire_name()
        );
        return;
    }
    if diff.reformat_only() {
        emitln!("structural: no token changes (formatting only)");
    }
    for change in &diff.changes {
        if change.kind == ChangeKind::Unchanged {
            continue;
        }
        let marker = match change.kind {
            ChangeKind::Added => '+',
            ChangeKind::Removed => '-',
            ChangeKind::Moved => '~',
            ChangeKind::Unchanged => ' ',
        };
        let line = change.new_line.or(change.old_line).unwrap_or(0);
        emitln!(
            "{marker} {:<10} L{line}  {}",
            change.kind.label(),
            display_safe(&change.text)
        );
    }
    emitln!(
        "summary: {} unchanged, {} added, {} removed, {} moved{}",
        diff.unchanged,
        diff.added,
        diff.removed,
        diff.moved,
        if diff.degraded() {
            " (degraded: input too large for token LCS)"
        } else {
            ""
        }
    );
}

fn print_structural_json(diff: &deep_diff_forge_syntax::StructuralDiff) {
    use deep_diff_forge_core::json_escape;
    use deep_diff_forge_syntax::ChangeKind;
    use std::fmt::Write as _;
    let num = |v: Option<u32>| v.map_or_else(|| "null".to_string(), |n| n.to_string());
    let mut changes = String::new();
    for (i, c) in diff
        .changes
        .iter()
        .filter(|c| c.kind != ChangeKind::Unchanged)
        .enumerate()
    {
        if i > 0 {
            changes.push_str(",\n");
        }
        let _ = write!(
            changes,
            "    {{\"kind\": {}, \"text\": {}, \"old_line\": {}, \"new_line\": {}}}",
            json_escape(c.kind.label()),
            json_escape(&c.text),
            num(c.old_line),
            num(c.new_line)
        );
    }
    let body = if changes.is_empty() {
        String::new()
    } else {
        format!("\n{changes}\n  ")
    };
    let lang = if diff.language == deep_diff_forge_syntax::Language::Rust {
        "rust"
    } else {
        "unsupported"
    };
    emitln!(
        "{{\n  \"schema\": \"deep-diff-forge.structural.v0\",\n  \"language\": {},\n  \"reformat_only\": {},\n  \"degraded\": {},\n  \"unavailable\": {},\n  \"summary\": {{\"unchanged\": {}, \"added\": {}, \"removed\": {}, \"moved\": {}}},\n  \"changes\": [{}]\n}}",
        json_escape(lang),
        diff.reformat_only(),
        diff.degraded(),
        diff.unavailable()
            .map_or_else(|| "null".into(), |reason| json_escape(reason.wire_name())),
        diff.unchanged,
        diff.added,
        diff.removed,
        diff.moved,
        body
    );
}

/// `highlight <path> [--color|--no-color]`: print the file with tree-sitter
/// syntax highlighting.
///
/// Colour is on when stdout is a terminal (or `--color` is forced) and off when
/// piped (or `--no-color`). Either way the output is terminal-safe: the source is
/// routed through the same control-char sanitiser, so a hostile file cannot
/// inject escapes — only fixed SGR colour codes are emitted.
fn highlight_cmd(opts: &[String]) {
    use std::io::IsTerminal as _;
    let positionals = args::admit_or_exit(
        opts,
        &args::Spec {
            flags: &["--color", "--no-color"],
            valued: &[],
            positionals: 1,
        },
        "deep-diff-forge highlight <path> [--color|--no-color]",
    );
    let Some(path) = positionals.first().copied() else {
        eprintln!("usage: deep-diff-forge highlight <path> [--color|--no-color]");
        std::process::exit(exit::USAGE);
    };
    let source = read_source_file_or_exit(path);
    let language = deep_diff_forge_syntax::detect_language(path);

    let color = if opts.iter().any(|a| a == "--no-color") {
        false
    } else if opts.iter().any(|a| a == "--color") {
        true
    } else {
        std::io::stdout().is_terminal()
    };

    if color {
        emit(&deep_diff_forge_syntax::highlight_to_ansi(
            language, &source,
        ));
    } else {
        // No colour requested/available: still sanitise so a hostile file is safe.
        emit(&display_safe(&source));
    }
}

/// `semantic <path> [--json]`: parse a source file and report its symbols.
fn semantic_cmd(opts: &[String]) {
    let positionals = args::admit_or_exit(
        opts,
        &args::Spec {
            flags: &["--json"],
            valued: &[],
            positionals: 1,
        },
        "deep-diff-forge semantic <path> [--json]",
    );
    let Some(path) = positionals.first().copied() else {
        eprintln!("usage: deep-diff-forge semantic <path> [--json]");
        std::process::exit(exit::USAGE);
    };
    let source = read_source_file_or_exit(path);
    let analysis = deep_diff_forge_syntax::analyze(
        path,
        &source,
        deep_diff_forge_syntax::SyntaxOptions::default(),
    );
    if opts.iter().any(|a| a == "--json") {
        // For `semantic` the sealed input is the source file's exact bytes.
        print_semantic_json(path, &analysis, &seal_of(source.as_bytes()));
    } else {
        print_semantic_human(path, &analysis);
    }
}

fn parse_status_str(status: &deep_diff_forge_core::ParseStatus) -> String {
    use deep_diff_forge_core::ParseStatus;
    match status {
        ParseStatus::Parsed => "parsed".to_string(),
        ParseStatus::ParsedWithErrors { errors } => format!("parsed_with_errors:{errors}"),
        ParseStatus::Fallback { reason } => format!("fallback:{}", reason.wire_name()),
    }
}

fn print_semantic_human(path: &str, analysis: &deep_diff_forge_syntax::SemanticAnalysis) {
    emitln!(
        "semantic: {} ({}, {})",
        display_safe(path),
        analysis.language.name(),
        parse_status_str(&analysis.parse_status)
    );
    for sym in &analysis.symbols {
        // Symbol names come from parsing attacker-controlled source — sanitize.
        emitln!(
            "  {:<9} {}  L{}-L{}",
            sym.kind,
            display_safe(&sym.name),
            sym.start_line,
            sym.end_line
        );
    }
    emitln!("{} symbol(s)", analysis.symbols.len());
}

fn print_semantic_json(
    path: &str,
    analysis: &deep_diff_forge_syntax::SemanticAnalysis,
    seal: &InputSeal,
) {
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
    emitln!(
        "{{\n  \"schema\": \"deep-diff-forge.semantic.v0\",\n{}  \"path\": {},\n  \"language\": {},\n  \"parse_status\": {},\n  \"symbols\": [{}]\n}}",
        seal.json_members(),
        json_escape(path),
        json_escape(analysis.language.name()),
        json_escape(&parse_status_str(&analysis.parse_status)),
        symbols
    );
}

/// Dispatch `deploy` subcommands.
fn deploy_cmd(opts: &[String]) {
    let positionals = args::admit_or_exit(
        opts,
        &args::Spec {
            flags: &["--json"],
            valued: &[],
            positionals: 1,
        },
        "deep-diff-forge deploy {status|release} [--json]",
    );
    let json = opts.iter().any(|a| a == "--json");
    match positionals.first().copied() {
        Some("status") => deploy_status(json),
        Some("release") => deploy_release(json),
        _ => {
            eprintln!("usage: deep-diff-forge deploy {{status|release}} [--json]");
            std::process::exit(exit::USAGE);
        }
    }
}

/// Report the release publication posture for the current version.
///
/// This is a declared snapshot (like `deploy status`): the actual release acts
/// are `git tag`, `gh release`, and `cargo publish`. The source is on both remotes,
/// so `github` and `gitlab` are `published`. `github-release` is `blocked`: a build
/// cannot know whether its own version was tagged, and a release exists only once the
/// owner pushes a `v*` tag. crates.io is `blocked`: publication is on hold by owner
/// decision, and release.yml publishes only when the repository variable
/// `DDF_CRATES_IO` is `open`.
fn deploy_release(json: bool) {
    use deep_diff_forge_core::{ReleasePlan, TargetState};
    let plan = ReleasePlan::new(env!("CARGO_PKG_VERSION"))
        .with_target("github", TargetState::Published)
        .with_target("gitlab", TargetState::Published)
        .with_target("github-release", TargetState::Blocked)
        .with_target("crates.io", TargetState::Blocked);

    if json {
        use deep_diff_forge_core::json_escape;
        use std::fmt::Write as _;
        let mut targets = String::new();
        for (i, t) in plan.targets.iter().enumerate() {
            if i > 0 {
                targets.push_str(", ");
            }
            let _ = write!(
                targets,
                "{{\"name\": {}, \"state\": {}}}",
                json_escape(&t.name),
                json_escape(t.state.as_str())
            );
        }
        let pending: Vec<String> = plan.pending().iter().map(|p| json_escape(p)).collect();
        emitln!(
            "{{\n  \"schema\": \"deep-diff-forge.release.v0\",\n  \"version\": {},\n  \"fully_published\": {},\n  \"targets\": [{}],\n  \"pending\": [{}]\n}}",
            json_escape(&plan.version),
            plan.fully_published(),
            targets,
            pending.join(", ")
        );
    } else {
        emitln!("deep-diff-forge release v{}", plan.version);
        for t in &plan.targets {
            emitln!("  {:<16} {}", t.name, t.state.as_str());
        }
        let pending = plan.pending();
        if pending.is_empty() {
            emitln!("fully published");
        } else {
            emitln!("pending: {}", pending.join(", "));
        }
    }
}

/// Emit a machine- or human-readable deployment status snapshot.
///
/// Gates are reported `not-run`: this is a static snapshot, not a gate run, and the JSON says so
/// with `"gates_run": false` (anchor E-06). Execute the gates with `just ci`.
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
        emitln!(
            "{{\n  \"schema\": \"deep-diff-forge.deployment-status.v0\",\n  \"repo\": \"{}\",\n  \"maturity\": \"{}\",\n  \"maturity_name\": \"{}\",\n  \"gates_run\": false,\n  \"gates\": [{}],\n  \"external_observers\": {{\"zellij\": \"observed\", \"habitat\": \"optional\"}}\n}}",
            status.repo,
            status.maturity.as_str(),
            status.maturity.name(),
            gates
        );
    } else {
        emitln!("deep-diff-forge deployment status");
        emitln!("repo:     {}", status.repo);
        emitln!(
            "maturity: {} ({})",
            status.maturity.as_str(),
            status.maturity.name()
        );
        let names: Vec<&str> = status.gates.iter().map(|g| g.name.as_str()).collect();
        emitln!(
            "gates:    {} (not run by this command; run them with: just ci)",
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
        Ok(PipelineData::Rendered(text)) => emit(&text),
        Ok(_) => {}
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(exit::INVALID_INPUT);
        }
    }
}

/// Exit 2 unless `analyze` was given exactly its three paths and no options. Before, a missing
/// path exited 4, which tells a caller to change the input instead of the command line.
fn analyze_paths_or_exit(rest: &[String]) {
    const USAGE: &str =
        "deep-diff-forge analyze <patch-file> <base-directory> <candidate-directory>";
    let spec = args::Spec {
        flags: &[],
        valued: &[],
        positionals: 3,
    };
    let given = args::admit_or_exit(rest, &spec, USAGE).len();
    if given != 3 {
        eprintln!("error: analyze takes 3 paths, got {given}");
        eprintln!("usage: {USAGE}");
        std::process::exit(exit::USAGE);
    }
}

/// Parse `--parallel serial|auto|<n>` into a `Parallelism` (default Auto); anything else is a
/// usage error. Before, an unreadable count quietly became `auto`, and 0 was reported as
/// `fixed:0` while one worker ran.
fn parse_parallelism(opts: &[String]) -> deep_diff_forge_core::Parallelism {
    use deep_diff_forge_core::Parallelism;
    match flag_value(opts, "--parallel").as_deref() {
        Some("serial") => Parallelism::Serial,
        Some("auto") | None => Parallelism::Auto,
        Some(n) => match n.parse::<u16>() {
            Ok(workers) if workers > 0 => Parallelism::Fixed(workers),
            _ => {
                eprintln!(
                    "error: --parallel takes serial, auto or a worker count from 1, not {}",
                    display_safe(n)
                );
                std::process::exit(exit::USAGE);
            }
        },
    }
}

/// Run the patch+risk cluster with bounded parallelism and a deterministic join.
/// `input` is the raw patch text, sealed into the `--json` document.
fn run_cluster(files: &[deep_diff_forge_core::ReviewFile], opts: &[String], seal: &InputSeal) {
    use deep_diff_forge_cluster::{join_label, parallelism_label, run_risk_cluster};
    use deep_diff_forge_core::JoinPolicy;
    let parallelism = parse_parallelism(opts);
    let run = run_risk_cluster(files, parallelism, JoinPolicy::RankedReviewOrder);
    if opts.iter().any(|a| a == "--json") {
        print_cluster_json(&run, seal);
    } else {
        print_rank_human(&run.ranked);
        emitln!(
            "cluster: {} dimension(s), parallelism={}, workers={}, join={}",
            run.receipt.dimensions.len(),
            parallelism_label(run.receipt.parallelism),
            run.receipt.worker_count,
            join_label(run.receipt.join_policy)
        );
    }
}

fn print_cluster_json(run: &deep_diff_forge_cluster::ClusterRun, seal: &InputSeal) {
    use deep_diff_forge_cluster::{dimension_label, join_label, parallelism_label};
    use deep_diff_forge_core::json_escape;
    use std::fmt::Write as _;
    let dims: Vec<String> = run
        .receipt
        .dimensions
        .iter()
        .map(|d| json_escape(dimension_label(*d)))
        .collect();
    let mut ranked = String::new();
    for (i, rf) in run.ranked.iter().enumerate() {
        if i > 0 {
            ranked.push_str(",\n");
        }
        let signals: Vec<String> = rf.signals.iter().map(|s| json_escape(s.label())).collect();
        let _ = write!(
            ranked,
            "    {{\"path\": {}, \"status\": {}, \"score\": {}, \"signals\": [{}]}}",
            json_escape(&rf.path),
            json_escape(rf.status.label()),
            rf.score,
            signals.join(", ")
        );
    }
    let body = if ranked.is_empty() {
        String::new()
    } else {
        format!("\n{ranked}\n  ")
    };
    emitln!(
        "{{\n  \"schema\": \"deep-diff-forge.cluster.v0\",\n{}  \"receipt\": {{\"dimensions\": [{}], \"parallelism\": {}, \"workers\": {}, \"join_policy\": {}, \"file_count\": {}}},\n  \"ranked\": [{}]\n}}",
        seal.json_members(),
        dims.join(", "),
        json_escape(&parallelism_label(run.receipt.parallelism)),
        run.receipt.worker_count,
        json_escape(join_label(run.receipt.join_policy)),
        run.receipt.file_count,
        body
    );
}

fn print_rank_human(ranked: &[deep_diff_forge_graph::RankedFile]) {
    for rf in ranked {
        let signals: Vec<&str> = rf.signals.iter().map(|s| s.label()).collect();
        emitln!(
            "{:>4}  {:<14} {}  [{}]",
            rf.score,
            rf.status.label(),
            display_safe(&rf.path),
            signals.join(",")
        );
    }
    emitln!("{} file(s) ranked", ranked.len());
}

fn print_rank_json(ranked: &[deep_diff_forge_graph::RankedFile], seal: &InputSeal) {
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
    emitln!(
        "{{\n  \"schema\": \"deep-diff-forge.rank.v0\",\n{}  \"ranked\": [{body}]\n}}",
        seal.json_members()
    );
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
        let status = file.status.label();
        let lossy = match file.patch_twin.lossy_lines {
            0 => String::new(),
            n => format!("  [{n} non-UTF-8 line(s) shown lossily]"),
        };
        emitln!(
            "{status:>14}  {}  (+{adds} -{dels}, {hunks} hunks){lossy}",
            display_safe(&file.path)
        );
    }
    emitln!("{} file(s) changed", files.len());
}

/// Every command: the name `<name> --help` answers for, its usage line, and what it does. The one
/// source of `--help`'s USAGE section and of each command's own `--help`, so the two cannot drift.
/// Before, `--help` worked only on `review` and `council help`; the other commands exited 2 on it.
const COMMANDS: &[(&str, &str, &str)] = &[
    ("--help", "deep-diff-forge --help", "Print every command."),
    (
        "--version",
        "deep-diff-forge --version",
        "Print the version.",
    ),
    (
        "--self-test",
        "deep-diff-forge --self-test",
        "Check the core model.",
    ),
    (
        "doctor",
        "deep-diff-forge doctor",
        "Print the runtime, cache, state and socket paths.",
    ),
    (
        "deploy",
        "deep-diff-forge deploy {status|release} [--json]",
        "Report the declared deployment state, or the per-target release posture.",
    ),
    (
        "semantic",
        "deep-diff-forge semantic <path> [--json]",
        "List a source file's top-level symbols with tree-sitter (Rust today).",
    ),
    (
        "highlight",
        "deep-diff-forge highlight <path> [--color|--no-color]",
        "Print a source file with terminal-safe syntax highlighting.",
    ),
    (
        "council",
        "deep-diff-forge council <example|prepare|review|gate|status|import|export-tuicr|import-tuicr>",
        "Run the review council; `council help` has the full usage.",
    ),
    (
        "analyze",
        "deep-diff-forge analyze <patch-file> <base-directory> <candidate-directory>",
        "Check that a patch turns the base into the candidate, then emit deep-diff-forge.analysis.v1.",
    ),
    (
        "structural",
        "deep-diff-forge structural <old> <new> [--json]",
        "Diff two source files by tokens; formatting-only changes report zero change.",
    ),
    (
        "review",
        "deep-diff-forge review [--probe [--cols N] [--rows N]] [--side] [--palette | --cmd NAME]",
        "Open the review cockpit on a patch from stdin.",
    ),
    (
        "daemon",
        "deep-diff-forge daemon {path|start [--foreground]|health|status|stop} [--socket PATH]",
        "Run or query the optional Unix-socket JSON-RPC daemon.",
    ),
    (
        "learn",
        "deep-diff-forge learn {status|record --stdin} [--json]",
        "Report or feed the local-only learning loop.",
    ),
    (
        "--stdin-patch",
        "deep-diff-forge --stdin-patch [--json | --jsonl | --rank | --cluster [--parallel N] | --layout inline|side-by-side] [--require-files] [--require-hunks]",
        "Review a unified or Git patch read from stdin.",
    ),
    (
        "--git-working-tree",
        "deep-diff-forge {--git-working-tree [--include-untracked] | --git-staged | --git-range A..B} [--repo PATH] [--acquisition-json | <--stdin-patch output flags>]",
        "Review a change Git returns, sealed over the exact bytes.",
    ),
    (
        "mcp",
        "deep-diff-forge mcp",
        "Serve the read-only MCP tools over stdio.",
    ),
    (
        "rpc",
        "deep-diff-forge rpc",
        "Speak the daemon's JSON-RPC over stdio, with no daemon.",
    ),
    (
        "envelope",
        "deep-diff-forge envelope -- <command> [args…]",
        "Run a command and print its result as one ddf.result.v1 object.",
    ),
    (
        "schema",
        "deep-diff-forge schema [result|exit-codes]",
        "Print the envelope's JSON Schema, or the exit-code registry.",
    ),
    (
        "explain",
        "deep-diff-forge explain <code> [--family engine|council] [--json]",
        "Name one exit code and the action it asks for.",
    ),
    (
        "claude-code-contract",
        "deep-diff-forge claude-code-contract",
        "Print the agent-facing output guarantees.",
    ),
    (
        "chain-contract",
        "deep-diff-forge chain-contract",
        "Print the Unix-filter chaining guarantees.",
    ),
    (
        "cluster-contract",
        "deep-diff-forge cluster-contract",
        "Print the parallel execution guarantees.",
    ),
    (
        "loom-contract",
        "deep-diff-forge loom-contract",
        "Print the assimilation-pipeline guarantees.",
    ),
];

/// `<command> --help` (or `-h`) as the first argument: print that command's usage and what it
/// does, and exit 0. `review` prints its own longer help; `council --help` is `council help`.
fn command_help(command: &str, first: Option<&String>) -> bool {
    let asked = first.is_some_and(|a| matches!(a.as_str(), "--help" | "-h"));
    let name = match command {
        "--git-staged" | "--git-range" => "--git-working-tree",
        other => other,
    };
    let Some((_, line, what)) = COMMANDS
        .iter()
        .find(|(n, _, _)| *n == name && !matches!(*n, "review" | "council" | "--help"))
    else {
        return false;
    };
    if asked {
        emitln!("{what}\n\nUSAGE:\n  {line}\n\nEvery command: deep-diff-forge --help");
    }
    asked
}

fn print_help() {
    use std::fmt::Write as _;
    let mut usage = String::new();
    for (_, line, _) in COMMANDS {
        let _ = writeln!(usage, "  {line}");
    }
    emitln!(
        "\
deep-diff-forge {version}

USAGE:
{usage}
MATURITY:
  L9 Learning. All engine layers L0-L8 are implemented; the L9 learning loop
  records local-only strategy receipts (learn status|record) and scores them,
  reporting whether a learned default would be trusted; no engine default reads
  the scores yet. It never uploads source and never mutates patch truth.
  Tagged releases are cut to GitHub (binary + checksums) and both git remotes
  (deploy release --json reports the per-target posture).
  crates.io publication is on hold until the codebase is production-ready; a
  configured registry token does not lift the hold.

MACHINE CONTRACT:
  envelope -- <command…> wraps any command's result in one ddf.result.v1 JSON
  object (exit code, registry outcome, stdout, stderr) and exits with its code.
  schema prints the envelope's JSON Schema or the exit-code registry; explain
  names one exit code. mcp serves read-only MCP tools over stdio (review_patch,
  rank_patch, explain_exit_code, exit_code_registry). rpc speaks the daemon's
  JSON-RPC protocol over stdio without a daemon.

NOT YET IMPLEMENTED (each is refused today):
  deep-diff-forge <old> <new>
  deep-diff-forge chain
  deep-diff-forge cluster
  deep-diff-forge loom
",
        version = env!("CARGO_PKG_VERSION")
    );
}

/// `envelope -- <command…>`: print the `ddf.result.v1` object and leave with the command's code.
fn run_envelope(args: &[String]) {
    match contract::envelope(args) {
        Ok((json, code)) => {
            // Unlike `emit`, a closed reader keeps the command's own exit code (council sa1).
            use std::io::Write as _;
            if let Err(err) = std::io::stdout().write_all(json.as_bytes())
                && err.kind() != std::io::ErrorKind::BrokenPipe
            {
                eprintln!("error: write failed: {err}");
                std::process::exit(exit::IO);
            }
            std::process::exit(code);
        }
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(exit::USAGE);
        }
    }
}

/// `rpc`: the daemon's JSON-RPC over stdio, in-process.
fn run_rpc(args: &[String]) {
    if !args.is_empty() {
        eprintln!("error: rpc takes no arguments (it serves the daemon protocol over stdio)");
        std::process::exit(exit::USAGE);
    }
    match rpc::serve(&mut std::io::stdin().lock(), &mut std::io::stdout().lock()) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {}
        Err(err) => {
            eprintln!("error: rpc transport failed: {err}");
            std::process::exit(exit::IO);
        }
    }
}

/// `mcp`: serve the read-only MCP tools over stdio until stdin closes.
fn run_mcp(args: &[String]) {
    if !args.is_empty() {
        eprintln!("error: mcp takes no arguments (it serves MCP over stdio)");
        std::process::exit(exit::USAGE);
    }
    match mcp::serve(&mut std::io::stdin().lock(), &mut std::io::stdout().lock()) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {}
        Err(err) => {
            eprintln!("error: mcp transport failed: {err}");
            std::process::exit(exit::IO);
        }
    }
}

/// Print a machine-contract answer, or its usage error with exit 2.
fn contract_output(result: Result<String, String>) {
    match result {
        Ok(text) => emit(&text),
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(exit::USAGE);
        }
    }
}

fn print_version() {
    emitln!("deep-diff-forge {}", env!("CARGO_PKG_VERSION"));
}

fn self_test() {
    let document = ReviewDocument::empty();
    assert_eq!(document.files.len(), 0);
    emitln!("self-test ok: core model initialized");
}

#[expect(
    clippy::disallowed_methods,
    reason = "the doctor door: reports the XDG cache and state locations"
)]
fn doctor() {
    let cache_dir = std::env::var("XDG_CACHE_HOME").map_or_else(
        |_| "~/.cache/deep-diff-forge".into(),
        |base| format!("{base}/deep-diff-forge"),
    );
    let state_dir = std::env::var("XDG_STATE_HOME").map_or_else(
        |_| "~/.local/state/deep-diff-forge".into(),
        |base| format!("{base}/deep-diff-forge"),
    );

    emitln!("doctor ok: bootstrap binary is executable");
    // Report the runtime dir + daemon socket honestly AND consistently: both go
    // through the daemon's own `var_os`-based resolvers (the single source of
    // truth), so doctor and the daemon never disagree — even on a non-UTF-8
    // $XDG_RUNTIME_DIR. Without it the daemon fails closed (no /tmp fallback), so
    // doctor advertises no path the daemon would refuse.
    match deep_diff_forge_daemon::runtime_base() {
        Some(base) => emitln!("runtime_dir={}", base.display()),
        None => emitln!("runtime_dir=<unset: set XDG_RUNTIME_DIR>"),
    }
    emitln!("cache_dir={cache_dir}");
    emitln!("state_dir={state_dir}");
    match deep_diff_forge_daemon::SocketLocation::resolve(None) {
        Ok(location) => emitln!("daemon_socket={}", location.path().display()),
        Err(_) => emitln!("daemon_socket=<unavailable: set XDG_RUNTIME_DIR or pass --socket>"),
    }
}

fn claude_code_contract() {
    emitln!(
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
    emitln!(
        "\
deep-diff-forge chain-contract v0

GUARANTEES:
  - chainable commands read stdin only when an explicit stdin flag is present.
  - stdout carries primary records; stderr carries diagnostics and progress.
  - JSON output is one complete document; JSONL output is one event per line.
  - every JSON document names its schema; every JSONL record names its event.
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
    emitln!(
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
    emitln!(
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
