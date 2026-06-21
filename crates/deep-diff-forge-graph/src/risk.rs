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
    /// Generated or vendored file (strongly suppressed).
    GeneratedOrVendored,
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
        }
    }
}

/// Lines (added+removed) at or above which a change counts as large.
pub const LARGE_CHANGE_LINES: usize = 80;
/// Hunk count at or above which a file counts as many-hunked.
pub const MANY_HUNKS: usize = 5;

/// Compute the risk score and contributing signals for one file.
///
/// Scores are deterministic and additive; generated/vendored files are
/// suppressed to zero, and test-only files are de-prioritized.
#[must_use]
pub fn score_file(file: &ReviewFile) -> (u32, Vec<RiskSignal>) {
    let path = &file.path;
    if is_generated_or_vendored(path) {
        return (0, vec![RiskSignal::GeneratedOrVendored]);
    }

    let mut signals = Vec::new();
    let mut score: u32 = 1; // base weight for any reviewable change

    let test = is_test(path);
    let config = is_config_or_lockfile(path);
    // Primary-source bonus: ranks reviewable source above tests/config, while
    // generated/vendored files (handled above) stay hard-suppressed to zero.
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

    match file.status {
        FileStatus::Added => {
            signals.push(RiskSignal::NewFile);
            score += 2;
        }
        FileStatus::Deleted => {
            signals.push(RiskSignal::DeletedFile);
            score += 3;
        }
        FileStatus::BinaryChanged => {
            signals.push(RiskSignal::BinaryChange);
            score += 1;
        }
        _ => {}
    }

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
    fn generated_file_is_suppressed_to_zero() {
        let (score, signals) = score_file(&modify("src/generated/schema.rs", 50));
        assert_eq!(score, 0);
        assert_eq!(signals, vec![RiskSignal::GeneratedOrVendored]);
    }

    #[test]
    fn vendored_path_is_suppressed() {
        let (score, _) = score_file(&modify("third_party/vendor/x.rs", 50));
        assert_eq!(score, 0);
    }

    #[test]
    fn target_dir_is_suppressed() {
        let (score, _) = score_file(&modify("target/debug/build.rs", 10));
        assert_eq!(score, 0);
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
        assert!(signals.is_empty());
    }

    #[test]
    fn plain_source_outranks_test_file() {
        let source = score_file(&modify("src/util.rs", 1)).0;
        let test = score_file(&modify("tests/util_it.rs", 1)).0;
        assert!(source > test);
    }

    #[test]
    fn generated_ranks_below_test() {
        let generated = score_file(&modify("src/generated/g.rs", 1)).0;
        let test = score_file(&modify("tests/it.rs", 1)).0;
        assert!(generated < test);
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
        assert_eq!(score, 0);
    }

    #[test]
    fn dist_path_is_suppressed() {
        let (score, _) = score_file(&modify("build/dist/bundle.rs", 5));
        assert_eq!(score, 0);
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
        assert_eq!(score, 0);
    }

    #[test]
    fn basename_handles_no_slash() {
        assert_eq!(basename("lib.rs"), "lib.rs");
        assert_eq!(basename("a/b/lib.rs"), "lib.rs");
    }
}
