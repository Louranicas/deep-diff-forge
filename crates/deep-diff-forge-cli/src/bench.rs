// DDF integration: deterministic benchmarks with receipts (plan 2026-10-30, M2 benchmarking).
// Design/status: docs/PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY.md#system-m-monitoring-evaluation-and-benchmarking
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcrates--deep-diff-forge-cli--src--bench
//! `deep-diff-forge bench`: time the engine's hot paths on generated inputs and print a receipt
//! (`deep-diff-forge.bench.v0`). The inputs are generated from fixed parameters, so every run
//! measures the same bytes, and each workload names its input's SHA-256. A receipt names the
//! build profile and the host, because numbers from different machines are not comparable;
//! `scripts/bench_compare.py` compares two receipts from one machine.

use serde_json::{Value, json};
use std::fmt::Write as _;
use std::time::{Duration, Instant};

/// A workload: a name, the input it measures, and one timed repetition.
struct Workload {
    name: &'static str,
    input: String,
    run: Box<dyn Fn(&str)>,
}

/// The bench clock door.
#[expect(
    clippy::disallowed_methods,
    reason = "the bench clock door: times each repetition of a workload"
)]
fn now() -> Instant {
    Instant::now()
}

/// A patch of `files` Rust files with `hunks` hunks of `lines` changed lines each. Of every 35
/// added lines, one carries `unsafe` and one a process spawn (a 20-line hunk has one of each),
/// so code evidence has work to do.
#[must_use]
pub fn generated_patch(files: usize, hunks: usize, lines: usize) -> String {
    let mut out = String::new();
    for f in 0..files {
        let _ = writeln!(out, "diff --git a/src/m{f}.rs b/src/m{f}.rs");
        let _ = writeln!(out, "--- a/src/m{f}.rs\n+++ b/src/m{f}.rs");
        for h in 0..hunks {
            let start = h * (lines + 10) + 1;
            let _ = writeln!(
                out,
                "@@ -{start},{n} +{start},{n} @@ fn f{h}()",
                n = lines + 1
            );
            let _ = writeln!(out, " fn f{h}() {{");
            for l in 0..lines {
                let _ = writeln!(out, "-    let v{l} = {l};");
                let new = match l % 35 {
                    0 => format!("    unsafe {{ g({l}) }} // f{f}"),
                    5 => format!("    let c{l} = std::process::Command::new(\"x{l}\");"),
                    _ => format!("    let v{l} = {} + {f};", l + 1),
                };
                let _ = writeln!(out, "+{new}");
            }
        }
    }
    out
}

/// A Rust source of `items` functions, and the same source with every fifth body changed and
/// every third function reformatted.
fn generated_sources(items: usize) -> (String, String) {
    let mut old = String::new();
    let mut new = String::new();
    for i in 0..items {
        let _ = writeln!(
            old,
            "fn f{i}(a: u32) -> u32 {{\n    let b = a + {i};\n    b * 2\n}}\n"
        );
        if i % 5 == 0 {
            let _ = writeln!(
                new,
                "fn f{i}(a: u32) -> u32 {{\n    let b = a + {};\n    b * 3\n}}\n",
                i + 1
            );
        } else if i % 3 == 0 {
            let _ = writeln!(new, "fn f{i}(a: u32) -> u32 {{ let b = a + {i}; b * 2 }}\n");
        } else {
            let _ = writeln!(
                new,
                "fn f{i}(a: u32) -> u32 {{\n    let b = a + {i};\n    b * 2\n}}\n"
            );
        }
    }
    (old, new)
}

fn review(input: &str) -> deep_diff_forge_patch::SealedReview {
    deep_diff_forge_patch::SealedReview::new(
        input.as_bytes(),
        deep_diff_forge_patch::InputGuards {
            files: false,
            hunks: false,
        },
        env!("CARGO_PKG_VERSION"),
    )
    .unwrap_or_else(|e| panic!("a generated bench input must parse: {e}"))
}

/// Findings anchored by path and line to every tenth changed line of `patch`'s files.
fn generated_findings(files: usize, hunks: usize, lines: usize) -> String {
    let mut out = String::new();
    for f in 0..files {
        for h in 0..hunks {
            for l in (0..lines).step_by(10) {
                let line = h * (lines + 10) + 2 + l;
                let _ = writeln!(
                    out,
                    "{}",
                    json!({"agent": "bench", "severity": "low", "title": format!("t{f}-{h}-{l}"), "body": "b", "path": format!("src/m{f}.rs"), "line": line})
                );
            }
        }
    }
    out
}

/// Findings anchored by hunk address to every hunk of `patch`, `per_hunk` each.
fn findings_by_address(patch: &str, per_hunk: usize) -> String {
    let mut out = String::new();
    for file in review(patch).files() {
        for hunk in &file.patch_twin.hunks {
            let address = hunk.address(&file.path);
            for i in 0..per_hunk {
                let _ = writeln!(
                    out,
                    "{}",
                    json!({"agent": "bench", "severity": "low", "title": format!("t{i}"), "body": "b", "address": address})
                );
            }
        }
    }
    out
}

fn workloads() -> Vec<Workload> {
    let patch = generated_patch(200, 4, 20);
    let (old, new) = generated_sources(400);
    let structural = format!("{old}\0{new}");
    let findings_patch = generated_patch(50, 4, 20);
    let findings = generated_findings(50, 4, 20);
    let parallel_patch = generated_patch(40, 4, 20);
    let address_patch = generated_patch(100, 4, 20);
    let by_address = findings_by_address(&address_patch, 5);
    vec![
        Workload {
            name: "parse_seal",
            input: patch.clone(),
            run: Box::new(|input| {
                let _ = review(input);
            }),
        },
        Workload {
            name: "rank_with_evidence",
            input: patch.clone(),
            run: Box::new({
                let files = review(&patch).files().to_vec();
                move |_| {
                    let _ = deep_diff_forge_graph::rank(&files);
                }
            }),
        },
        Workload {
            name: "outline_page",
            input: patch.clone(),
            run: Box::new({
                let sealed = review(&patch);
                move |_| {
                    let _ = deep_diff_forge_daemon::views::outline(
                        &sealed,
                        &deep_diff_forge_daemon::views::Page::default(),
                    );
                }
            }),
        },
        Workload {
            name: "findings_verify",
            input: findings.clone(),
            run: Box::new({
                let sealed = review(&findings_patch);
                move |input| {
                    let _ = deep_diff_forge_agent::findings::verify(
                        sealed.files(),
                        &sealed.seal().input_sha256,
                        input.lines(),
                    );
                }
            }),
        },
        Workload {
            name: "findings_by_address",
            input: by_address,
            run: Box::new({
                let sealed = review(&address_patch);
                move |input| {
                    let _ = deep_diff_forge_agent::findings::verify(
                        sealed.files(),
                        &sealed.seal().input_sha256,
                        input.lines(),
                    );
                }
            }),
        },
        Workload {
            name: "structural_rust",
            input: structural,
            run: Box::new(|input| {
                let (old, new) = input.split_once('\0').unwrap_or((input, input));
                let _ = deep_diff_forge_syntax::structural_diff(
                    deep_diff_forge_syntax::Language::Rust,
                    old,
                    new,
                );
            }),
        },
        Workload {
            name: "daemon_parallel",
            input: parallel_patch,
            run: Box::new(|input| {
                // Four clients, four `diff.rank` each, through the daemon's own request path
                // (admission, dispatch, metrics) on one shared engine.
                let engine = deep_diff_forge_daemon::Engine::new();
                let line = json!({"jsonrpc": "2.0", "id": 1, "method": "diff.rank", "params": {"patch": input}}).to_string();
                std::thread::scope(|scope| {
                    for _ in 0..4 {
                        scope.spawn(|| {
                            for _ in 0..4 {
                                let _ = deep_diff_forge_daemon::process_line(&line, &engine);
                            }
                        });
                    }
                });
            }),
        },
    ]
}

/// The names of every workload, in run order.
#[must_use]
pub fn workload_names() -> Vec<&'static str> {
    workloads().iter().map(|w| w.name).collect()
}

fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

/// Throughput in KiB per second for `bytes` in `ns` nanoseconds, exact in integers (0 for 0 ns).
#[must_use]
pub fn kib_per_s(bytes: u64, ns: u64) -> u64 {
    if ns == 0 {
        return 0;
    }
    u64::try_from(u128::from(bytes) * 1_000_000_000 / u128::from(ns) / 1024).unwrap_or(u64::MAX)
}

/// Nanoseconds as milliseconds with three decimals (`1234567` is `1.234`).
#[must_use]
pub fn millis(ns: u64) -> String {
    format!("{}.{:03}", ns / 1_000_000, (ns / 1_000) % 1_000)
}

/// `values` sorted, then the value at `percent` (nearest rank).
#[must_use]
pub fn percentile(values: &[u64], percent: usize) -> u64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    if sorted.is_empty() {
        return 0;
    }
    let rank = (sorted.len() * percent).div_ceil(100).max(1);
    sorted[rank - 1]
}

/// Run the selected workloads (`only` empty: all), each once to warm up and then `repetitions`
/// times, and return the `deep-diff-forge.bench.v0` receipt.
///
/// # Errors
/// The unknown workload name, when `only` names one.
pub fn run(only: &[String], repetitions: usize) -> Result<Value, String> {
    let all = workloads();
    if let Some(unknown) = only
        .iter()
        .find(|n| !all.iter().any(|w| w.name == n.as_str()))
    {
        return Err(unknown.clone());
    }
    let mut results = Vec::new();
    for workload in all
        .iter()
        .filter(|w| only.is_empty() || only.iter().any(|n| n == w.name))
    {
        (workload.run)(&workload.input);
        let samples: Vec<u64> = (0..repetitions)
            .map(|_| {
                let start = now();
                (workload.run)(&workload.input);
                nanos(start.elapsed())
            })
            .collect();
        let median = percentile(&samples, 50);
        let bytes = workload.input.len() as u64;
        results.push(json!({
            "name": workload.name,
            "input_bytes": bytes,
            "input_sha256": deep_diff_forge_core::sha256_hex(workload.input.as_bytes()),
            "repetitions": repetitions,
            "min_ns": samples.iter().min().copied().unwrap_or(0),
            "median_ns": median,
            "p95_ns": percentile(&samples, 95),
            "kib_per_s": kib_per_s(bytes, median),
        }));
    }
    Ok(json!({
        "schema": "deep-diff-forge.bench.v0",
        "version": env!("CARGO_PKG_VERSION"),
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "host": {
            "cpus": std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        },
        "workloads": results,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_inputs_are_deterministic_and_parse() {
        assert_eq!(generated_patch(3, 2, 5), generated_patch(3, 2, 5));
        let files = review(&generated_patch(3, 2, 5)).files().to_vec();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].patch_twin.hunks.len(), 2);
        // The evidence the patch carries is found.
        let ranked = deep_diff_forge_graph::rank(&files);
        assert!(ranked.iter().all(|r| !r.evidence.is_empty()));
        let (old, new) = generated_sources(10);
        assert_ne!(old, new);
        // A real change, not formatting only, and within the structural budgets.
        let diff = deep_diff_forge_syntax::structural_diff(
            deep_diff_forge_syntax::Language::Rust,
            &old,
            &new,
        );
        assert!(!diff.reformat_only() && !diff.degraded());
    }

    /// Every workload's input bytes are pinned: a change to a generator is a change to what the
    /// benchmark measures, so it must update this list in the same commit, and
    /// `scripts/bench_compare.py` reports such a workload as not compared.
    #[test]
    fn every_workload_input_is_pinned() {
        let pinned = [
            (
                "parse_seal",
                "22da2a2e81a542149830dee499b67b90897ce7ba5cc0a46d509281a253032358",
            ),
            (
                "rank_with_evidence",
                "22da2a2e81a542149830dee499b67b90897ce7ba5cc0a46d509281a253032358",
            ),
            (
                "outline_page",
                "22da2a2e81a542149830dee499b67b90897ce7ba5cc0a46d509281a253032358",
            ),
            (
                "findings_verify",
                "46e7457eb86a17eda75a5305f0611533ed68d370f43f88b3a20e720c2a5ebac0",
            ),
            (
                "findings_by_address",
                "7c075fad8da15a2234c565855959d97548f7153e2dff43dd105dd404c06540c6",
            ),
            (
                "structural_rust",
                "3619701b61b2cbbf72fa3e48d0b1dddc15d8bd515bd3a25b5916235ec10a5433",
            ),
            (
                "daemon_parallel",
                "074d175495ad4020c5800880b499d55c13d5ef129050b80431d07524a51ec3a4",
            ),
        ];
        let actual: Vec<(&str, String)> = workloads()
            .iter()
            .map(|w| (w.name, deep_diff_forge_core::sha256_hex(w.input.as_bytes())))
            .collect();
        let expected: Vec<(&str, String)> =
            pinned.iter().map(|(n, h)| (*n, (*h).to_owned())).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn generated_findings_are_all_accepted() {
        let sealed = review(&generated_patch(5, 2, 20));
        let findings = generated_findings(5, 2, 20);
        let verdict = deep_diff_forge_agent::findings::verify(
            sealed.files(),
            &sealed.seal().input_sha256,
            findings.lines(),
        );
        assert_eq!(
            (verdict.accepted.len(), verdict.refused.len()),
            (5 * 2 * 2, 0)
        );
    }

    #[test]
    fn throughput_and_millis_are_exact() {
        assert_eq!(kib_per_s(1024, 1_000_000_000), 1);
        assert_eq!(kib_per_s(2048, 500_000_000), 4);
        assert_eq!(kib_per_s(1023, 1_000_000_000), 0);
        assert_eq!(kib_per_s(5, 0), 0);
        assert_eq!(kib_per_s(u64::MAX, 1), u64::MAX);
        assert_eq!(millis(1_234_567), "1.234");
        assert_eq!(millis(5_000), "0.005");
        assert_eq!(millis(0), "0.000");
        assert_eq!(millis(12_000_000_000), "12000.000");
    }

    #[test]
    fn percentiles_use_the_nearest_rank() {
        assert_eq!(percentile(&[], 50), 0);
        assert_eq!(percentile(&[7], 50), 7);
        assert_eq!(percentile(&[5, 1, 3, 2, 4], 50), 3);
        assert_eq!(percentile(&[5, 1, 3, 2, 4], 95), 5);
        assert_eq!(percentile(&[5, 1, 3, 2, 4], 1), 1);
        assert_eq!(percentile(&(1..=100).collect::<Vec<_>>(), 95), 95);
    }

    #[test]
    fn a_run_names_every_workload_and_refuses_an_unknown_one() {
        let receipt = run(&["structural_rust".to_owned()], 1).unwrap();
        assert_eq!(receipt["schema"], "deep-diff-forge.bench.v0");
        assert_eq!(
            receipt["profile"],
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            }
        );
        let w = &receipt["workloads"][0];
        assert_eq!(
            (w["name"].as_str(), w["repetitions"].as_u64()),
            (Some("structural_rust"), Some(1))
        );
        assert_eq!(w["input_sha256"].as_str().unwrap().len(), 64);
        assert!(w["min_ns"].as_u64().unwrap() <= w["median_ns"].as_u64().unwrap());
        assert!(w["median_ns"].as_u64().unwrap() <= w["p95_ns"].as_u64().unwrap());
        assert_eq!(receipt["workloads"].as_array().unwrap().len(), 1);
        assert_eq!(run(&["nope".to_owned()], 1).unwrap_err(), "nope");
        assert_eq!(
            workload_names(),
            [
                "parse_seal",
                "rank_with_evidence",
                "outline_page",
                "findings_verify",
                "findings_by_address",
                "structural_rust",
                "daemon_parallel"
            ]
        );
    }
}
