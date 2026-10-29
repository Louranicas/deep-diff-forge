// DDF review route: DDF-R12, DDF-R13. Status and remaining work are recorded in the linked ledger; this comment does not claim delivery.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#remaining-module-routes
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcrates--deep-diff-forge-graph--src--risk
use deep_diff_forge_core::{FileStatus, PatchLineKind, ReviewFile};

/// A deterministic, explainable reason a file is ranked where it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskSignal {
    /// Touches a likely public API surface (`lib.rs`, `mod.rs`, an `api/` path).
    PublicApiSurface,
    /// A newly added file.
    NewFile,
    /// A deleted file.
    DeletedFile,
    /// A binary change (no reviewable text).
    BinaryChange,
    /// A large change by added+removed line count.
    LargeChange,
    /// Many hunks in one file.
    ManyHunks,
    /// A configuration or lockfile change.
    ConfigOrLockfile,
    /// Test-only change (lowers priority).
    TestOnly,
    /// Generated or vendored path: no bonuses, but never below the base weight.
    GeneratedOrVendored,
    /// The path or a changed line holds bidi controls, invisible or control characters, or the
    /// path traverses (`..`, absolute) or names `.git`: what a reviewer sees may differ from
    /// what is applied (Trojan Source, CVE-2021-42574).
    HazardousText,
    /// A symlink is created, changed, removed or made from a file: its target can point
    /// anywhere, and a later write through it lands there.
    SymlinkChange,
    /// A file becomes executable (a new `100755` file, or a mode change that adds an `x` bit).
    ExecutableBit,
}

impl RiskSignal {
    /// Stable snake-case label for machine output.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::PublicApiSurface => "public_api_surface",
            Self::NewFile => "new_file",
            Self::DeletedFile => "deleted_file",
            Self::BinaryChange => "binary_change",
            Self::LargeChange => "large_change",
            Self::ManyHunks => "many_hunks",
            Self::ConfigOrLockfile => "config_or_lockfile",
            Self::TestOnly => "test_only",
            Self::GeneratedOrVendored => "generated_or_vendored",
            Self::HazardousText => "hazardous_text",
            Self::SymlinkChange => "symlink_change",
            Self::ExecutableBit => "executable_bit",
        }
    }
}

/// Lines (added+removed) at or above which a change counts as large.
pub const LARGE_CHANGE_LINES: usize = 80;
/// Hunk count at or above which a file counts as many-hunked.
pub const MANY_HUNKS: usize = 5;

/// Compute the risk score and contributing signals for one file.
///
/// Scores are deterministic and additive; test-only files are de-prioritized.
///
/// A generated or vendored path earns no bonus for being source, public API, large or
/// many-hunked, but it keeps the base weight and its new/deleted/binary signals. The path
/// is chosen by whoever wrote the patch, so it may lower a file's priority but never hide
/// it: before, a new `src/generated/x.rs` that spawned a process scored 0 and ranked below
/// a one-line README edit.
#[must_use]
pub fn score_file(file: &ReviewFile) -> (u32, Vec<RiskSignal>) {
    let path = &file.path;
    let mut signals = Vec::new();
    let mut score: u32 = 1; // base weight for any reviewable change
    if is_generated_or_vendored(path) {
        signals.push(RiskSignal::GeneratedOrVendored);
        score += status_weight(file, &mut signals);
        score += hazard_weight(file, &mut signals);
        score += mode_weight(file, &mut signals);
        return (score, signals);
    }

    let test = is_test(path);
    let config = is_config_or_lockfile(path);
    // Primary-source bonus: ranks reviewable source above tests, config and
    // generated/vendored files (handled above, without bonuses).
    if !test && !config {
        score += 1;
    }

    if is_public_api(path) {
        signals.push(RiskSignal::PublicApiSurface);
        score += 5;
    }
    if test {
        signals.push(RiskSignal::TestOnly);
    } else if config {
        signals.push(RiskSignal::ConfigOrLockfile);
        score += 1;
    }

    score += status_weight(file, &mut signals);
    score += hazard_weight(file, &mut signals);
    score += mode_weight(file, &mut signals);

    let (adds, dels) = change_counts(file);
    let hunks = file.patch_twin.hunks.len();
    if adds + dels >= LARGE_CHANGE_LINES {
        signals.push(RiskSignal::LargeChange);
        score += 3;
    }
    if hunks >= MANY_HUNKS {
        signals.push(RiskSignal::ManyHunks);
        score += 2;
    }

    (score, signals)
}

/// +3 when the path or a changed (added or removed) line carries an objective hazard, recording
/// the signal. Context lines are not counted: they are not part of the change.
fn hazard_weight(file: &ReviewFile, signals: &mut Vec<RiskSignal>) -> u32 {
    let in_lines = file
        .patch_twin
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .any(|line| {
            line.kind != PatchLineKind::Context
                && !deep_diff_forge_core::text_hazards(&line.text).is_empty()
        });
    // A rename or copy is as hazardous as the path it leaves: moving `.git/hooks/pre-commit` to
    // `src/y.rs` scored nothing while the reverse rename was flagged (bug hunt 2026-10-28, H4-07).
    let from_hazardous = file
        .patch_twin
        .metadata
        .iter()
        .filter_map(|m| {
            m.strip_prefix("rename from ")
                .or_else(|| m.strip_prefix("copy from "))
        })
        .any(|from| !deep_diff_forge_core::path_hazards(from).is_empty());
    if in_lines || from_hazardous || !deep_diff_forge_core::path_hazards(&file.path).is_empty() {
        signals.push(RiskSignal::HazardousText);
        3
    } else {
        0
    }
}

/// +3 for a symlink and +2 for a file made executable, recording each signal (bug hunt
/// 2026-10-28, H4-07). Modes come from the extended headers, which the parser has checked are
/// octal.
fn mode_weight(file: &ReviewFile, signals: &mut Vec<RiskSignal>) -> u32 {
    const SYMLINK: u32 = 0o120_000;
    const REGULAR: u32 = 0o100_000;
    let modes = |prefixes: &[&str]| -> Vec<u32> {
        file.patch_twin
            .metadata
            .iter()
            .filter_map(|m| prefixes.iter().find_map(|p| m.strip_prefix(p)))
            .filter_map(|mode| u32::from_str_radix(mode, 8).ok())
            .collect()
    };
    let old = modes(&["old mode ", "deleted file mode "]);
    let new = modes(&["new mode ", "new file mode "]);
    let executable = |mode: &u32| mode & 0o170_000 == REGULAR && mode & 0o111 != 0;
    let mut weight = 0;
    if old
        .iter()
        .chain(&new)
        .any(|mode| mode & 0o170_000 == SYMLINK)
    {
        signals.push(RiskSignal::SymlinkChange);
        weight += 3;
    }
    if new.iter().any(executable) && !old.iter().any(executable) {
        signals.push(RiskSignal::ExecutableBit);
        weight += 2;
    }
    weight
}

/// The weight of a file's status (added, deleted, binary), recording its signal.
fn status_weight(file: &ReviewFile, signals: &mut Vec<RiskSignal>) -> u32 {
    let (signal, weight) = match file.status {
        FileStatus::Added => (RiskSignal::NewFile, 2),
        FileStatus::Deleted => (RiskSignal::DeletedFile, 3),
        FileStatus::BinaryChanged => (RiskSignal::BinaryChange, 1),
        _ => return 0,
    };
    signals.push(signal);
    weight
}

/// Count added and removed lines across a file's hunks.
#[must_use]
pub fn change_counts(file: &ReviewFile) -> (usize, usize) {
    let mut adds = 0;
    let mut dels = 0;
    for hunk in &file.patch_twin.hunks {
        for line in &hunk.lines {
            match line.kind {
                PatchLineKind::Added => adds += 1,
                PatchLineKind::Removed => dels += 1,
                PatchLineKind::Context => {}
            }
        }
    }
    (adds, dels)
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn is_public_api(path: &str) -> bool {
    matches!(basename(path), "lib.rs" | "mod.rs") || path.contains("/api/")
}

fn is_test(path: &str) -> bool {
    path.contains("/tests/")
        || path.starts_with("tests/")
        || path.contains("/test/")
        || basename(path).contains("_test.")
        || basename(path).contains(".test.")
}

fn is_config_or_lockfile(path: &str) -> bool {
    let base = basename(path);
    matches!(
        base,
        "Cargo.toml"
            | "Cargo.lock"
            | "package.json"
            | "package-lock.json"
            | "deny.toml"
            | "rustfmt.toml"
    ) || matches!(
        base.rsplit('.').next(),
        Some("toml" | "yaml" | "yml" | "ini" | "cfg" | "lock")
    )
}

fn is_generated_or_vendored(path: &str) -> bool {
    const DIRS: [&str; 5] = ["vendor", "node_modules", "target", "generated", "dist"];
    const SUFFIXES: [&str; 3] = [".min.js", ".pb.rs", ".pb.go"];
    path.split('/').any(|c| DIRS.contains(&c)) || SUFFIXES.iter().any(|s| path.ends_with(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_patch::parse;

    fn file(input: &str) -> ReviewFile {
        parse(input).unwrap().into_iter().next().unwrap()
    }

    fn modify(path: &str, body_lines: usize) -> ReviewFile {
        use std::fmt::Write as _;
        let n = body_lines.max(1);
        let mut s = String::new();
        let _ = write!(s, "--- a/{path}\n+++ b/{path}\n@@ -1,{n} +1,{n} @@\n");
        for i in 0..body_lines {
            let _ = write!(s, "-old{i}\n+new{i}\n");
        }
        file(&s)
    }

    #[test]
    fn lib_rs_is_public_api() {
        let (_, signals) = score_file(&modify("src/lib.rs", 1));
        assert!(signals.contains(&RiskSignal::PublicApiSurface));
    }

    #[test]
    fn mod_rs_is_public_api() {
        let (_, signals) = score_file(&modify("src/foo/mod.rs", 1));
        assert!(signals.contains(&RiskSignal::PublicApiSurface));
    }

    #[test]
    fn api_path_is_public_api() {
        let (_, signals) = score_file(&modify("src/api/users.rs", 1));
        assert!(signals.contains(&RiskSignal::PublicApiSurface));
    }

    #[test]
    fn plain_source_is_not_public_api() {
        let (_, signals) = score_file(&modify("src/helpers.rs", 1));
        assert!(!signals.contains(&RiskSignal::PublicApiSurface));
    }

    #[test]
    fn generated_file_keeps_only_the_base_weight() {
        let (score, signals) = score_file(&modify("src/generated/schema.rs", 500));
        assert_eq!(score, 1);
        assert_eq!(signals, vec![RiskSignal::GeneratedOrVendored]);
    }

    #[test]
    fn vendored_path_is_suppressed() {
        let (score, _) = score_file(&modify("third_party/vendor/x.rs", 50));
        assert_eq!(score, 1);
    }

    /// The patch author picks the path, so a generated or vendored path may lower a file but
    /// never hide it: a new or deleted file there keeps its status signal and outranks a
    /// one-line edit elsewhere.
    #[test]
    fn a_generated_path_never_hides_a_new_or_deleted_file() {
        let mut added = modify("src/generated/x.rs", 1);
        added.status = FileStatus::Added;
        assert_eq!(
            score_file(&added),
            (
                3,
                vec![RiskSignal::GeneratedOrVendored, RiskSignal::NewFile]
            )
        );
        let mut deleted = modify("vendor/y.rs", 1);
        deleted.status = FileStatus::Deleted;
        assert_eq!(
            score_file(&deleted),
            (
                4,
                vec![RiskSignal::GeneratedOrVendored, RiskSignal::DeletedFile]
            )
        );
        let readme = score_file(&modify("README.md", 1)).0;
        assert!(score_file(&added).0 > readme);
    }

    #[test]
    fn target_dir_is_suppressed() {
        let (score, _) = score_file(&modify("target/debug/build.rs", 10));
        assert_eq!(score, 1);
    }

    #[test]
    fn test_file_is_flagged_test_only() {
        let (_, signals) = score_file(&modify("crates/x/tests/it.rs", 1));
        assert!(signals.contains(&RiskSignal::TestOnly));
    }

    #[test]
    fn underscore_test_file_is_test_only() {
        let (_, signals) = score_file(&modify("src/parser_test.rs", 1));
        assert!(signals.contains(&RiskSignal::TestOnly));
    }

    #[test]
    fn test_file_ranks_below_equivalent_source() {
        let source = score_file(&modify("src/thing.rs", 1)).0;
        let test = score_file(&modify("tests/thing_it.rs", 1)).0;
        assert!(test < source);
    }

    #[test]
    fn large_change_signal_fires() {
        let (_, signals) = score_file(&modify("src/big.rs", 50)); // 100 lines
        assert!(signals.contains(&RiskSignal::LargeChange));
    }

    #[test]
    fn small_change_has_no_large_signal() {
        let (_, signals) = score_file(&modify("src/small.rs", 2));
        assert!(!signals.contains(&RiskSignal::LargeChange));
    }

    #[test]
    fn many_hunks_signal_fires() {
        use std::fmt::Write as _;
        let mut s = String::from("--- a/src/x.rs\n+++ b/src/x.rs\n");
        for i in 0..6 {
            let n = i * 10 + 1;
            let _ = write!(s, "@@ -{n},1 +{n},1 @@\n-a\n+b\n");
        }
        let (_, signals) = score_file(&file(&s));
        assert!(signals.contains(&RiskSignal::ManyHunks));
    }

    #[test]
    fn cargo_lock_is_config() {
        let (_, signals) = score_file(&modify("Cargo.lock", 1));
        assert!(signals.contains(&RiskSignal::ConfigOrLockfile));
    }

    #[test]
    fn plain_modified_source_scores_base_plus_source_bonus() {
        // base (1) + primary-source bonus (1) = 2, no other signals.
        let (score, signals) = score_file(&modify("src/util.rs", 1));
        assert_eq!(score, 2);
        assert_eq!(signals.len(), 0);
    }

    #[test]
    fn plain_source_outranks_test_file() {
        let source = score_file(&modify("src/util.rs", 1)).0;
        let test = score_file(&modify("tests/util_it.rs", 1)).0;
        assert!(source > test);
    }

    /// A changed line or a path with bidi controls, invisible or control characters, or a path
    /// that traverses, adds `hazardous_text` (+3), even under a generated path; an unchanged
    /// context line does not.
    #[test]
    fn hazardous_text_raises_the_score() {
        let trojan =
            file("--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-a\n+if ok \u{202E}{ grant() }\n");
        let plain = file("--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-a\n+if ok { grant() }\n");
        let (t, ts) = score_file(&trojan);
        let (p, _) = score_file(&plain);
        assert_eq!((t, ts.contains(&RiskSignal::HazardousText)), (p + 3, true));
        let context = file("--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,2 @@\n x\u{200B}\n-a\n+b\n");
        assert!(!score_file(&context).1.contains(&RiskSignal::HazardousText));
        let generated = file(
            "--- a/gen/generated/x.rs\n+++ b/gen/generated/x.rs\n@@ -1 +1 @@\n-a\n+b\u{1b}[2J\n",
        );
        assert_eq!(score_file(&generated).0, 4);
        let traversal = file("--- a/../x\n+++ b/../x\n@@ -1 +1 @@\n-a\n+b\n");
        assert!(
            score_file(&traversal)
                .1
                .contains(&RiskSignal::HazardousText)
        );
        assert_eq!(RiskSignal::HazardousText.label(), "hazardous_text");
    }

    /// Bug hunt 2026-10-28, H4-07: the path a rename or copy leaves counts, so moving a file
    /// whose name hides a bidi control into a plain name is flagged like the reverse. (A source
    /// that traverses or names `.git` is refused by the parser since H1-09.)
    #[test]
    fn a_hazardous_rename_or_copy_source_is_flagged() {
        let moved = |verb: &str, from: &str| {
            format!(
                "diff --git a/{from} b/src/y.rs\nsimilarity index 100%\n{verb} from {from}\n{verb} to src/y.rs\n"
            )
        };
        for verb in ["rename", "copy"] {
            let (_, hidden) = score_file(&file(&moved(verb, "src/x\u{202E}sr.rs")));
            assert!(
                hidden.contains(&RiskSignal::HazardousText),
                "{verb}: {hidden:?}"
            );
            let (_, plain) = score_file(&file(&moved(verb, "src/x.rs")));
            assert!(
                !plain.contains(&RiskSignal::HazardousText),
                "{verb}: {plain:?}"
            );
            assert!(parse(&moved(verb, ".git/hooks/pre-commit")).is_err());
        }
    }

    #[test]
    fn generated_never_outranks_test_or_source() {
        let generated = score_file(&modify("src/generated/g.rs", 500)).0;
        let test = score_file(&modify("tests/it.rs", 1)).0;
        let source = score_file(&modify("src/util.rs", 1)).0;
        assert!(generated <= test && generated < source);
    }

    #[test]
    fn deleted_outweighs_added_for_same_path_shape() {
        let added = score_file(
            &file("diff --git a/src/x.rs b/src/x.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/x.rs\n@@ -0,0 +1,1 @@\n+a\n"),
        )
        .0;
        let deleted = score_file(
            &file("diff --git a/src/x.rs b/src/x.rs\ndeleted file mode 100644\n--- a/src/x.rs\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-a\n"),
        )
        .0;
        assert!(deleted > added);
    }

    #[test]
    fn public_api_large_change_accumulates_signals() {
        let (_, signals) = score_file(&modify("src/lib.rs", 50));
        assert!(signals.contains(&RiskSignal::PublicApiSurface));
        assert!(signals.contains(&RiskSignal::LargeChange));
    }

    #[test]
    fn node_modules_is_suppressed() {
        let (score, _) = score_file(&modify("web/node_modules/dep/index.rs", 5));
        assert_eq!(score, 1);
    }

    #[test]
    fn dist_path_is_suppressed() {
        let (score, _) = score_file(&modify("build/dist/bundle.rs", 5));
        assert_eq!(score, 1);
    }

    #[test]
    fn test_path_with_slash_test_is_test_only() {
        let (_, signals) = score_file(&modify("src/test/helpers.rs", 1));
        assert!(signals.contains(&RiskSignal::TestOnly));
    }

    #[test]
    fn just_below_large_threshold_has_no_large_signal() {
        // 39 lines each side = 78 < 80.
        let (_, signals) = score_file(&modify("src/x.rs", 39));
        assert!(!signals.contains(&RiskSignal::LargeChange));
    }

    #[test]
    fn new_file_signal_fires() {
        let f = file(
            "diff --git a/n.rs b/n.rs\nnew file mode 100644\n--- /dev/null\n+++ b/n.rs\n@@ -0,0 +1,1 @@\n+x\n",
        );
        let (_, signals) = score_file(&f);
        assert!(signals.contains(&RiskSignal::NewFile));
    }

    #[test]
    fn deleted_file_signal_fires() {
        let f = file(
            "diff --git a/d.rs b/d.rs\ndeleted file mode 100644\n--- a/d.rs\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-x\n",
        );
        let (_, signals) = score_file(&f);
        assert!(signals.contains(&RiskSignal::DeletedFile));
    }

    #[test]
    fn binary_change_signal_fires() {
        let f = file("diff --git a/p.png b/p.png\nBinary files a/p.png and b/p.png differ\n");
        let (_, signals) = score_file(&f);
        assert!(signals.contains(&RiskSignal::BinaryChange));
    }

    #[test]
    fn config_file_signal_fires() {
        let (_, signals) = score_file(&modify("config/app.yaml", 1));
        assert!(signals.contains(&RiskSignal::ConfigOrLockfile));
    }

    #[test]
    fn cargo_toml_is_config() {
        let (_, signals) = score_file(&modify("Cargo.toml", 1));
        assert!(signals.contains(&RiskSignal::ConfigOrLockfile));
    }

    #[test]
    fn public_api_outranks_plain_source() {
        let api = score_file(&modify("src/lib.rs", 1)).0;
        let plain = score_file(&modify("src/util.rs", 1)).0;
        assert!(api > plain);
    }

    #[test]
    fn change_counts_are_accurate() {
        let f = file("--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n ctx\n-a\n+b\n-c\n+d\n");
        assert_eq!(change_counts(&f), (2, 2));
    }

    #[test]
    fn labels_are_stable() {
        assert_eq!(RiskSignal::PublicApiSurface.label(), "public_api_surface");
        assert_eq!(
            RiskSignal::GeneratedOrVendored.label(),
            "generated_or_vendored"
        );
        assert_eq!(RiskSignal::TestOnly.label(), "test_only");
    }

    #[test]
    fn score_is_deterministic_across_runs() {
        let f = modify("src/lib.rs", 50);
        assert_eq!(score_file(&f), score_file(&f));
    }

    #[test]
    fn min_js_is_generated() {
        let (score, _) = score_file(&modify("web/app.min.js", 5));
        assert_eq!(score, 1);
    }

    /// Bug hunt 2026-10-28, H4-07: a symlink or a newly executable file is a risk signal, with
    /// its own weight; neither scored anything before.
    #[test]
    fn symlinks_and_new_executables_are_risk_signals() {
        use RiskSignal::{DeletedFile, ExecutableBit, GeneratedOrVendored, NewFile, SymlinkChange};
        let scored = |input: &str| score_file(&file(input));
        let header = |path: &str, modes: &str| format!("diff --git a/{path} b/{path}\n{modes}");
        let new_file = |path: &str, mode: &str| {
            header(
                path,
                &format!("new file mode {mode}\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1 @@\n+t\n"),
            )
        };
        assert_eq!(
            scored(&new_file("link", "120000")),
            (7, vec![NewFile, SymlinkChange])
        );
        assert_eq!(
            scored(&header(
                "link",
                "deleted file mode 120000\n--- a/link\n+++ /dev/null\n@@ -1 +0,0 @@\n-t\n"
            )),
            (8, vec![DeletedFile, SymlinkChange])
        );
        assert_eq!(
            scored(&new_file("run.sh", "100755")),
            (6, vec![NewFile, ExecutableBit])
        );
        assert_eq!(scored(&new_file("run.sh", "100644")), (4, vec![NewFile]));
        assert_eq!(scored(&new_file("sub", "160000")), (4, vec![NewFile]));
        for x in ["100100", "100010", "100001"] {
            assert_eq!(
                scored(&new_file("run.sh", x)).1,
                vec![NewFile, ExecutableBit],
                "{x}"
            );
        }
        let mode = |old: &str, new: &str| {
            scored(&header("f", &format!("old mode {old}\nnew mode {new}\n")))
        };
        assert_eq!(mode("100644", "100755"), (4, vec![ExecutableBit]));
        assert_eq!(mode("100755", "100644"), (2, vec![]));
        assert_eq!(mode("100755", "100775"), (2, vec![]));
        assert_eq!(
            mode("120000", "100755"),
            (7, vec![SymlinkChange, ExecutableBit])
        );
        assert_eq!(mode("100644", "120000"), (5, vec![SymlinkChange]));
        // A generated path never hides a new executable.
        assert_eq!(
            scored(&new_file("vendor/run.sh", "100755")),
            (5, vec![GeneratedOrVendored, NewFile, ExecutableBit])
        );
        assert_eq!(SymlinkChange.label(), "symlink_change");
        assert_eq!(ExecutableBit.label(), "executable_bit");
    }

    #[test]
    fn basename_handles_no_slash() {
        assert_eq!(basename("lib.rs"), "lib.rs");
        assert_eq!(basename("a/b/lib.rs"), "lib.rs");
    }
}
