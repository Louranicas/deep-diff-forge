#![no_main]
// DDF slice C: in-process fuzz of the `deep-diff-forge rpc` stdio loop (the daemon socket's
// framing and `process_line`), compiled from the CLI's own module source.
// Design/status: docs/ROADMAP.md#status-2026-10-26 and docs/API_AND_IPC.md.
use libfuzzer_sys::fuzz_target;

#[expect(
    dead_code,
    reason = "the fuzz target drives only the stdio loop of the CLI module"
)]
#[path = "../../crates/deep-diff-forge-cli/src/rpc.rs"]
mod rpc;
#[path = "stdio_invariants.rs"]
mod stdio_invariants;

fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }
    let Some((cap, input)) =
        stdio_invariants::cap_and_input(data, deep_diff_forge_daemon::MAX_REQUEST_BYTES)
    else {
        return;
    };
    let mut output = Vec::new();
    rpc::serve_with(&mut &input[..], &mut output, cap).expect("in-memory stdio cannot fail");
    let responses = stdio_invariants::check_responses(&output);
    assert!(
        responses <= stdio_invariants::line_count(input),
        "at most one response per input line"
    );
});
