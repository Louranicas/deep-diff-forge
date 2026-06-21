use crate::risk::{RiskSignal, score_file};
use deep_diff_forge_core::{FileStatus, ReviewFile};

/// A file placed in the ranked review stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedFile {
    /// File path.
    pub path: String,
    /// File status.
    pub status: FileStatus,
    /// Risk score (higher = review first).
    pub score: u32,
    /// Contributing signals.
    pub signals: Vec<RiskSignal>,
}

/// Rank the review by descending risk score, with a deterministic path
/// tie-break, so the same review always produces the same order.
#[must_use]
pub fn rank(files: &[ReviewFile]) -> Vec<RankedFile> {
    let mut ranked: Vec<RankedFile> = files
        .iter()
        .map(|file| {
            let (score, signals) = score_file(file);
            RankedFile {
                path: file.path.clone(),
                status: file.status,
                score,
                signals,
            }
        })
        .collect();
    ranked.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_patch::parse;

    const MULTI: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,1 +1,1 @@
-a
+b
diff --git a/tests/it.rs b/tests/it.rs
--- a/tests/it.rs
+++ b/tests/it.rs
@@ -1,1 +1,1 @@
-a
+b
diff --git a/src/generated/g.rs b/src/generated/g.rs
--- a/src/generated/g.rs
+++ b/src/generated/g.rs
@@ -1,1 +1,1 @@
-a
+b
";

    fn ranked(input: &str) -> Vec<RankedFile> {
        rank(&parse(input).unwrap())
    }

    #[test]
    fn ranks_public_api_first() {
        let r = ranked(MULTI);
        assert_eq!(r[0].path, "src/lib.rs");
    }

    #[test]
    fn generated_file_ranks_last() {
        let r = ranked(MULTI);
        assert_eq!(r.last().unwrap().path, "src/generated/g.rs");
        assert_eq!(r.last().unwrap().score, 0);
    }

    #[test]
    fn ranking_is_descending_by_score() {
        let r = ranked(MULTI);
        for pair in r.windows(2) {
            assert!(pair[0].score >= pair[1].score);
        }
    }

    #[test]
    fn ranking_is_deterministic() {
        assert_eq!(ranked(MULTI), ranked(MULTI));
    }

    #[test]
    fn tie_break_is_path_ascending() {
        // Two equivalent plain-source files: tie broken by path.
        let input = "\
--- a/src/zeta.rs
+++ b/src/zeta.rs
@@ -1,1 +1,1 @@
-a
+b
--- a/src/alpha.rs
+++ b/src/alpha.rs
@@ -1,1 +1,1 @@
-a
+b
";
        let r = ranked(input);
        assert_eq!(r[0].score, r[1].score);
        assert_eq!(r[0].path, "src/alpha.rs");
        assert_eq!(r[1].path, "src/zeta.rs");
    }

    #[test]
    fn empty_review_ranks_empty() {
        assert!(rank(&[]).is_empty());
    }

    #[test]
    fn ranked_file_carries_status() {
        let r = ranked("--- a/src/x.rs\n+++ b/src/x.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n");
        assert_eq!(r[0].status, FileStatus::Modified);
    }

    #[test]
    fn ranked_file_carries_signals() {
        let r = ranked("--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n");
        assert!(r[0].signals.contains(&RiskSignal::PublicApiSurface));
    }

    #[test]
    fn all_files_appear_in_ranking() {
        let r = ranked(MULTI);
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn new_file_outranks_plain_modify_same_path_depth() {
        let input = "\
diff --git a/src/added.rs b/src/added.rs
new file mode 100644
--- /dev/null
+++ b/src/added.rs
@@ -0,0 +1,1 @@
+x
diff --git a/src/edited.rs b/src/edited.rs
--- a/src/edited.rs
+++ b/src/edited.rs
@@ -1,1 +1,1 @@
-a
+b
";
        let r = ranked(input);
        assert_eq!(r[0].path, "src/added.rs");
    }
}
