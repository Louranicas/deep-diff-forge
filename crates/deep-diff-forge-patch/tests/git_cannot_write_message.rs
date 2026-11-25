//! S3.34.c: the message of a refusal for a patch `git apply` cannot write. A file of its own, so
//! that changes to the parse-error type and to the conformance tests merge without touching it.

use deep_diff_forge_patch::{PatchParseError, parse};

/// S3.34.c, the missed mutant of `PatchParseError::fmt_git_outcome` (its `GitCannotWrite` arm
/// deleted, so the message is empty): the refusal of a patch `git apply` cannot write says why,
/// with the section's line.
#[test]
fn a_patch_git_cannot_write_is_refused_with_its_reason() {
    let patch = "diff --git a/d b/d\nnew file mode 100644\n--- /dev/null\n+++ b/d\n@@ -0,0 +1 @@\n+a\ndiff --git a/d/e b/d/e\nnew file mode 100644\n--- /dev/null\n+++ b/d/e\n@@ -0,0 +1 @@\n+b\n";
    let error = parse(patch).unwrap_err();
    assert!(
        matches!(error, PatchParseError::GitCannotWrite { .. }),
        "{error:?}"
    );
    assert_eq!(
        error.to_string(),
        "the file section beginning at line 7 cannot be written by git apply, which would fail after removing files: one path is both a file and the directory of another",
        "the refusal of a patch git cannot write lost its message"
    );
}

/// Package P3 of 0.11.1 (row 22 of the reading of the macOS CI run of 0.11.0): a new symlink with
/// an empty target is refused on every system, and the refusal says what is true on each. Git on
/// Linux cannot write it (its `symlink(2)` refuses an empty target, after Git has removed what the
/// patch deletes); Git on macOS writes the link. "cannot be written by git apply" was Linux's.
#[test]
fn an_empty_symlink_target_is_refused_with_what_each_system_does() {
    let patch = "diff --git a/x b/x\ndeleted file mode 100644\nindex 1111111..0000000\n--- a/x\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\ndiff --git a/s b/s\nnew file mode 120000\nindex 0000000..e69de29\n";
    let error = parse(patch).unwrap_err();
    assert!(
        matches!(error, PatchParseError::GitCannotWrite { .. }),
        "{error:?}"
    );
    assert_eq!(
        error.to_string(),
        "the file section beginning at line 8 is refused: an empty symlink target, which git apply on Linux fails to write after removing files (git apply on macOS writes the link)"
    );
}
