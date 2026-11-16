// DDF integration: R03, R07, R46. The conformance table (`CONFORMANCE.md`): one test per row, each feeding the row's forms to DDF, to `git apply --numstat --summary` and to a real `git apply` over a seeded object store, asserting what the row states.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-patch--tests--conformance
//! Each row of `CONFORMANCE.md` is a test here named after it (`row_<row>`), and
//! [`every_row_has_a_test_and_every_test_a_row`] holds the two to each other. Every case checks
//! the property too: when DDF reads a patch and Git applies it, Git's tree is the tree the
//! review describes.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real git processes"
)]

mod support;

use deep_diff_forge_core::{FileStatus, ReviewFile};
use deep_diff_forge_patch::{ParseOptions, PatchParseError, decode, parse_decoded};
use support::{Repo, Tree, agree, regular, reviewed_tree};

/// What DDF and Git each made of one patch.
struct Seen {
    patch: String,
    ddf: Result<Vec<ReviewFile>, PatchParseError>,
    /// `git apply --numstat --summary`, or `None` when Git refuses to read the patch.
    reading: Option<String>,
    /// The tree a real `git apply` leaves, or `None` when it refuses.
    applied: Option<Tree>,
}

impl Seen {
    /// DDF's review; the case must be one DDF reads.
    fn read(&self) -> &[ReviewFile] {
        self.ddf
            .as_deref()
            .unwrap_or_else(|e| panic!("DDF refused {:?}: {e}", self.patch))
    }

    fn status(&self) -> (String, FileStatus) {
        let files = self.read();
        assert_eq!(files.len(), 1, "{:?}", self.patch);
        (files[0].path.clone(), files[0].status)
    }

    /// DDF refuses the case, with the error `expected` names.
    fn refused(&self, expected: fn(&PatchParseError) -> bool) {
        match &self.ddf {
            Err(e) if expected(e) => {}
            other => panic!("{:?}: {other:?}", self.patch),
        }
    }

    /// DDF refuses the case as a shape Git does not write.
    fn refused_as_unwritten(&self) {
        self.refused(|e| matches!(e, PatchParseError::HeaderNotWrittenByGit { .. }));
    }

    /// Git applies the case; its tree.
    fn git_tree(&self) -> &Tree {
        assert!(self.reading.is_some(), "Git reads {:?}", self.patch);
        self.applied
            .as_ref()
            .unwrap_or_else(|| panic!("Git applies {:?}", self.patch))
    }

    /// Git writes nothing: it refuses to read the patch, or to apply it.
    fn git_writes_nothing(&self) {
        assert!(self.applied.is_none(), "Git applied {:?}", self.patch);
    }
}

/// The tree before every case: `x` holds `a`, `y` holds `y`, `e` is empty.
fn before() -> Tree {
    [("x", &b"a\n"[..]), ("y", b"y\n"), ("e", b"")]
        .into_iter()
        .map(|(path, content)| (path.to_owned(), regular(content)))
        .collect()
}

/// A content as the tree holds it.
fn file(content: &str, executable: bool) -> support::Entry {
    support::Entry {
        executable,
        ..regular(content.as_bytes())
    }
}

/// A repository whose store holds `x`'s blob (`@OLD@`) and `stored` (`@NEW@`).
fn rig(tag: &str) -> Repo {
    Repo::new(tag, &[b"a\n", b"stored\n", b"y\n", b""])
}

/// Runs `patch` (with `@OLD@`, `@NEW@` and `@ZERO@` filled in) through DDF and Git, and checks
/// the property: when both accept it, Git's tree is the reviewed one.
fn see(repo: &Repo, patch: &str) -> Seen {
    let patch = patch
        .replace("@OLD@", &repo.ids[0])
        .replace("@NEW@", &repo.ids[1])
        .replace("@ZERO@", &"0".repeat(repo.ids[0].len()));
    let ddf = parse_decoded(&decode(patch.as_bytes()), ParseOptions::default());
    let reading = repo.reading(patch.as_bytes());
    repo.reset(&before());
    let checks = repo.checks(patch.as_bytes());
    let applied = repo.apply(&before(), patch.as_bytes());
    assert_eq!(
        checks,
        applied.is_some(),
        "--check and apply differ: {patch:?}"
    );
    if let (Ok(files), Some(git)) = (&ddf, &applied) {
        let reviewed = reviewed_tree(&before(), files)
            .unwrap_or_else(|| panic!("the review of {patch:?} does not fit its tree"));
        assert!(
            agree(git, &reviewed),
            "DDF read a patch git apply writes otherwise: {patch:?}\n git: {git:?}\n ddf: {reviewed:?}"
        );
    }
    Seen {
        patch,
        ddf,
        reading,
        applied,
    }
}

const X: &str = "diff --git a/x b/x\n";
const HUNK: &str = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";

#[test]
fn row_diff_git() {
    let r = rig("diff-git");
    let s = see(&r, &format!("{X}index @OLD@..@NEW@ 100644\n{HUNK}"));
    assert_eq!(s.status(), ("x".into(), FileStatus::Modified));
    assert_eq!(s.git_tree()["x"], file("b\n", false));
    // Alone, the line is no patch to Git.
    let s = see(&r, X);
    assert!(s.reading.is_none());
    s.refused(|e| matches!(e, PatchParseError::SectionWithoutPatch { .. }));
    // Halves that name two paths give no name, and with nothing else naming the files Git
    // refuses the section. Git's diff writes such a section only for two files compared with
    // `git diff --no-index` (a binary or a mode-only pair): read as the pair, a stated limit
    // (Git writes nothing). A file created or deleted under such halves, which no Git writes,
    // is refused; 0.8.4 took the rightmost ` b/`.
    for headers in [
        "index @OLD@..@NEW@ 100644\nBinary files a/x and b/z differ\n",
        "old mode 100644\nnew mode 100755\n",
    ] {
        let s = see(&r, &format!("diff --git a/x b/z\n{headers}"));
        assert!(s.reading.is_none(), "{headers}");
        assert_eq!(s.read()[0].path, "z");
    }
    let s = see(
        &r,
        "diff --git a/x b/z\ndeleted file mode 100644\nindex @OLD@..@ZERO@\nBinary files a/x and /dev/null differ\n",
    );
    assert!(s.reading.is_none());
    s.refused(|e| matches!(e, PatchParseError::InvalidPath { line_number: 1 }));
}

#[test]
fn row_old_mode() {
    let r = rig("old-mode");
    let s = see(&r, &format!("{X}old mode 100644\nnew mode 100755\n"));
    assert_eq!(s.status(), ("x".into(), FileStatus::ModeChanged));
    assert!(s.git_tree()["x"].executable);
    // Repeated, the last wins in Git; refused, as by 0.8.4.
    let s = see(
        &r,
        &format!("{X}old mode 100755\nold mode 100644\nnew mode 100755\n"),
    );
    assert!(s.git_tree()["x"].executable);
    s.refused(|e| matches!(e, PatchParseError::InconsistentHeader { .. }));
    // Not octal: Git refuses.
    let s = see(&r, &format!("{X}old mode 10064x\nnew mode 100755\n"));
    assert!(s.reading.is_none());
    s.refused(|e| matches!(e, PatchParseError::InvalidMode { .. }));
    // A mode Git reads as 100644 but never writes: refused now (0.8.4 read it).
    let s = see(&r, &format!("{X}old mode 100644\nnew mode 100664\n{HUNK}"));
    assert_eq!(s.git_tree()["x"], file("b\n", false));
    s.refused_as_unwritten();
    // After `index`, where Git never writes it: Git reads it; refused now.
    let s = see(
        &r,
        &format!("{X}index @OLD@..@NEW@\nold mode 100644\nnew mode 100755\n{HUNK}"),
    );
    assert_eq!(s.git_tree()["x"], file("b\n", true));
    s.refused_as_unwritten();
}

#[test]
fn row_new_mode() {
    let r = rig("new-mode");
    // Alone it names no change: Git applies nothing.
    let s = see(&r, &format!("{X}new mode 100755\n"));
    s.git_writes_nothing();
    s.refused(|e| matches!(e, PatchParseError::InvalidMode { .. }));
    // Before `old mode`: Git reads the change; refused now (0.8.4 read it).
    let s = see(&r, &format!("{X}new mode 100755\nold mode 100644\n"));
    assert!(s.git_tree()["x"].executable);
    s.refused_as_unwritten();
}

#[test]
fn row_deleted_file_mode() {
    let r = rig("deleted-file-mode");
    let s = see(
        &r,
        &format!(
            "{X}deleted file mode 100644\nindex @OLD@..@ZERO@\n--- a/x\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\n"
        ),
    );
    assert_eq!(s.status(), ("x".into(), FileStatus::Deleted));
    assert!(!s.git_tree().contains_key("x"));
    // An empty file, with no body.
    let s = see(
        &r,
        "diff --git a/e b/e\ndeleted file mode 100644\nindex e69de29..0000000\n",
    );
    assert_eq!(s.status(), ("e".into(), FileStatus::Deleted));
    assert!(!s.git_tree().contains_key("e"));
    // Repeated: Git deletes; refused, as by 0.8.4.
    let s = see(
        &r,
        &format!(
            "{X}deleted file mode 100644\ndeleted file mode 100644\nindex @OLD@..@ZERO@\n--- a/x\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\n"
        ),
    );
    assert!(!s.git_tree().contains_key("x"));
    s.refused(|e| matches!(e, PatchParseError::InconsistentHeader { .. }));
    // With `new file mode`: Git refuses ("inconsistent header lines").
    let s = see(
        &r,
        &format!("{X}deleted file mode 100644\nnew file mode 100644\n"),
    );
    assert!(s.reading.is_none());
    s.refused(|e| matches!(e, PatchParseError::InconsistentHeader { .. }));
}

#[test]
fn row_new_file_mode() {
    let r = rig("new-file-mode");
    let create = "diff --git a/n b/n\nnew file mode @MODE@\nindex @ZERO@..@NEW@\n--- /dev/null\n+++ b/n\n@@ -0,0 +1 @@\n+stored\n";
    let s = see(&r, &create.replace("@MODE@", "100755"));
    assert_eq!(s.status(), ("n".into(), FileStatus::Added));
    assert_eq!(s.git_tree()["n"], file("stored\n", true));
    // An empty file, with no body.
    let s = see(
        &r,
        "diff --git a/n b/n\nnew file mode 100644\nindex 0000000..e69de29\n",
    );
    assert_eq!(s.status(), ("n".into(), FileStatus::Added));
    assert_eq!(s.git_tree()["n"], file("", false));
    // `new mode` after it wins in Git (a symlink); refused, as by 0.8.4.
    let s = see(
        &r,
        "diff --git a/n b/n\nnew file mode 100644\nnew mode 120000\nindex @ZERO@..@NEW@\n--- /dev/null\n+++ b/n\n@@ -0,0 +1 @@\n+stored\n",
    );
    assert!(s.git_tree()["n"].symlink);
    s.refused(|e| matches!(e, PatchParseError::InconsistentHeader { .. }));
    // A mode Git reads as a submodule: Git makes a directory; refused, as by 0.8.4.
    let s = see(&r, &create.replace("@MODE@", "644"));
    assert!(s.git_tree().contains_key("n/"));
    s.refused(|e| matches!(e, PatchParseError::InvalidMode { .. }));
    // `100664`, which Git reads as `100644` and never writes: refused now.
    let s = see(&r, &create.replace("@MODE@", "100664"));
    assert_eq!(s.git_tree()["n"], file("stored\n", false));
    s.refused_as_unwritten();
}

const COPY: &str = "diff --git a/y b/c\nsimilarity index 100%\ncopy from y\ncopy to c\n";

#[test]
fn row_copy_from() {
    let r = rig("copy-from");
    let s = see(&r, COPY);
    assert_eq!(s.status(), ("c".into(), FileStatus::Copied));
    assert_eq!(s.git_tree()["c"], file("y\n", false));
    // Without its similarity line it is read alike.
    let s = see(&r, "diff --git a/y b/c\ncopy from y\ncopy to c\n");
    assert_eq!(s.status(), ("c".into(), FileStatus::Copied));
    // Before `similarity index`: Git reads it; refused now (0.8.4 read it).
    let s = see(
        &r,
        "diff --git a/y b/c\ncopy from y\nsimilarity index 100%\ncopy to c\n",
    );
    assert_eq!(s.git_tree()["c"], file("y\n", false));
    s.refused_as_unwritten();
    // Twice: the last wins in Git; refused, as by 0.8.4.
    let s = see(
        &r,
        "diff --git a/y b/c\nsimilarity index 100%\ncopy from x\ncopy from y\ncopy to c\n",
    );
    assert_eq!(s.git_tree()["c"], file("y\n", false));
    s.refused(|e| matches!(e, PatchParseError::InvalidPath { .. }));
}

#[test]
fn row_copy_to() {
    let r = rig("copy-to");
    // Twice: Git writes the last; 0.8.4 read that too; refused now.
    let s = see(&r, &format!("{COPY}copy to d\n"));
    assert!(s.git_tree().contains_key("d") && !s.git_tree().contains_key("c"));
    s.refused_as_unwritten();
    // Before `copy from`: Git reads it; refused now.
    let s = see(
        &r,
        "diff --git a/y b/c\nsimilarity index 100%\ncopy to c\ncopy from y\n",
    );
    assert_eq!(s.git_tree()["c"], file("y\n", false));
    s.refused_as_unwritten();
}

const RENAME: &str = "diff --git a/x b/r\nsimilarity index 100%\nrename from x\nrename to r\n";

#[test]
fn row_rename_from() {
    let r = rig("rename-from");
    let s = see(&r, RENAME);
    assert_eq!(s.status(), ("r".into(), FileStatus::Renamed));
    assert_eq!(s.git_tree()["r"], file("a\n", false));
    assert!(!s.git_tree().contains_key("x"));
    // With an edit.
    let s = see(
        &r,
        "diff --git a/x b/r\nsimilarity index 50%\nrename from x\nrename to r\nindex @OLD@..@NEW@ 100644\n--- a/x\n+++ b/r\n@@ -1 +1 @@\n-a\n+b\n",
    );
    assert_eq!(s.git_tree()["r"], file("b\n", false));
    // Without its similarity line it is read alike.
    let s = see(&r, "diff --git a/x b/r\nrename from x\nrename to r\n");
    assert_eq!(s.status(), ("r".into(), FileStatus::Renamed));
    // Twice: the last wins in Git; refused, as by 0.8.4.
    let s = see(
        &r,
        "diff --git a/x b/r\nsimilarity index 100%\nrename from y\nrename from x\nrename to r\n",
    );
    assert_eq!(s.git_tree()["r"], file("a\n", false));
    s.refused(|e| matches!(e, PatchParseError::InvalidPath { .. }));
}

#[test]
fn row_rename_to() {
    let r = rig("rename-to");
    // Twice: Git writes the last; 0.8.4 read that too; refused now.
    let s = see(&r, &format!("{RENAME}rename to s\n"));
    assert!(s.git_tree().contains_key("s") && !s.git_tree().contains_key("r"));
    s.refused_as_unwritten();
    // Before `rename from`: Git reads it; refused now.
    let s = see(
        &r,
        "diff --git a/x b/r\nsimilarity index 100%\nrename to r\nrename from x\n",
    );
    assert_eq!(s.git_tree()["r"], file("a\n", false));
    s.refused_as_unwritten();
}

#[test]
fn row_rename_old() {
    let r = rig("rename-old");
    let s = see(
        &r,
        "diff --git a/x b/r\nsimilarity index 100%\nrename old x\nrename new r\n",
    );
    assert_eq!(s.git_tree()["r"], file("a\n", false));
    s.refused(|e| matches!(e, PatchParseError::UnsupportedFormat { .. }));
}

#[test]
fn row_rename_new() {
    let r = rig("rename-new");
    let s = see(
        &r,
        "diff --git a/x b/r\nsimilarity index 100%\nrename from x\nrename new r\n",
    );
    assert_eq!(s.git_tree()["r"], file("a\n", false));
    s.refused(|e| matches!(e, PatchParseError::UnsupportedFormat { .. }));
}

#[test]
fn row_similarity_index() {
    let r = rig("similarity-index");
    // Beside a change in place, which Git never writes: Git applies it; refused now.
    let s = see(
        &r,
        &format!("{X}similarity index 90%\nindex @OLD@..@NEW@ 100644\n{HUNK}"),
    );
    assert_eq!(s.git_tree()["x"], file("b\n", false));
    s.refused_as_unwritten();
    // Scores Git ignores (over 100) or reads without its `%`: refused now.
    for score in ["101%", "90"] {
        let s = see(
            &r,
            &format!("diff --git a/x b/r\nsimilarity index {score}\nrename from x\nrename to r\n"),
        );
        assert_eq!(s.git_tree()["r"], file("a\n", false));
        s.refused_as_unwritten();
    }
}

#[test]
fn row_dissimilarity_index() {
    let r = rig("dissimilarity-index");
    let s = see(
        &r,
        &format!("{X}dissimilarity index 100%\nindex @OLD@..@NEW@ 100644\n{HUNK}"),
    );
    assert_eq!(s.status(), ("x".into(), FileStatus::Modified));
    assert_eq!(s.git_tree()["x"], file("b\n", false));
    // Beside a rename, which Git never writes: Git renames; refused now.
    let s = see(
        &r,
        "diff --git a/x b/r\ndissimilarity index 10%\nrename from x\nrename to r\n",
    );
    assert_eq!(s.git_tree()["r"], file("a\n", false));
    s.refused_as_unwritten();
}

#[test]
fn row_index() {
    let r = rig("index");
    for (index, read) in [
        ("index @OLD@..@NEW@ 100644\n", true),
        // With no mode on a change in place, twice, or with ids that are not hex: Git applies
        // each; refused now (0.8.4 read them).
        ("index @OLD@..@NEW@\n", false),
        (
            "index @OLD@..@NEW@ 100644\nindex @OLD@..@NEW@ 100644\n",
            false,
        ),
        ("index zzzzzzz..yyyyyyy 100644\n", false),
    ] {
        let s = see(&r, &format!("{X}{index}{HUNK}"));
        assert_eq!(s.git_tree()["x"], file("b\n", false), "{index}");
        if read {
            assert_eq!(s.status(), ("x".into(), FileStatus::Modified));
        } else {
            s.refused_as_unwritten();
        }
    }
    // With a mode beside the mode lines, which Git never writes: refused, as by 0.8.4.
    let s = see(
        &r,
        &format!("{X}old mode 100644\nnew mode 100755\nindex @OLD@..@NEW@ 100644\n{HUNK}"),
    );
    assert!(s.git_tree()["x"].executable);
    s.refused(|e| matches!(e, PatchParseError::InconsistentHeader { .. }));
    // After the `---`/`+++`: Git reads it; refused, as by 0.8.4.
    let s = see(
        &r,
        &format!("{X}--- a/x\n+++ b/x\nindex @OLD@..@NEW@ 100644\n@@ -1 +1 @@\n-a\n+b\n"),
    );
    assert_eq!(s.git_tree()["x"], file("b\n", false));
    s.refused(|e| matches!(e, PatchParseError::MisplacedHeader { .. }));
}

#[test]
fn row_old_name() {
    let r = rig("old-name");
    // Without its `index` line it is read alike (DDF's own rendering of a traditional diff).
    let s = see(&r, &format!("{X}{HUNK}"));
    assert_eq!(s.status(), ("x".into(), FileStatus::Modified));
    // Names that differ from each other with no rename line: Git renames.
    let s = see(
        &r,
        "diff --git a/x b/r\nindex @OLD@..@NEW@ 100644\n--- a/x\n+++ b/r\n@@ -1 +1 @@\n-a\n+b\n",
    );
    assert_eq!(s.status(), ("r".into(), FileStatus::Renamed));
    assert_eq!(s.git_tree()["r"], file("b\n", false));
    // DDF's rendering of it is a shape Git writes, and reads back the same.
    let files = s.read().to_vec();
    let rendered = deep_diff_forge_patch::render_unified(&files);
    assert_eq!(
        deep_diff_forge_patch::parse(&rendered).unwrap(),
        files,
        "{rendered}"
    );
    // A second pair: Git checks it; refused, as by 0.8.4.
    let s = see(&r, &format!("{X}--- a/x\n+++ b/x\n{HUNK}"));
    assert_eq!(s.git_tree()["x"], file("b\n", false));
    s.refused(|e| matches!(e, PatchParseError::MisplacedHeader { .. }));
    // `/dev/null` without `new file mode`: refused, as by 0.8.4 (Git reads `dev/null`).
    let s = see(
        &r,
        &format!("{X}--- /dev/null\n+++ b/x\n@@ -0,0 +1 @@\n+b\n"),
    );
    s.git_writes_nothing();
    s.refused(|e| matches!(e, PatchParseError::InconsistentHeader { .. }));
}

#[test]
fn row_new_name() {
    let r = rig("new-name");
    // Not the `rename to` name: Git refuses ("inconsistent new filename").
    let s = see(
        &r,
        "diff --git a/x b/r\nsimilarity index 90%\nrename from x\nrename to r\nindex @OLD@..@NEW@ 100644\n--- a/x\n+++ b/s\n@@ -1 +1 @@\n-a\n+b\n",
    );
    assert!(s.reading.is_none());
    s.refused(|e| matches!(e, PatchParseError::InvalidPath { .. }));
    // Without its `---`: Git names no file; refused, as by 0.8.4.
    let s = see(
        &r,
        &format!("{X}index @OLD@..@NEW@ 100644\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n"),
    );
    assert!(s.reading.is_none());
    s.refused(|e| matches!(e, PatchParseError::MisplacedHeader { .. }));
}

#[test]
fn row_hunk_header() {
    let r = rig("hunk-header");
    // With no `---`/`+++` before it, which Git never writes: Git applies; refused now.
    let s = see(
        &r,
        &format!("{X}index @OLD@..@NEW@ 100644\n@@ -1 +1 @@\n-a\n+b\n"),
    );
    assert_eq!(s.git_tree()["x"], file("b\n", false));
    s.refused_as_unwritten();
}

#[test]
fn row_other_line() {
    let r = rig("other-line");
    // It ends the header: the mode change is the patch.
    let s = see(&r, &format!("{X}old mode 100644\nnew mode 100755\nnote\n"));
    assert_eq!(s.status(), ("x".into(), FileStatus::ModeChanged));
    assert_eq!(s.git_tree()["x"], file("a\n", true));
    // Git reads a hunk after it as a new traditional patch; refused, as by 0.8.4.
    let s = see(
        &r,
        &format!("{X}old mode 100644\nnew mode 100755\nnote\n{HUNK}"),
    );
    assert_eq!(s.git_tree()["x"], file("b\n", true));
    s.refused(|e| matches!(e, PatchParseError::MisplacedHeader { .. }));
    // With no change in the headers before it: "patch with only garbage".
    let s = see(&r, &format!("{X}index @OLD@..@NEW@ 100644\nnote\n"));
    s.git_writes_nothing();
    s.refused(|e| matches!(e, PatchParseError::SectionWithoutPatch { .. }));
}

#[test]
fn row_git_binary_patch() {
    let r = rig("git-binary-patch");
    let s = see(
        &r,
        &format!("{X}index @OLD@..@ZERO@ 100644\nGIT binary patch\nliteral 0\nHcmV?d00001\n\n"),
    );
    assert_eq!(s.status(), ("x".into(), FileStatus::BinaryChanged));
    assert_eq!(s.git_tree()["x"], file("", false));
    // With a carriage return it is no marker to Git, which changes only the mode; 0.8.4
    // reviewed a binary change; refused now.
    let s = see(
        &r,
        &format!(
            "{X}old mode 100644\nnew mode 100755\nindex @OLD@..@ZERO@\nGIT binary patch\r\nliteral 0\nHcmV?d00001\n\n"
        ),
    );
    assert_eq!(s.git_tree()["x"], file("a\n", true));
    s.refused_as_unwritten();
    // With no `literal` or `delta` line: Git reads no patch; refused, as by 0.8.4.
    let s = see(
        &r,
        &format!("{X}index @OLD@..@ZERO@ 100644\nGIT binary patch\n\n"),
    );
    assert!(s.reading.is_none());
    s.refused(|e| matches!(e, PatchParseError::UnsupportedFormat { .. }));
}

#[test]
fn row_binary_files() {
    let r = rig("binary-files");
    let s = see(
        &r,
        &format!("{X}index @OLD@..@NEW@ 100644\nBinary files a/x and b/x differ\n"),
    );
    assert_eq!(s.status(), ("x".into(), FileStatus::BinaryChanged));
    assert_eq!(s.git_tree()["x"], file("stored\n", false));
    let s = see(
        &r,
        "diff --git a/n b/n\nnew file mode 100644\nindex @ZERO@..@NEW@\nBinary files /dev/null and b/n differ\n",
    );
    assert_eq!(s.git_tree()["n"], file("stored\n", false));
    assert_eq!(s.read()[0].path, "n");
    // With a carriage return: as for `GIT binary patch`.
    let s = see(
        &r,
        &format!(
            "{X}old mode 100644\nnew mode 100755\nindex @OLD@..@NEW@\nBinary files a/x and b/x differ\r\n"
        ),
    );
    assert_eq!(s.git_tree()["x"], file("a\n", true));
    s.refused_as_unwritten();
    // Right after `diff --git`: no header, so Git reads no patch; refused, as by 0.8.4.
    let s = see(&r, &format!("{X}Binary files a/x and b/x differ\n"));
    assert!(s.reading.is_none());
    s.refused(|e| matches!(e, PatchParseError::SectionWithoutPatch { .. }));
}

#[test]
fn row_files_differ() {
    let r = rig("files-differ");
    // Git writes the stored blob, and no Git writes the line: refused now (0.8.4 read a
    // binary change, v0.8.3 `modified` with no hunk: G1).
    for patch in [
        format!("{X}index @OLD@..@NEW@ 100644\nFiles a/x and b/x differ\n"),
        "diff --git a/n b/n\nnew file mode 100755\nindex @ZERO@..@NEW@\nFiles /dev/null and b/n differ\n".to_owned(),
    ] {
        let s = see(&r, &patch);
        assert!(s.git_tree().values().any(|e| e.content.as_deref() == Some(&b"stored\n"[..])));
        s.refused_as_unwritten();
    }
    // After a hunk it is no marker to Git, which applies the hunk: read, as by 0.8.4.
    let s = see(
        &r,
        &format!("{X}index @OLD@..@NEW@ 100644\n{HUNK}Files a/x and b/x differ\n"),
    );
    assert_eq!(s.status(), ("x".into(), FileStatus::Modified));
    assert_eq!(s.git_tree()["x"], file("b\n", false));
}

#[test]
fn row_traditional() {
    let r = rig("traditional");
    let s = see(&r, HUNK);
    assert_eq!(s.status(), ("x".into(), FileStatus::Modified));
    assert_eq!(s.git_tree()["x"], file("b\n", false));
    let s = see(&r, "--- /dev/null\n+++ b/n\n@@ -0,0 +1 @@\n+stored\n");
    assert_eq!(s.status(), ("n".into(), FileStatus::Added));
    assert_eq!(s.git_tree()["n"], file("stored\n", false));
    // With no hunk: not a patch to Git.
    let s = see(&r, "--- a/x\n+++ b/x\n");
    assert!(s.reading.is_none());
    s.refused(|e| matches!(e, PatchParseError::SectionWithoutPatch { .. }));
}

#[test]
fn row_no_line_feed() {
    let r = rig("no-line-feed");
    let s = see(&r, &format!("{X}old mode 100644\nnew mode 100755"));
    s.git_writes_nothing();
    s.refused(|e| matches!(e, PatchParseError::UnterminatedLine { .. }));
}

/// The table and the tests are held to each other: every row of `CONFORMANCE.md` has a test
/// named `row_<row>` here, and every such test names a row.
#[test]
fn every_row_has_a_test_and_every_test_a_row() {
    let table = include_str!("../CONFORMANCE.md");
    let rows: std::collections::BTreeSet<String> = table
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|rest| rest.split_once('`'))
        .map(|(row, _)| format!("row_{}", row.replace('-', "_")))
        .collect();
    let tests: std::collections::BTreeSet<String> = include_str!("conformance.rs")
        .lines()
        .filter_map(|line| line.trim().strip_prefix("fn row_"))
        .filter_map(|rest| rest.split_once('('))
        .map(|(name, _)| format!("row_{name}"))
        .collect();
    assert!(rows.len() >= 20, "{rows:?}");
    assert_eq!(rows, tests);
}
