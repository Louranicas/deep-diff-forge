// DDF integration: R02, R09, R19, R21. Validate that a patch transforms the captured base into the captured candidate before attaching semantics; export explicit coverage and snapshot scope.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#shared-service
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-cli--main
mod analysis;
mod args;
mod bench;
mod contract;
mod eval;
mod git_input;
mod mcp;
mod rpc;
mod textfile;
use deep_diff_forge_core::exit;
use deep_diff_forge_core::{InputSeal, ReviewDocument, display_safe, display_safe_bounded};

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

// Declared after `emitln!`, which it uses (a `macro_rules!` is visible only below it).
mod agent;
mod doctor;

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
    let args = argv_or_exit();
    let command = args.first().map(String::as_str);
    let rest = args.get(1..).unwrap_or_default();
    if command.is_some_and(|command| command_help(command, rest)) {
        return;
    }
    refuse_arguments_to_bare_commands(command, rest.first());
    match command {
        None | Some("--help" | "-h" | "help") => print_help(),
        Some("--version" | "-V" | "version") => print_version(),
        Some("--self-test" | "self-test") => self_test(),
        Some("doctor") => doctor(),
        Some("findings") => findings_cmd(rest),
        Some("bench") => bench_cmd(rest),
        Some("eval") => eval_cmd(rest),
        Some("deploy") => {
            deploy_cmd(rest);
        }
        Some("council") => match deep_diff_forge_review::cli::dispatch(rest) {
            Ok((output, code)) => {
                emit(&output);
                if code != 0 {
                    std::process::exit(code);
                }
            }
            Err(error) => {
                eprintln!("error: {}", display_safe_bounded(&error.to_string(), 200));
                std::process::exit(exit::COUNCIL_REFUSED);
            }
        },
        Some("analyze") => {
            analyze_paths_or_exit(rest);
            match analysis::run(rest) {
                Ok(json) => emit(&json),
                Err(error) => {
                    eprintln!("error: {error}");
                    std::process::exit(error.exit_code());
                }
            }
        }
        Some("semantic") => semantic_cmd(rest),
        Some("highlight") => highlight_cmd(rest),
        Some("structural") => structural_cmd(rest),
        Some("review") => review_cmd(rest),
        Some("daemon") => daemon_cmd(rest),
        Some("learn") => learn_cmd(rest),
        Some("--stdin-patch") => stdin_patch(rest),
        Some(selector @ ("--git-working-tree" | "--git-staged" | "--git-range")) => {
            git_input::run(selector, rest);
        }
        Some("schema") => schema_cmd(rest),
        Some("explain") => contract_output(contract::explain(rest)),
        Some("envelope") => run_envelope(rest),
        Some("mcp") => run_mcp(rest),
        Some("agent") => agent::run(rest),
        Some("rpc") => run_rpc(rest),
        Some("claude-code-contract") => claude_code_contract(),
        Some("chain-contract") => chain_contract(),
        Some("cluster-contract") => cluster_contract(),
        Some("loom-contract") => loom_contract(),
        Some(command) => {
            eprintln!("unknown command: {}", display_safe_bounded(command, 200));
            eprintln!("run `deep-diff-forge --help` for supported bootstrap commands");
            std::process::exit(exit::USAGE);
        }
    }
}

/// Decode process arguments without `std::env::args`' non-UTF-8 panic. Paths remain UTF-8 at the
/// current CLI contract; an undecodable argument is therefore an input-read failure, not a bug.
fn argv_or_exit() -> Vec<String> {
    std::env::args_os()
        .skip(1)
        .map(|argument| {
            argument.into_string().unwrap_or_else(|argument| {
                eprintln!(
                    "error: argument is not valid UTF-8: {}",
                    display_safe_bounded(&argument.to_string_lossy(), 200)
                );
                std::process::exit(exit::IO);
            })
        })
        .collect()
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
const REVIEW_FLAGS: [&str; 8] = [
    "--json",
    "--jsonl",
    "--rank",
    "--cluster",
    "--outline",
    "--hunks",
    "--require-files",
    "--require-hunks",
];
/// The valued options shared by `--stdin-patch` and the structured Git selections.
const REVIEW_VALUED: [&str; 6] = [
    "--layout",
    "--parallel",
    "--address",
    "--path",
    "--max-bytes",
    "--cursor",
];
const STDIN_PATCH: args::Spec = args::Spec {
    flags: &REVIEW_FLAGS,
    valued: &REVIEW_VALUED,
    positionals: 0,
};
const STDIN_PATCH_USAGE: &str = "deep-diff-forge --stdin-patch [--json | --jsonl | --rank | --cluster [--parallel N] | --layout inline|side-by-side | --outline | --hunks [--address A]... [--path P]...] [--max-bytes N] [--cursor C] [--require-files] [--require-hunks]";
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
    validate_review_options_or_exit(opts);
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

/// Validate cross-option contracts before acquiring or reading a patch. A recognized option that
/// cannot affect the selected mode is more dangerous than an unknown one: automation otherwise
/// proceeds under a control it only believes was applied.
pub(crate) fn validate_review_options_or_exit(opts: &[String]) {
    let has = |name: &str| opts.iter().any(|arg| arg == name);
    if has("--json") && has("--jsonl") {
        eprintln!("error: --json and --jsonl are mutually exclusive");
        std::process::exit(exit::USAGE);
    }
    // One output mode: before, `--outline --hunks` printed the outline and `--rank --cluster`
    // one of the two, each with exit 0 (bug hunt 2026-10-28, H2-07).
    let modes: Vec<&str> = ["--outline", "--hunks", "--cluster", "--rank"]
        .into_iter()
        .filter(|mode| has(mode))
        .collect();
    if modes.len() > 1 {
        eprintln!(
            "error: {} are separate modes; choose one",
            modes.join(" and ")
        );
        std::process::exit(exit::USAGE);
    }
    if has("--outline")
        && let Some(flag) = ["--address", "--path"].iter().find(|flag| has(flag))
    {
        eprintln!("error: {flag} selects hunks; it requires --hunks, not --outline");
        std::process::exit(exit::USAGE);
    }
    let view = has("--outline") || has("--hunks");
    if !view
        && let Some(flag) = ["--address", "--path", "--max-bytes", "--cursor"]
            .iter()
            .find(|flag| has(flag))
    {
        eprintln!("error: {flag} requires --outline or --hunks");
        std::process::exit(exit::USAGE);
    }
    if has("--parallel") && !has("--cluster") {
        eprintln!("error: --parallel requires --cluster");
        std::process::exit(exit::USAGE);
    }
    if let Some(layout) = flag_value(opts, "--layout")
        && deep_diff_forge_projection::layout_from_str(&layout).is_none()
    {
        eprintln!(
            "error: unknown layout: {} (expected inline|side-by-side)",
            display_safe_bounded(&layout, 200)
        );
        std::process::exit(exit::USAGE);
    }
    if has("--max-bytes") {
        let _ = view_page(opts);
    }
    if has("--parallel") {
        let _ = parse_parallelism(opts);
    }
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
    if opts.iter().any(|a| a == "--outline") {
        emit_view(opts, |page| {
            deep_diff_forge_daemon::views::outline(review, page)
        });
    } else if opts.iter().any(|a| a == "--hunks") {
        let addresses = flag_values(opts, "--address");
        let paths = flag_values(opts, "--path");
        let selection = match (addresses.is_empty(), paths.is_empty()) {
            (true, true) => deep_diff_forge_daemon::views::Selection::All,
            (false, true) => deep_diff_forge_daemon::views::Selection::Addresses(addresses),
            (true, false) => deep_diff_forge_daemon::views::Selection::Paths(paths),
            (false, false) => {
                eprintln!("error: --hunks takes --address or --path, not both");
                std::process::exit(exit::USAGE);
            }
        };
        emit_view(opts, |page| {
            deep_diff_forge_daemon::views::hunks(review, &selection, page)
        });
    } else if opts.iter().any(|a| a == "--cluster") {
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

/// `daemon {path|start|health|status|metrics|stop} [--socket PATH]`: drive the optional
/// UDS JSON-RPC review daemon.
const DAEMON_USAGE: &str = "deep-diff-forge daemon {path|start [--foreground] [--allow-repo PATH]... [--log-format text|json]|health|status|metrics [--format json|prometheus] [--out PATH]|stop} [--socket PATH]";

fn daemon_cmd(opts: &[String]) {
    use deep_diff_forge_daemon::SocketLocation;
    let positionals = args::admit_or_exit(
        opts,
        &args::Spec {
            flags: &["--foreground"],
            valued: &[
                "--socket",
                "--allow-repo",
                "--log-format",
                "--format",
                "--out",
            ],
            positionals: 1,
        },
        DAEMON_USAGE,
    );
    let sub = positionals.first().copied();
    // Validate the subcommand first, so an unknown one reports usage whatever the environment.
    // `start` always serves in the foreground (a supervisor such as systemd owns the process);
    // `--foreground` says so and belongs to `start` alone.
    if sub != Some("start") && opts.iter().any(|a| a == "--foreground") {
        eprintln!("error: --foreground applies only to `daemon start`");
        eprintln!("usage: {DAEMON_USAGE}");
        std::process::exit(exit::USAGE);
    }
    if sub != Some("start") && opts.iter().any(|a| a == "--allow-repo") {
        eprintln!("error: --allow-repo applies only to `daemon start`");
        std::process::exit(exit::USAGE);
    }
    let log_format = match flag_value(opts, "--log-format").as_deref() {
        _ if sub != Some("start") && opts.iter().any(|a| a == "--log-format") => {
            eprintln!("error: --log-format applies only to `daemon start`");
            std::process::exit(exit::USAGE);
        }
        None | Some("text") => deep_diff_forge_daemon::LogFormat::Text,
        Some("json") => deep_diff_forge_daemon::LogFormat::Json,
        Some(other) => {
            eprintln!(
                "error: --log-format takes text or json, not {}",
                display_safe(other)
            );
            std::process::exit(exit::USAGE);
        }
    };
    let metrics_output = metrics_output_or_exit(opts, sub);
    if !matches!(
        sub,
        Some("path" | "start" | "health" | "status" | "metrics" | "stop")
    ) {
        eprintln!("usage: {DAEMON_USAGE}");
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
            let roots: Vec<std::path::PathBuf> = flag_values(opts, "--allow-repo")
                .into_iter()
                .map(std::path::PathBuf::from)
                .collect();
            let engine = engine_allowing_or_exit(&roots).with_log_format(log_format);
            if let Err(err) = deep_diff_forge_daemon::run_server_with(&location, engine) {
                eprintln!("error: daemon failed: {err}");
                std::process::exit(exit::DAEMON_OR_TERMINAL);
            }
        }
        Some("health") => daemon_client(&location, r#"{"id":1,"method":"daemon.health"}"#),
        Some("status") => daemon_client(&location, r#"{"id":1,"method":"daemon.status"}"#),
        Some("metrics") => match metrics_output {
            MetricsOutput::Prometheus(out) => daemon_metrics_prometheus(&location, out.as_deref()),
            MetricsOutput::Json => {
                daemon_client(&location, r#"{"id":1,"method":"daemon.metrics"}"#);
            }
        },
        Some("stop") => daemon_stop(&location),
        _ => {
            eprintln!("usage: {DAEMON_USAGE}");
            std::process::exit(exit::USAGE);
        }
    }
}

/// `daemon stop`: ask the daemon to shut down, then wait for it to exit.
fn daemon_stop(location: &deep_diff_forge_daemon::SocketLocation) {
    // The start lock is opened before the stop, while the socket path still resolves.
    let waiter = location.stop_waiter();
    daemon_client(location, r#"{"id":1,"method":"daemon.shutdown"}"#);
    // `stop` returns once the daemon has exited (its drain done, its start lock free),
    // so a `daemon start` right after it does not find the old daemon in the way.
    match waiter.and_then(|waiter| waiter.wait(deep_diff_forge_daemon::STOP_WAIT)) {
        Ok(true) => {}
        Ok(false) => {
            eprintln!(
                "error: the daemon at {} did not exit within {} s",
                display_safe(&location.path().to_string_lossy()),
                deep_diff_forge_daemon::STOP_WAIT.as_secs()
            );
            std::process::exit(exit::DAEMON_OR_TERMINAL);
        }
        Err(err) => {
            eprintln!(
                "error: cannot tell whether the daemon at {} exited: {err}",
                display_safe(&location.path().to_string_lossy())
            );
            std::process::exit(exit::DAEMON_OR_TERMINAL);
        }
    }
}

/// What `daemon metrics` prints.
enum MetricsOutput {
    /// The `daemon.metrics` answer as the daemon sent it.
    Json,
    /// The Prometheus text, on stdout or written to the `--out` path.
    Prometheus(Option<String>),
}

/// The `--format` and `--out` of `daemon metrics`; exit 2 on one that does not apply.
fn metrics_output_or_exit(opts: &[String], sub: Option<&str>) -> MetricsOutput {
    let prometheus = match flag_value(opts, "--format").as_deref() {
        _ if sub != Some("metrics") && opts.iter().any(|a| a == "--format") => {
            eprintln!("error: --format applies only to `daemon metrics`");
            std::process::exit(exit::USAGE);
        }
        None | Some("json") => false,
        Some("prometheus") => true,
        Some(other) => {
            eprintln!(
                "error: --format takes json or prometheus, not {}",
                display_safe(other)
            );
            std::process::exit(exit::USAGE);
        }
    };
    // `--out` writes the Prometheus text for node_exporter's textfile collector; the JSON answer
    // stays on stdout.
    let out = flag_value(opts, "--out");
    if out.is_some() && !prometheus {
        eprintln!("error: --out applies only to `daemon metrics --format prometheus`");
        std::process::exit(exit::USAGE);
    }
    if prometheus {
        MetricsOutput::Prometheus(out)
    } else {
        MetricsOutput::Json
    }
}

fn daemon_client(location: &deep_diff_forge_daemon::SocketLocation, line: &str) {
    emitln!("{}", daemon_request_or_exit(location, line));
}

/// `daemon metrics --format prometheus [--out PATH]`: the daemon's `daemon.metrics` answer in the
/// Prometheus text format, on stdout or written whole to PATH. A daemon that answers no usable
/// metrics document exits 6 and leaves PATH as it was, so a stale file shows in its mtime.
fn daemon_metrics_prometheus(location: &deep_diff_forge_daemon::SocketLocation, out: Option<&str>) {
    let response = daemon_request_or_exit(location, r#"{"id":1,"method":"daemon.metrics"}"#);
    let reply: serde_json::Value = serde_json::from_str(&response).unwrap_or_default();
    let text = match deep_diff_forge_daemon::exposition::prometheus_text(&reply["result"]) {
        Ok(text) => text,
        Err(reason) => {
            let reason = reply["error"]["message"]
                .as_str()
                .map_or(reason, |message| {
                    format!("the daemon refused daemon.metrics: {message}")
                });
            eprintln!(
                "error: no metrics from the daemon at {}: {}",
                display_safe(&location.path().to_string_lossy()),
                display_safe(&reason)
            );
            std::process::exit(exit::DAEMON_OR_TERMINAL);
        }
    };
    let Some(out) = out else {
        emit(&text);
        return;
    };
    if let Err(err) = textfile::write_atomically(std::path::Path::new(out), text.as_bytes()) {
        eprintln!("error: cannot write {}: {err}", display_safe(out));
        std::process::exit(exit::IO);
    }
}

/// One request to the daemon and its answer line; exit 6 when no daemon this user may use
/// answers.
fn daemon_request_or_exit(location: &deep_diff_forge_daemon::SocketLocation, line: &str) -> String {
    match deep_diff_forge_daemon::request(location, line) {
        Ok(response) => response,
        Err(err) => {
            // Something is there, but this user may not use it: a daemon of another user, or a
            // socket whose mode refuses the connect.
            let what = if err.kind() == std::io::ErrorKind::PermissionDenied {
                "cannot use the daemon at"
            } else {
                "no daemon at"
            };
            eprintln!(
                "error: {what} {}: {err}",
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
const REVIEW_USAGE: &str = "deep-diff-forge review [--probe [--cols N] [--rows N]] [--side] [--palette | --cmd NAME] [--findings FILE]";

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
            valued: &["--cmd", "--cols", "--rows", "--findings"],
            positionals: 0,
        },
        REVIEW_USAGE,
    );
    let findings = flag_value(opts, "--findings").map(|path| read_findings_or_exit(&path));
    // An unknown command name or a size that is not a number is refused before stdin is read.
    // Before, the name fell through to `rank` and the size to its default, both with exit 0.
    let command = flag_value(opts, "--cmd").map(|name| palette_index_or_exit(&name));
    let cols = probe_size_or_exit(opts, "--cols", 100).max(40);
    let rows = probe_size_or_exit(opts, "--rows", 30).max(8);

    let input = read_capped_or_exit(std::io::stdin().lock(), "stdin");
    let files = parse_or_exit(&input);
    if let Some(path) = deep_diff_forge_tui::duplicate_path(&files) {
        eprintln!(
            "error: the patch names \"{}\" twice; the cockpit matches content and notes by path and \
             would show one section for both. Review each patch on its own.",
            deep_diff_forge_core::display_safe(path)
        );
        std::process::exit(exit::INVALID_INPUT);
    }
    // Surface the engine's own findings (risk signals, semantic changes) as
    // grounded, anchored inline notes so the review explains *why* a file
    // matters in-place.
    let ranked = deep_diff_forge_graph::rank(&files);
    let mut notes = deep_diff_forge_tui::engine_annotations(&files, &ranked);
    // `--findings`: what agents found, checked against this patch first; only accepted findings
    // become notes, as agent notes that are never grounded.
    if let Some(findings) = findings {
        use deep_diff_forge_agent::findings::{annotations, verify};
        let seal = deep_diff_forge_core::sha256_hex(input.as_bytes());
        let verdict = verify(&files, &seal, findings.lines());
        eprintln!(
            "findings: {} accepted, {} refused, {} over the limit (findings verify shows why)",
            verdict.accepted.len(),
            verdict.refused.len(),
            verdict.over_limit
        );
        notes.extend(annotations(&verdict.accepted, &files));
    }
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
  deep-diff-forge review [--probe [--cols N] [--rows N]] [--side] [--palette | --cmd NAME] [--findings FILE]

Reads a unified/Git patch from stdin and opens the review TUI.

OPTIONS:
  --probe        Render one headless frame; does not require a TTY
  --cols N       Probe width, minimum 40 columns
  --rows N       Probe height, minimum 8 rows
  --side         Start in side-by-side layout
  --palette      Open the command palette before rendering/running
  --cmd NAME     Run a palette command and show its result panel
  --findings F   Add the agent findings in F (JSONL) that pass findings verify, as agent notes
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
    use std::os::unix::fs::OpenOptionsExt as _;
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NONBLOCK)
        .open(path)
    {
        Ok(f) => f,
        Err(err) => {
            eprintln!("error: could not read {}: {err}", display_safe(path));
            std::process::exit(exit::IO);
        }
    };
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        eprintln!(
            "error: source is not a regular file: {}",
            display_safe_bounded(path, 200)
        );
        std::process::exit(exit::IO);
    }
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
        // Line feeds stay line breaks without colour too (bug hunt 2026-10-28, H4-01: the first
        // fix covered only the coloured path, so piped output was still one line).
        emit(&deep_diff_forge_syntax::display_safe_lines(&source));
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

const BENCH_USAGE: &str = "deep-diff-forge bench [--quick] [--workload NAME]... [--json]";

/// `bench`: time the engine's hot paths on generated inputs (15 repetitions, or 5 with
/// `--quick`) and print `deep-diff-forge.bench.v0` (`--json`) or a table. Exit 2 for an unknown
/// workload.
fn bench_cmd(opts: &[String]) {
    args::admit_or_exit(
        opts,
        &args::Spec {
            flags: &["--quick", "--json"],
            valued: &["--workload"],
            positionals: 0,
        },
        BENCH_USAGE,
    );
    let repetitions = if opts.iter().any(|a| a == "--quick") {
        5
    } else {
        15
    };
    let receipt = match bench::run(&flag_values(opts, "--workload"), repetitions) {
        Ok(receipt) => receipt,
        Err(unknown) => {
            eprintln!(
                "error: no workload {}; the workloads are {}",
                display_safe(&unknown),
                bench::workload_names().join(", ")
            );
            std::process::exit(exit::USAGE);
        }
    };
    if opts.iter().any(|a| a == "--json") {
        emitln!(
            "{}",
            serde_json::to_string_pretty(&receipt).unwrap_or_default()
        );
        return;
    }
    emitln!(
        "deep-diff-forge bench ({} build, {} repetitions)",
        receipt["profile"].as_str().unwrap_or("?"),
        repetitions
    );
    for w in receipt["workloads"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
    {
        emitln!(
            "  {:<20} median {:>11} ms  p95 {:>11} ms  {:>10} KiB/s",
            w["name"].as_str().unwrap_or("?"),
            bench::millis(w["median_ns"].as_u64().unwrap_or(0)),
            bench::millis(w["p95_ns"].as_u64().unwrap_or(0)),
            w["kib_per_s"].as_u64().unwrap_or(0)
        );
    }
}

const EVAL_USAGE: &str = "deep-diff-forge eval [--corpus DIR] [--json]";

/// `eval`: score the engine against the labelled corpus in DIR (default `fixtures/eval`) and
/// print `deep-diff-forge.eval.v0` (`--json`) or a summary. Exit 0 when every case passes, 7 when
/// one fails, 4 when the corpus is malformed or out of date, 3 when a file cannot be read.
fn eval_cmd(opts: &[String]) {
    args::admit_or_exit(
        opts,
        &args::Spec {
            flags: &["--json"],
            valued: &["--corpus"],
            positionals: 0,
        },
        EVAL_USAGE,
    );
    let dir = flag_value(opts, "--corpus").unwrap_or_else(|| "fixtures/eval".to_owned());
    let receipt = match eval::run(std::path::Path::new(&dir)) {
        Ok(receipt) => receipt,
        Err(err) => {
            eprintln!("error: eval: {}", display_safe(&err.to_string()));
            std::process::exit(match err {
                eval::EvalError::Read(_) => exit::IO,
                eval::EvalError::Corpus(_) => exit::INVALID_INPUT,
            });
        }
    };
    let failures = receipt["failures"].as_array().map_or(0, Vec::len);
    if opts.iter().any(|a| a == "--json") {
        emitln!(
            "{}",
            serde_json::to_string_pretty(&receipt).unwrap_or_default()
        );
    } else {
        emitln!("deep-diff-forge eval: {} cases", receipt["cases"]);
        for (name, m) in receipt["metrics"].as_object().into_iter().flatten() {
            emitln!("  {name:<20} {}/{}", m["right"], m["total"]);
        }
        emitln!("  known misses         {}", receipt["known_misses"]);
        for failure in receipt["failures"].as_array().into_iter().flatten() {
            emitln!("  FAIL {}", failure.as_str().unwrap_or(""));
        }
    }
    if failures > 0 {
        std::process::exit(EXIT_REFUSED);
    }
}

const FINDINGS_USAGE: &str = "deep-diff-forge findings verify --findings FILE [--require-all] [--require-files] [--require-hunks]";

/// `findings verify`: check agent findings (JSONL, one `deep-diff-forge.finding.v0` per line)
/// against the patch on stdin, and print the sealed `deep-diff-forge.findings.v0` verdict.
/// `findings_sha256` covers the file's exact bytes. Exit 0; 7 with `--require-all` when any
/// finding is refused or over the limit (or `--require-files` / `--require-hunks` refuse the
/// patch); 4 when the patch does not parse or the findings file is over 8 MiB or not UTF-8; 3
/// when it cannot be read; 2 on a usage error.
fn findings_cmd(opts: &[String]) {
    use deep_diff_forge_agent::findings::{document, verify};
    let positionals = args::admit_or_exit(
        opts,
        &args::Spec {
            flags: &["--require-all", "--require-files", "--require-hunks"],
            valued: &["--findings"],
            positionals: 1,
        },
        FINDINGS_USAGE,
    );
    let (Some("verify"), Some(path)) =
        (positionals.first().copied(), flag_value(opts, "--findings"))
    else {
        eprintln!("usage: {FINDINGS_USAGE}");
        std::process::exit(exit::USAGE);
    };
    let text = read_findings_or_exit(&path);
    let bytes = read_capped_bytes_or_exit(std::io::stdin().lock(), "stdin");
    let review = sealed_review_or_exit(&bytes, InputGuards::from_opts(opts));
    let seal = review.seal();
    let verdict = verify(review.files(), &seal.input_sha256, text.lines());
    let doc = document(
        &verdict,
        &seal.input_sha256,
        &deep_diff_forge_core::sha256_hex(text.as_bytes()),
        &seal.tool_version,
    );
    emitln!("{}", serde_json::to_string_pretty(&doc).unwrap_or_default());
    let all_accepted = verdict.refused.is_empty() && verdict.over_limit == 0;
    if opts.iter().any(|a| a == "--require-all") && !all_accepted {
        std::process::exit(EXIT_REFUSED);
    }
}

/// A findings file: at most 8 MiB of UTF-8. Exit 3 when it cannot be read, 4 when it is over
/// the limit or not UTF-8.
fn read_findings_or_exit(path: &str) -> String {
    use deep_diff_forge_agent::findings::MAX_FINDINGS_BYTES;
    use std::io::Read as _;
    let mut findings = Vec::new();
    let read = std::fs::File::open(path).and_then(|file| {
        file.take(MAX_FINDINGS_BYTES as u64 + 1)
            .read_to_end(&mut findings)
    });
    if let Err(err) = read {
        eprintln!("error: cannot read findings {}: {err}", display_safe(path));
        std::process::exit(exit::IO);
    }
    if findings.len() > MAX_FINDINGS_BYTES {
        eprintln!(
            "error: findings {} are over the {MAX_FINDINGS_BYTES}-byte limit",
            display_safe(path)
        );
        std::process::exit(exit::INVALID_INPUT);
    }
    String::from_utf8(findings).unwrap_or_else(|_| {
        eprintln!("error: findings {} are not UTF-8", display_safe(path));
        std::process::exit(exit::INVALID_INPUT);
    })
}

const DEPLOY_USAGE: &str =
    "deep-diff-forge deploy {status|release|doctor} [--json] [--socket PATH]";

/// Dispatch `deploy` subcommands.
fn deploy_cmd(opts: &[String]) {
    let positionals = args::admit_or_exit(
        opts,
        &args::Spec {
            flags: &["--json"],
            valued: &["--socket"],
            positionals: 1,
        },
        DEPLOY_USAGE,
    );
    let json = opts.iter().any(|a| a == "--json");
    let socket = flag_value(opts, "--socket");
    let sub = positionals.first().copied();
    if socket.is_some() && sub != Some("doctor") {
        eprintln!("error: --socket is accepted only with deploy doctor");
        eprintln!("usage: {DEPLOY_USAGE}");
        std::process::exit(exit::USAGE);
    }
    match sub {
        Some("status") => deploy_status(json),
        Some("release") => deploy_release(json),
        Some("doctor") => deploy_doctor(json, socket.as_deref().map(std::path::Path::new)),
        _ => {
            eprintln!("usage: {DEPLOY_USAGE}");
            std::process::exit(exit::USAGE);
        }
    }
}

/// `deploy doctor`: check the daemon location and Git, then exit 6 if a check failed.
fn deploy_doctor(json: bool, socket: Option<&std::path::Path>) {
    let findings = doctor::gather(socket);
    let checks = doctor::checks(&findings);
    if json {
        emitln!(
            "{}",
            serde_json::to_string_pretty(&doctor::document(findings.version, &checks))
                .unwrap_or_default()
        );
    } else {
        emit(&doctor::text(&checks));
    }
    std::process::exit(doctor::exit_code(&checks));
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
            "    {{\"path\": {}, \"status\": {}, \"score\": {}, \"signals\": [{}]",
            json_escape(&rf.path),
            json_escape(rf.status.label()),
            rf.score,
            signals.join(", ")
        );
        // Present only when found, as `hazards` is (the daemon's `diff.rank` is the same
        // document: cli/tests/daemon_parity.rs).
        if !rf.evidence.is_empty() {
            let evidence: Vec<String> = rf
                .evidence
                .iter()
                .map(|e| {
                    format!(
                        "{{\"signal\": {}, \"address\": {}, \"side\": {}, \"line\": {}, \"basis\": {}}}",
                        json_escape(e.signal.label()),
                        json_escape(&e.address),
                        json_escape(e.side.label()),
                        e.line,
                        json_escape(deep_diff_forge_graph::EVIDENCE_BASIS)
                    )
                })
                .collect();
            let _ = write!(items, ", \"evidence\": [{}]", evidence.join(", "));
        }
        if rf.evidence_truncated {
            items.push_str(", \"evidence_truncated\": true");
        }
        items.push('}');
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

/// `--max-bytes` and `--cursor` for `--outline` / `--hunks` (the daemon's view budget and page).
fn view_page(opts: &[String]) -> deep_diff_forge_daemon::views::Page {
    let max_bytes = flag_value(opts, "--max-bytes").map_or(
        deep_diff_forge_daemon::views::DEFAULT_MAX_BYTES,
        |value| {
            value.parse().unwrap_or_else(|_| {
                eprintln!(
                    "error: --max-bytes takes a number of bytes, not {}",
                    display_safe(&value)
                );
                std::process::exit(exit::USAGE);
            })
        },
    );
    deep_diff_forge_daemon::views::Page {
        max_bytes,
        cursor: flag_value(opts, "--cursor"),
    }
}

/// Print a view as the one-line JSON the daemon and MCP return for the same page, so
/// `--max-bytes` bounds what is printed (pretty printing added up to half again, bug hunt
/// 2026-10-28, H5-02) and a CLI page is byte-for-byte a daemon page; a refused request
/// (budget, cursor, address, path) exits 2.
fn emit_view(
    opts: &[String],
    view: impl Fn(
        &deep_diff_forge_daemon::views::Page,
    ) -> Result<serde_json::Value, deep_diff_forge_daemon::views::ViewError>,
) {
    match view(&view_page(opts)) {
        Ok(value) => emitln!(
            "{}",
            deep_diff_forge_core::json_line_safe(&value.to_string())
        ),
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(exit::USAGE);
        }
    }
}

/// Every value following `name` in `opts`, in order (for repeatable options).
fn flag_values(opts: &[String], name: &str) -> Vec<String> {
    opts.windows(2)
        .filter(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
        .collect()
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
        DEPLOY_USAGE,
        "Report the declared deployment state or the per-target release posture, or check this machine (doctor: exit 6 when a check fails).",
    ),
    (
        "bench",
        BENCH_USAGE,
        "Time the engine's hot paths on generated inputs; --json prints the deep-diff-forge.bench.v0 receipt that scripts/bench_compare.py compares.",
    ),
    (
        "eval",
        EVAL_USAGE,
        "Score ranking, code evidence and findings against the labelled corpus (deep-diff-forge.eval.v0); exit 7 when a case fails.",
    ),
    (
        "findings",
        FINDINGS_USAGE,
        "Check agent findings (JSONL) against the patch on stdin: each is accepted with its anchor, address and line normalised, or refused with a reason; claims are never verified.",
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
        REVIEW_USAGE,
        "Open the review cockpit on a patch from stdin; --findings adds the agent findings that pass `findings verify`, as agent notes.",
    ),
    (
        "daemon",
        DAEMON_USAGE,
        "Run or query the optional Unix-socket JSON-RPC daemon; --allow-repo lets review.change read those repositories; metrics --format prometheus --out PATH writes a node_exporter textfile.",
    ),
    (
        "learn",
        "deep-diff-forge learn {status|record --stdin} [--json]",
        "Report or feed the local-only learning loop.",
    ),
    (
        "--stdin-patch",
        "deep-diff-forge --stdin-patch [--json | --jsonl | --rank | --cluster [--parallel N] | --layout inline|side-by-side | --outline | --hunks [--address A]... [--path P]...] [--max-bytes N] [--cursor C] [--require-files] [--require-hunks]",
        "Review a unified or Git patch read from stdin.",
    ),
    (
        "--git-working-tree",
        "deep-diff-forge {--git-working-tree [--include-untracked] | --git-staged | --git-range A..B} [--repo PATH] [--acquisition-json | <--stdin-patch output flags>]",
        "Review a change Git returns, sealed over the exact bytes.",
    ),
    (
        "agent",
        "deep-diff-forge agent {setup --client claude|codex [--allow-repo PATH]... | doctor} [--json]",
        "Print the MCP setup for Claude Code or Codex (never written), or check that every agent surface agrees.",
    ),
    (
        "mcp",
        "deep-diff-forge mcp [--allow-repo PATH]...",
        "Serve the read-only MCP tools over stdio; --allow-repo lets review_change read those repositories.",
    ),
    (
        "rpc",
        "deep-diff-forge rpc [--allow-repo PATH]...",
        "Speak the daemon's JSON-RPC over stdio, with no daemon; --allow-repo lets review.change read those repositories.",
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

/// `<command> --help` (or `-h`) anywhere: print that command's usage and what it
/// does, and exit 0. `review` prints its own longer help; `council --help` is `council help`.
fn command_help(command: &str, args: &[String]) -> bool {
    let asked = args.iter().any(|a| matches!(a.as_str(), "--help" | "-h"));
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
  names one exit code. mcp serves read-only MCP tools over stdio (review_outline,
  review_hunks, review_patch, rank_patch, review_change, verify_findings,
  explain_exit_code, exit_code_registry).
  rpc speaks the daemon's JSON-RPC protocol over stdio without a daemon.

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

/// The repository roots named by `--allow-repo`, the only options `rpc` and `mcp` take; each
/// must exist. Nothing is allowed unless the operator names it.
fn allowed_repos_or_exit(args: &[String], usage: &str) -> Vec<std::path::PathBuf> {
    args::admit_or_exit(
        args,
        &args::Spec {
            flags: &[],
            valued: &["--allow-repo"],
            positionals: 0,
        },
        usage,
    );
    flag_values(args, "--allow-repo")
        .into_iter()
        .map(std::path::PathBuf::from)
        .collect()
}

/// An engine allowed to read `roots`, or exit 2 naming the root that does not resolve.
fn engine_allowing_or_exit(roots: &[std::path::PathBuf]) -> deep_diff_forge_daemon::Engine {
    deep_diff_forge_daemon::Engine::new()
        .allowing_repos(roots)
        .unwrap_or_else(|(root, e)| {
            eprintln!(
                "error: --allow-repo {}: {e}",
                display_safe(&root.to_string_lossy())
            );
            std::process::exit(exit::USAGE);
        })
}

/// `rpc`: the daemon's JSON-RPC over stdio, in-process.
fn run_rpc(args: &[String]) {
    let roots = allowed_repos_or_exit(args, "deep-diff-forge rpc [--allow-repo PATH]...");
    let (mut input, mut output) = (std::io::stdin().lock(), std::io::stdout().lock());
    let served = if roots.is_empty() {
        rpc::serve(&mut input, &mut output)
    } else {
        rpc::serve_engine(&mut input, &mut output, &engine_allowing_or_exit(&roots))
    };
    match served {
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
    let roots = allowed_repos_or_exit(args, "deep-diff-forge mcp [--allow-repo PATH]...");
    // Resolve every root now, so a typo fails at start rather than at the first call.
    let canonical: Vec<std::path::PathBuf> = roots
        .iter()
        .map(|root| {
            std::fs::canonicalize(root).unwrap_or_else(|e| {
                eprintln!(
                    "error: --allow-repo {}: {e}",
                    display_safe(&root.to_string_lossy())
                );
                std::process::exit(exit::USAGE);
            })
        })
        .collect();
    let (mut input, mut output) = (std::io::stdin().lock(), std::io::stdout().lock());
    let served = if canonical.is_empty() {
        mcp::serve(&mut input, &mut output)
    } else {
        mcp::serve_allowing(&mut input, &mut output, canonical)
    };
    match served {
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
