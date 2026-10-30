//! `deep-diff-forge bench` end to end (plan 2026-10-30, M2): the receipt's shape, the
//! repetition count `--quick` sets, the table, and the refusal of an unknown workload. The
//! workload with the cheapest input keeps this fast in a debug build.

#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives the real binary as a child process"
)]

use serde_json::Value;
use std::process::{Command, Output};

const CHEAP: &str = "findings_verify";

fn bench(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .arg("bench")
        .args(args)
        .output()
        .expect("run")
}

fn receipt(args: &[&str]) -> Value {
    let out = bench(args);
    assert!(out.status.success(), "{out:?}");
    serde_json::from_slice(&out.stdout).expect("a JSON receipt")
}

#[test]
fn quick_runs_five_repetitions_and_the_default_fifteen() {
    for (args, repetitions) in [
        (&["--quick", "--workload", CHEAP, "--json"][..], 5),
        (&["--workload", CHEAP, "--json"][..], 15),
    ] {
        let r = receipt(args);
        assert_eq!(r["schema"], "deep-diff-forge.bench.v0");
        let workloads = r["workloads"].as_array().unwrap();
        assert_eq!(workloads.len(), 1, "{r:#}");
        assert_eq!(workloads[0]["name"], CHEAP);
        assert_eq!(workloads[0]["repetitions"], repetitions, "{args:?}");
    }
}

#[test]
fn the_table_names_the_build_and_each_workload() {
    let out = bench(&["--quick", "--workload", CHEAP]);
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    assert!(
        text.starts_with(&format!(
            "deep-diff-forge bench ({profile} build, 5 repetitions)\n"
        )),
        "{text}"
    );
    let row = text.lines().nth(1).unwrap();
    assert!(row.trim_start().starts_with(CHEAP), "{text}");
    assert!(
        row.contains(" ms  p95 ") && row.ends_with(" KiB/s"),
        "{row}"
    );
    assert_eq!(text.lines().count(), 2, "{text}");
}

#[test]
fn an_unknown_workload_is_a_usage_error_naming_the_choices() {
    let out = bench(&["--workload", "nope"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty(), "{out:?}");
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("no workload nope"), "{err}");
    assert!(
        err.contains(CHEAP) && err.contains("daemon_parallel"),
        "{err}"
    );
}
