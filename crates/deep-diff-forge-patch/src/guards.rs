// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-patch--guards
//! Input contracts a caller can require of a parsed patch (`--require-files`, `--require-hunks`
//! on the CLI; `require_files` and `require_hunks` on the daemon's `diff.compute`). One predicate
//! serves every surface, so the CLI and the daemon refuse exactly the same inputs.

use deep_diff_forge_core::{PatchLineKind, ReviewFile};

/// Which input contracts to apply. There is no `Default` (anchor C-04): "no guards" must be
/// written out, so `..Default::default()` cannot silently drop a contract a caller asked for.
///
/// ```
/// let none = deep_diff_forge_patch::InputGuards { files: false, hunks: false };
/// assert_eq!(none.refusal(&[]), None);
/// ```
///
/// ```compile_fail
/// let _ = deep_diff_forge_patch::InputGuards::default();
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputGuards {
    /// Refuse a patch that parses to 0 files.
    pub files: bool,
    /// Refuse a patch with 0 hunks or 0 added and removed lines.
    pub hunks: bool,
}

impl InputGuards {
    /// The first contract `files` breaks, in order (files, then hunks), as the reason the CLI
    /// prints after `refused: `; `None` when every requested contract holds.
    #[must_use]
    pub fn refusal(self, files: &[ReviewFile]) -> Option<&'static str> {
        if self.files && files.is_empty() {
            Some("0 files in input (--require-files)")
        } else if self.hunks && !has_reviewable_hunks(files) {
            Some("0 hunks in input (--require-hunks)")
        } else {
            None
        }
    }
}

/// True when at least one hunk across all files carries an added or removed line (so "0 hunks"
/// and "0 changed lines" are both refused). A header-only (`diff --git a/x b/x`) or rename-only
/// diff parses to one file with no hunks; a hunk made only of context lines changes nothing.
/// Neither is something a review can judge.
#[must_use]
pub fn has_reviewable_hunks(files: &[ReviewFile]) -> bool {
    files
        .iter()
        .flat_map(|f| f.patch_twin.hunks.iter())
        .flat_map(|h| h.lines.iter())
        .any(|l| matches!(l.kind, PatchLineKind::Added | PatchLineKind::Removed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(patch: &str) -> Vec<ReviewFile> {
        crate::parse(patch).unwrap()
    }

    #[test]
    fn refusals_come_in_order_and_only_when_requested() {
        let both = InputGuards {
            files: true,
            hunks: true,
        };
        assert_eq!(
            both.refusal(&[]),
            Some("0 files in input (--require-files)")
        );
        assert_eq!(
            InputGuards {
                files: false,
                hunks: true
            }
            .refusal(&[]),
            Some("0 hunks in input (--require-hunks)")
        );
        let none = InputGuards {
            files: false,
            hunks: false,
        };
        assert_eq!(none.refusal(&[]), None);
        let header_only = files("diff --git a/x b/x\nindex 1..2 100644\n");
        assert_eq!(
            both.refusal(&header_only),
            Some("0 hunks in input (--require-hunks)")
        );
        assert_eq!(
            InputGuards {
                files: true,
                hunks: false
            }
            .refusal(&header_only),
            None
        );
        let changed = files("--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n");
        assert_eq!(both.refusal(&changed), None);
        let context_only = files("--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n a\n");
        assert!(!has_reviewable_hunks(&context_only));
        let added = files("--- a/x\n+++ b/x\n@@ -0,0 +1,1 @@\n+b\n");
        let removed = files("--- a/x\n+++ b/x\n@@ -1,1 +0,0 @@\n-a\n");
        assert!(has_reviewable_hunks(&added) && has_reviewable_hunks(&removed));
    }
}
