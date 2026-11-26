//! Every third-party action in every workflow is pinned to a full commit SHA, so a moved tag
//! or a compromised release of an action cannot change what CI or the release runs.
//!
//! The rest of `SECURITY.md`'s "Supply chain and releases" that the repository's configuration
//! holds is read here too (lane T3e of 0.11.1): the workflows' grants, checkout credentials, job
//! time limits and toolchains; the exact pins of the tree-sitter crates; the TUI's ratatui
//! features; no advisory waiver; and what Dependabot watches. Each is read line by line from the
//! file's fixed layout (no YAML or TOML parser, as the readers above).

use std::path::Path;

/// The `uses:` targets of a workflow, with the line each is on.
fn uses(workflow: &str) -> Vec<(usize, &str)> {
    workflow
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let rest = line
                .trim_start()
                .strip_prefix("- ")
                .unwrap_or(line.trim_start());
            let target = rest.strip_prefix("uses:")?.trim();
            Some((i + 1, target.split_whitespace().next().unwrap_or_default()))
        })
        .collect()
}

/// A target that is local (`./…`) or `owner/repo[/path]@<40 lowercase hex>`.
fn pinned(target: &str) -> bool {
    if target.starts_with("./") {
        return true;
    }
    let Some((action, sha)) = target.split_once('@') else {
        return false;
    };
    action.contains('/')
        && sha.len() == 40
        && sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[test]
fn every_action_is_pinned_to_a_commit() {
    let dir = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../.github/workflows"
    ));
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "yml" && e != "yaml") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for (line, target) in uses(&text) {
            seen += 1;
            assert!(
                pinned(target),
                "{}:{line}: `{target}` is not pinned to a commit SHA",
                path.display()
            );
        }
    }
    assert!(
        seen > 30,
        "only {seen} `uses:` lines found: the reader is broken"
    );
}

#[test]
fn the_pin_reader_is_exact() {
    let sha = "24c54180a607b1449ed407dd24f251e4e9147c8d";
    assert!(pinned(&format!("actions/checkout@{sha}")));
    assert!(pinned(&format!("github/codeql-action/init@{sha}")));
    assert!(pinned("./.github/workflows/ci.yml"));
    for bad in [
        "actions/checkout@v4".to_owned(),
        "actions/checkout@main".to_owned(),
        format!("actions/checkout@{}", &sha[..39]),
        format!("actions/checkout@{sha}0"),
        format!("actions/checkout@{}", sha.to_uppercase()),
        format!("checkout@{sha}"),
        "actions/checkout".to_owned(),
    ] {
        assert!(!pinned(&bad), "{bad}");
    }
    let text = "steps:\n  - uses: a/b@x # c\n    with:\n  - name: n\n    uses: ./local\n";
    assert_eq!(uses(text), [(2, "a/b@x"), (5, "./local")]);
}

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn read(path: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{path}")).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Every workflow, as (file name, text), sorted by name.
fn workflows() -> Vec<(String, String)> {
    let mut all: Vec<(String, String)> = std::fs::read_dir(format!("{ROOT}/.github/workflows"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "yml" || e == "yaml"))
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read_to_string(&path).unwrap())
        })
        .collect();
    all.sort();
    all
}

/// `SECURITY.md` with its line wrapping undone.
fn security_words() -> String {
    read("SECURITY.md")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// --- grants, checkout credentials, time limits (SECURITY.md: "use least-privilege job
// permissions …, do not persist checkout credentials, and have bounded run times") -----------

/// One job of a workflow: whether it calls a reusable workflow (a job-level `uses:`, which takes
/// no `timeout-minutes:`), its job-level time limit and grants, and its steps, each as its lines.
#[derive(Debug, Default)]
struct WorkflowJob {
    name: String,
    calls: bool,
    timeout: Option<String>,
    grants: Vec<String>,
    steps: Vec<Vec<String>>,
}

/// The workflow-level grants and the jobs, from the fixed two-space layout. Comments, a line's
/// trailing ` #` comment and blank lines are left out.
fn read_workflow(workflow: &str) -> (Vec<String>, Vec<WorkflowJob>) {
    let mut top = Vec::new();
    let mut jobs: Vec<WorkflowJob> = Vec::new();
    let mut section = "";
    let mut in_grants = false;
    for line in workflow.lines() {
        let code = line.split(" #").next().unwrap_or_default().trim_end();
        if code.trim_start().is_empty() || code.trim_start().starts_with('#') {
            continue;
        }
        if !code.starts_with(' ') {
            section = code.split(':').next().unwrap_or_default();
            if let Some(value) = code.strip_prefix("permissions:")
                && !value.trim().is_empty()
            {
                top.push(value.trim().to_owned());
            }
            continue;
        }
        if section == "permissions" {
            top.push(code.trim().to_owned());
            continue;
        }
        if section != "jobs" {
            continue;
        }
        if let Some(name) = code
            .strip_prefix("  ")
            .and_then(|rest| rest.strip_suffix(':'))
            .filter(|key| !key.starts_with(' ') && !key.contains(' '))
        {
            jobs.push(WorkflowJob {
                name: name.to_owned(),
                ..WorkflowJob::default()
            });
            in_grants = false;
            continue;
        }
        let Some(job) = jobs.last_mut() else { continue };
        if let Some(key) = code.strip_prefix("    ").filter(|k| !k.starts_with(' ')) {
            in_grants = key == "permissions:";
            if let Some(value) = key.strip_prefix("permissions:")
                && !value.trim().is_empty()
            {
                job.grants.push(value.trim().to_owned());
            }
            job.calls |= key.starts_with("uses:");
            if let Some(value) = key.strip_prefix("timeout-minutes:") {
                job.timeout = Some(value.trim().to_owned());
            }
            continue;
        }
        if in_grants
            && let Some(grant) = code.strip_prefix("      ").filter(|g| !g.starts_with(' '))
        {
            job.grants.push(grant.trim().to_owned());
            continue;
        }
        if code.starts_with("      - ") {
            job.steps.push(Vec::new());
        }
        if let Some(step) = job.steps.last_mut() {
            step.push(code.to_owned());
        }
    }
    (top, jobs)
}

/// The write grants each workflow's jobs may hold, reviewed: in `release.yml` only the tag-only
/// `release` job, which runs no project code (`release_workflow.rs`); in `codeql.yml` the upload
/// of its findings. No other job holds any.
const WRITE_GRANTS: &[(&str, &str, &[&str])] = &[
    (
        "release.yml",
        "release",
        &["contents: write", "attestations: write", "id-token: write"],
    ),
    ("codeql.yml", "analyze", &["security-events: write"]),
];

/// Holds the grants, checkout and time-limit clauses of SECURITY.md's sentence on GitHub Actions
/// for every workflow (the SHA pins are `every_action_is_pinned_to_a_commit`).
#[test]
fn every_workflow_grants_least_privilege_keeps_no_checkout_credential_and_bounds_each_job() {
    let (mut jobs_read, mut checkouts) = (0, 0);
    for (name, text) in workflows() {
        let (top, jobs) = read_workflow(&text);
        assert_eq!(
            top,
            ["contents: read"],
            "{name}: the workflow-level grants, which every job without its own block holds"
        );
        for job in &jobs {
            jobs_read += 1;
            let writes: Vec<&str> = job
                .grants
                .iter()
                .map(String::as_str)
                .filter(|grant| grant.ends_with(": write") || *grant == "write-all")
                .collect();
            let reviewed = WRITE_GRANTS
                .iter()
                .find(|(file, job_name, _)| *file == name && *job_name == job.name)
                .map_or(&[][..], |(_, _, grants)| *grants);
            assert_eq!(
                writes, reviewed,
                "{name}: job `{}` holds the write grants {writes:?}, where the reviewed set is {reviewed:?}",
                job.name
            );
            assert!(
                job.calls || job.timeout.is_some(),
                "{name}: job `{}` has no job-level timeout-minutes: its run time is not bounded",
                job.name
            );
            for step in &job.steps {
                let checkout = step.iter().any(|line| {
                    let rest = line.trim_start();
                    rest.strip_prefix("- ")
                        .unwrap_or(rest)
                        .starts_with("uses: actions/checkout@")
                });
                if checkout {
                    checkouts += 1;
                    assert!(
                        step.iter()
                            .any(|line| line.trim() == "persist-credentials: false"),
                        "{name}: job `{}` checks out with persisted credentials:\n{}",
                        job.name,
                        step.join("\n")
                    );
                }
            }
        }
    }
    assert!(
        jobs_read > 15 && checkouts > 15,
        "only {jobs_read} jobs and {checkouts} checkouts read: the reader is broken"
    );
}

#[test]
fn the_workflow_reader_reads_grants_limits_and_steps() {
    let workflow = "on:\n  push:\npermissions:\n  contents: read # x\njobs:\n  a:\n    timeout-minutes: 5\n    permissions:\n      contents: write\n      id-token: write\n    steps:\n      - uses: actions/checkout@x\n        with:\n          persist-credentials: false\n      - run: true\n  b:\n    uses: ./.github/workflows/ci.yml\n    permissions: write-all\n";
    let (top, jobs) = read_workflow(workflow);
    assert_eq!(top, ["contents: read"]);
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[0].grants, ["contents: write", "id-token: write"]);
    assert_eq!(jobs[0].timeout.as_deref(), Some("5"));
    assert_eq!(jobs[0].steps.len(), 2);
    assert!(jobs[0].steps[0][2].contains("persist-credentials: false"));
    assert!(jobs[1].calls && jobs[1].timeout.is_none());
    assert_eq!(jobs[1].grants, ["write-all"]);
    assert_eq!(
        read_workflow("permissions: read-all\njobs:\n").0,
        ["read-all"]
    );
}

// --- toolchains (SECURITY.md: "CI and release toolchains are pinned") -------------------------

/// SECURITY.md's sentence on toolchains, and the two it names.
const TOOLCHAIN_SENTENCE: &str = "CI and release toolchains are pinned (stable 1.99.1, `nightly-2026-10-25`), so a toolchain release cannot change a gate's result.";
const STABLE: &str = "1.99.1";
const NIGHTLY: &str = "nightly-2026-10-25";

/// Every toolchain a workflow, script or recipe names: the `toolchain:` input of
/// dtolnay/rust-toolchain, `rustup toolchain install X`, `--toolchain X`, `cargo +X`,
/// `toolchain=X`, `RUSTUP_TOOLCHAIN=X` and a `${…TOOLCHAIN:-X}` default. A variable (`$…`) is not
/// a name and is skipped; comment lines are left out.
fn toolchains(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in text.lines() {
        let code = line.split(" #").next().unwrap_or_default();
        if code.trim_start().starts_with('#') {
            continue;
        }
        for marker in [
            "toolchain: ",
            "toolchain install ",
            "--toolchain ",
            "cargo +",
            "cargo \"+",
            "toolchain=",
            "TOOLCHAIN=",
            "TOOLCHAIN:-",
        ] {
            for (at, _) in code.match_indices(marker) {
                let rest = code[at + marker.len()..].trim_start_matches(['"', '\'']);
                let word: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
                    .collect();
                if !word.is_empty() {
                    found.push(word);
                }
            }
        }
    }
    found
}

#[test]
fn every_ci_and_release_toolchain_is_one_security_md_names() {
    assert!(
        security_words().contains(TOOLCHAIN_SENTENCE),
        "SECURITY.md no longer says {TOOLCHAIN_SENTENCE:?}: hold this check to what it says now"
    );
    // The declared MSRV is checked on its own fixed release (ci.yml's `msrv`, the replica's
    // `cargo +1.88.0`): a pin too, not a floating channel.
    let manifest = read("Cargo.toml");
    let msrv = manifest
        .lines()
        .find_map(|line| line.strip_prefix("rust-version = \""))
        .and_then(|rest| rest.strip_suffix('"'))
        .map(|version| format!("{version}.0"))
        .expect("the workspace's rust-version");
    for file in ["rust-toolchain", "rust-toolchain.toml"] {
        for dir in ["", "fuzz/"] {
            assert!(
                !Path::new(&format!("{ROOT}/{dir}{file}")).exists(),
                "{dir}{file} overrides every pin below"
            );
        }
    }
    let mut texts = workflows();
    for dir in ["scripts", "scripts/security"] {
        for entry in std::fs::read_dir(format!("{ROOT}/{dir}")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "sh" || e == "py") {
                texts.push((
                    path.display().to_string(),
                    std::fs::read_to_string(&path).unwrap(),
                ));
            }
        }
    }
    texts.push(("justfile".to_owned(), read("justfile")));
    let mut seen = Vec::new();
    for (name, text) in &texts {
        for toolchain in toolchains(text) {
            assert!(
                [STABLE, NIGHTLY, msrv.as_str()].contains(&toolchain.as_str()),
                "{name} names the toolchain `{toolchain}`, which is not {STABLE}, {NIGHTLY} or the MSRV {msrv}"
            );
            seen.push(toolchain);
        }
        let actions = text
            .lines()
            .filter(|line| line.contains("uses: dtolnay/rust-toolchain@"))
            .count();
        let inputs = text
            .lines()
            .filter(|line| line.trim_start().starts_with("toolchain: "))
            .count();
        assert_eq!(
            actions, inputs,
            "{name}: a dtolnay/rust-toolchain step without its own `toolchain:` input"
        );
    }
    for pin in [STABLE, NIGHTLY] {
        assert!(
            seen.iter().any(|name| name == pin),
            "nothing names {pin}: the reader is broken"
        );
    }
    assert!(
        seen.len() > 15,
        "only {} toolchains read: the reader is broken",
        seen.len()
    );
}

#[test]
fn the_toolchain_reader_finds_every_form() {
    let text = "      - uses: dtolnay/rust-toolchain@x\n        with:\n          toolchain: stable\n# toolchain: commented\nrustup toolchain install nightly --profile minimal\ncargo +beta check\ntoolchain=1.2.3\nRUSTUP_TOOLCHAIN=\"nightly-2020-01-01\" cargo x\nt=\"${DDF_FUZZ_TOOLCHAIN:-nightly-2026-10-25}\"\ncargo \"+$toolchain\" fuzz\nrustup which --toolchain \"$toolchain\" cargo\n";
    assert_eq!(
        toolchains(text),
        [
            "stable",
            "nightly",
            "beta",
            "1.2.3",
            "nightly-2020-01-01",
            "nightly-2026-10-25"
        ]
    );
}

// --- the tree-sitter pins and the TUI's features (SECURITY.md: "Tree-sitter crates, which execute
// native build scripts, are pinned exactly. The TUI disables unused default features.") --------

/// Every workspace manifest and the fuzz harness's: (path, text).
fn manifests() -> Vec<(String, String)> {
    let mut all = vec![
        ("Cargo.toml".to_owned(), read("Cargo.toml")),
        ("fuzz/Cargo.toml".to_owned(), read("fuzz/Cargo.toml")),
    ];
    let mut crates: Vec<_> = std::fs::read_dir(format!("{ROOT}/crates"))
        .unwrap()
        .map(|entry| entry.unwrap().path().join("Cargo.toml"))
        .filter(|path| path.exists())
        .collect();
    crates.sort();
    for path in crates {
        let text = std::fs::read_to_string(&path).unwrap();
        all.push((path.display().to_string(), text));
    }
    all
}

/// The dependencies of a manifest whose name starts with `prefix`, from any table whose header
/// names dependencies: (name, the line that declares it, or the lines of its own table).
fn dependencies(manifest: &str, prefix: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    let mut in_deps = false;
    let mut own: Option<usize> = None;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
            in_deps = header.contains("dependencies");
            own = None;
            // `[dependencies.<name>]`, `[target.'…'.dependencies.<name>]`
            if let Some((_, name)) = header.rsplit_once("dependencies.")
                && name.starts_with(prefix)
            {
                found.push((name.to_owned(), String::new()));
                own = Some(found.len() - 1);
            }
            continue;
        }
        if let Some(at) = own {
            found[at].1.push_str(line);
            found[at].1.push('\n');
        } else if in_deps
            && let Some((name, _)) = line.split_once('=')
            && name.trim().starts_with(prefix)
        {
            found.push((name.trim().to_owned(), line.to_owned()));
        }
    }
    found
}

/// The version requirement of a dependency's declaration: `"req"` or `version = "req"`.
fn requirement(declaration: &str) -> Option<&str> {
    let value = declaration
        .split_once("version = \"")
        .map(|(_, rest)| rest)
        .or_else(|| declaration.split_once("= \"").map(|(_, rest)| rest))?;
    value.split('"').next()
}

/// `=MAJOR.MINOR.PATCH`: the one release a requirement admits.
fn exact(requirement: &str) -> bool {
    requirement.strip_prefix('=').is_some_and(|version| {
        let parts: Vec<&str> = version.split('.').collect();
        parts.len() == 3
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// The versions a Cargo.lock records for `name`.
fn locked(lock: &str, name: &str) -> Vec<String> {
    lock.split("[[package]]")
        .filter(|entry| {
            entry
                .lines()
                .any(|line| line == format!("name = \"{name}\""))
        })
        .filter_map(|entry| {
            entry
                .lines()
                .find_map(|line| line.strip_prefix("version = \""))
                .map(|v| v.trim_end_matches('"').to_owned())
        })
        .collect()
}

#[test]
fn every_tree_sitter_dependency_is_pinned_to_one_release() {
    let locks = [
        ("Cargo.lock", read("Cargo.lock")),
        ("fuzz/Cargo.lock", read("fuzz/Cargo.lock")),
    ];
    let mut pinned = 0;
    for (path, manifest) in manifests() {
        for (name, declaration) in dependencies(&manifest, "tree-sitter") {
            // `x.workspace = true` or `x = { workspace = true }`: the root's pin, read there.
            if name.ends_with(".workspace") || declaration.contains("workspace = true") {
                continue;
            }
            let wanted = requirement(&declaration);
            assert!(
                wanted.is_some_and(exact),
                "{path}: `{name}` is not pinned to one release (`=x.y.z`): {declaration:?}"
            );
            let version = &wanted.unwrap()[1..];
            for (lock_path, lock) in &locks {
                for got in locked(lock, &name) {
                    assert_eq!(
                        got, version,
                        "{lock_path} resolves `{name}` to {got}, where {path} pins ={version}"
                    );
                }
            }
            pinned += 1;
        }
    }
    assert!(
        pinned >= 2,
        "only {pinned} tree-sitter dependencies read: the reader is broken"
    );
}

/// The features a dependency declaration lists (`features = ["a", "b"]`), sorted.
fn features(declaration: &str) -> Vec<String> {
    let mut list: Vec<String> = declaration
        .split_once("features = [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(inside, _)| {
            inside
                .split(',')
                .map(|f| f.trim().trim_matches('"').to_owned())
                .filter(|f| !f.is_empty())
                .collect()
        })
        .unwrap_or_default();
    list.sort();
    list
}

/// The features the TUI takes from ratatui, reviewed: its crossterm 0.29 backend and the layout
/// cache. ratatui's defaults (all widgets, macros, underline colour, its own crossterm choice)
/// stay off. Every ratatui dependency in the workspace says so, since features unify: one with
/// the defaults would turn them on for the TUI too.
const RATATUI_FEATURES: [&str; 2] = ["crossterm_0_29", "layout-cache"];

#[test]
fn the_tui_takes_only_the_reviewed_ratatui_features() {
    let mut read_tui = false;
    for (path, manifest) in manifests() {
        for (name, declaration) in dependencies(&manifest, "ratatui") {
            assert!(
                declaration.contains("default-features = false"),
                "{path}: `{name}` keeps ratatui's default features: {declaration:?}"
            );
            assert_eq!(
                features(&declaration),
                RATATUI_FEATURES,
                "{path}: `{name}` takes features other than the reviewed ones"
            );
            read_tui |= path.ends_with("deep-diff-forge-tui/Cargo.toml");
        }
    }
    assert!(
        read_tui,
        "no ratatui dependency of the TUI read: the reader is broken"
    );
}

#[test]
fn the_manifest_readers_read_both_forms() {
    let manifest = "[package]\nname = \"x\"\n[dependencies]\n# tree-sitter = \"0.1\"\ntree-sitter = \"=0.26.13\"\ntree-sitter-rust = { version = \"0.24\", features = [\"a\"] }\nratatui = { version = \"0.30\", default-features = false, features = [\"b\", \"a\"] }\n[target.'cfg(unix)'.dependencies.tree-sitter-x]\nversion = \"=1.2\"\n[dev-dependencies]\ntree-sitter-y = \"=1.2.3\"\n";
    let found = dependencies(manifest, "tree-sitter");
    let names: Vec<&str> = found.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "tree-sitter",
            "tree-sitter-rust",
            "tree-sitter-x",
            "tree-sitter-y"
        ]
    );
    let requirements: Vec<Option<&str>> = found.iter().map(|(_, d)| requirement(d)).collect();
    assert_eq!(
        requirements,
        [Some("=0.26.13"), Some("0.24"), Some("=1.2"), Some("=1.2.3")]
    );
    let pinned: Vec<bool> = requirements.iter().map(|r| r.is_some_and(exact)).collect();
    assert_eq!(pinned, [true, false, false, true]);
    assert!(!exact("=0.26") && !exact("=0.26.x") && !exact("^0.26.13") && !exact("0.26.13"));
    let ratatui = dependencies(manifest, "ratatui");
    assert_eq!(features(&ratatui[0].1), ["a", "b"]);
    let lock = "[[package]]\nname = \"a\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"ab\"\nversion = \"2.0.0\"\n";
    assert_eq!(locked(lock, "a"), ["1.0.0"]);
}

// --- no advisory waivers (SECURITY.md: "There are no advisory waivers.") ----------------------

/// Where cargo-deny and cargo-audit would read a second configuration: cargo-deny looks for
/// `deny.toml`, `.deny.toml` or `.cargo/deny.toml` from the manifest's directory up (the fuzz
/// harness's run reads the root's today), cargo-audit for `.cargo/audit.toml` in the checkout.
const OTHER_ADVISORY_CONFIGS: [&str; 7] = [
    ".deny.toml",
    ".cargo/deny.toml",
    ".cargo/audit.toml",
    "fuzz/deny.toml",
    "fuzz/.deny.toml",
    "fuzz/.cargo/deny.toml",
    "fuzz/.cargo/audit.toml",
];

/// The value of `ignore =` in the `[advisories]` table of a cargo-deny configuration.
fn advisory_ignores(deny: &str) -> Option<String> {
    let mut in_advisories = false;
    for line in deny.lines().map(str::trim) {
        if line.starts_with('[') {
            in_advisories = line == "[advisories]";
        } else if in_advisories
            && let Some(value) = line.strip_prefix("ignore")
            && let Some(value) = value.trim_start().strip_prefix('=')
        {
            return Some(
                value
                    .split(" #")
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned(),
            );
        }
    }
    None
}

#[test]
fn no_advisory_is_waived() {
    let deny = read("deny.toml");
    assert_eq!(
        advisory_ignores(&deny).as_deref(),
        Some("[]"),
        "deny.toml's [advisories] ignore list must stay empty"
    );
    for path in OTHER_ADVISORY_CONFIGS {
        assert!(
            !Path::new(&format!("{ROOT}/{path}")).exists(),
            "{path} is a second advisory configuration, where a waiver could sit"
        );
    }
    let mut runs = 0;
    let mut texts = workflows();
    texts.push((
        "scripts/ci_local.sh".to_owned(),
        read("scripts/ci_local.sh"),
    ));
    for (name, text) in &texts {
        for line in text.lines() {
            if line.trim_start().starts_with('#')
                || !(line.contains("cargo audit") || line.contains("cargo deny"))
            {
                continue;
            }
            runs += 1;
            assert!(
                !line.contains("--ignore"),
                "{name}: an advisory run waives an advisory: {}",
                line.trim()
            );
        }
    }
    assert!(
        runs >= 10,
        "only {runs} advisory runs read: the reader is broken"
    );
}

#[test]
fn the_advisory_reader_reads_the_ignore_list() {
    assert_eq!(
        advisory_ignores(
            "[advisories]\nversion = 2\nignore = [] # none\n[bans]\nignore = [\"x\"]\n"
        )
        .as_deref(),
        Some("[]")
    );
    assert_eq!(
        advisory_ignores("[advisories]\nignore = [\n  \"RUSTSEC-2026-0002\",\n]\n").as_deref(),
        Some("[")
    );
    assert_eq!(advisory_ignores("[bans]\nignore = []\n"), None);
}

// --- Dependabot (SECURITY.md: "Dependabot monitors both Cargo graphs, the Python client's build
// requirement and Actions.") ------------------------------------------------------------------

/// The (ecosystem, directory) of each `updates:` entry of `.github/dependabot.yml`, and every
/// ignored dependency name.
fn dependabot(config: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut ignored = Vec::new();
    for line in config.lines() {
        let code = line.split(" #").next().unwrap_or_default();
        if let Some(ecosystem) = code.strip_prefix("  - package-ecosystem: ") {
            entries.push((ecosystem.trim().to_owned(), String::new()));
        } else if let Some(directory) = code.strip_prefix("    directory: ")
            && let Some(entry) = entries.last_mut()
        {
            entry.1 = directory.trim().trim_matches('"').to_owned();
        } else if let Some(name) = code.trim_start().strip_prefix("- dependency-name: ") {
            ignored.push(name.trim().trim_matches('"').to_owned());
        }
    }
    (entries, ignored)
}

#[test]
fn dependabot_watches_both_cargo_graphs_the_python_build_requirement_and_actions() {
    let (entries, ignored) = dependabot(&read(".github/dependabot.yml"));
    let mut entries = entries;
    entries.sort();
    let want = [
        ("cargo", "/"),
        ("cargo", "/fuzz"),
        ("github-actions", "/"),
        ("pip", "/sdk/python"),
    ];
    assert_eq!(
        entries,
        want.map(|(e, d)| (e.to_owned(), d.to_owned())),
        "Dependabot's ecosystems and directories"
    );
    // Each directory holds what its ecosystem reads: the two lockfiles, the Python client's
    // build requirement.
    for path in ["Cargo.lock", "fuzz/Cargo.lock"] {
        assert!(Path::new(&format!("{ROOT}/{path}")).exists(), "{path}");
    }
    let pyproject = read("sdk/python/pyproject.toml");
    assert!(
        pyproject.contains("[build-system]\nrequires = [\"setuptools"),
        "sdk/python/pyproject.toml's build requirement"
    );
    assert!(
        !ignored.iter().any(|name| name.contains('*')),
        "an ignore rule that matches every dependency stops the watch: {ignored:?}"
    );
}
