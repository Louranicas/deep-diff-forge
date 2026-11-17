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
use deep_diff_forge_patch::{ParseOptions, PatchParseError, decode, old_path, parse_decoded};
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

/// The tree before every case: `x` holds `a`, `y` holds `y`, `e` is empty, `d/x` holds `dx`, and
/// `m` has two sections with one line in common.
fn before() -> Tree {
    [
        ("x", &b"a\n"[..]),
        ("y", b"y\n"),
        ("e", b""),
        ("d/x", b"dx\n"),
        ("m", b"[prod]\nallow = false\n[test]\nallow = false\n"),
    ]
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
    Repo::with_format(tag, &[b"a\n", b"stored\n", b"y\n", b"", b"dx\n"], "sha1")
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
    let (ok, _, after) = repo.apply_outcome(&before(), patch.as_bytes());
    assert_eq!(checks, ok, "--check and apply differ: {patch:?}");
    // Git applies a patch whole or not at all: a refusal writes nothing.
    assert!(
        ok || after == before(),
        "Git refused {patch:?} but wrote {after:?}"
    );
    let applied = ok.then_some(after);
    if let (Ok(files), Some(git)) = (&ddf, &applied) {
        let reviewed = reviewed_tree(&before(), files, &repo.contents)
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
    // A source with a doubled slash names the file Git reads, squashed (S3A-L-1).
    let s = see(
        &r,
        "diff --git a/d/x b/c\nsimilarity index 100%\ncopy from d//x\ncopy to c\n",
    );
    assert_eq!(s.git_tree()["c"], file("dx\n", false));
    assert_eq!(old_path(&s.read()[0]).as_deref(), Some("d/x"));
}

#[test]
fn row_copy_to() {
    let r = rig("copy-to");
    // Twice: Git writes the last; 0.8.4 read that too; refused now.
    let s = see(&r, &format!("{COPY}copy to k\n"));
    assert!(s.git_tree().contains_key("k") && !s.git_tree().contains_key("c"));
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
    // A source with a doubled slash: Git removes `d/x`, and the review names `d/x` (S3A-L-1;
    // 0.8.4 and 19a4cc6 showed `d//x`, so the oracle kept a `d/x` Git removed).
    let s = see(
        &r,
        "diff --git a/d/x b/r\nsimilarity index 100%\nrename from d//x\nrename to r\n",
    );
    assert_eq!(s.git_tree()["r"], file("dx\n", false));
    assert!(!s.git_tree().contains_key("d/x"));
    assert_eq!(old_path(&s.read()[0]).as_deref(), Some("d/x"));
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
    // A `---`/`+++` with no hunk after it, which Git never writes (an empty file is `new file
    // mode` and `index`): Git creates the empty file; refused now (0.8.4 read it).
    let s = see(
        &r,
        "diff --git a/n b/n\nnew file mode 100644\nindex @ZERO@..e69de29\n--- /dev/null\n+++ b/n\n",
    );
    assert_eq!(s.git_tree()["n"], file("", false));
    s.refused_as_unwritten();
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

const M: &str = "diff --git a/m b/m\nindex 1111111..2222222 100644\n--- a/m\n+++ b/m\n";

#[test]
fn row_hunk_placement() {
    let r = rig("hunk-placement");
    let refused = |s: &Seen| {
        s.refused(|e| matches!(e, PatchParseError::HunkPlacedElsewhere { .. }));
    };
    // `git diff -U0`: a hunk with no context, past line 1. Git places it at the end of the file
    // (`match_end`): line 4 changes, where the review showed line 2 (S3B-H-1).
    let s = see(
        &r,
        &format!("{M}@@ -2 +2 @@\n-allow = false\n+allow = true\n"),
    );
    assert_eq!(
        s.git_tree()["m"],
        file("[prod]\nallow = false\n[test]\nallow = true\n", false)
    );
    refused(&s);
    // An insertion with no context: Git appends it at the end of the file.
    let s = see(&r, &format!("{M}@@ -2,0 +3 @@\n+x = 1\n"));
    assert_eq!(
        s.git_tree()["m"],
        file(
            "[prod]\nallow = false\n[test]\nallow = false\nx = 1\n",
            false
        )
    );
    refused(&s);
    // A hunk after one with no trailing context, which no Git writes.
    let s = see(
        &r,
        &format!(
            "{M}@@ -1 +1 @@\n-[prod]\n+[stage]\n@@ -3,2 +3,2 @@\n [test]\n-allow = false\n+allow = true\n"
        ),
    );
    refused(&s);
    // At line 1 Git anchors a hunk with no context at both ends: on a longer file it refuses
    // the whole patch. Read; C1 allows the refusal.
    let s = see(&r, &format!("{M}@@ -1 +1 @@\n-[prod]\n+[stage]\n"));
    assert_eq!(s.status(), ("m".into(), FileStatus::Modified));
    s.git_writes_nothing();
    // The same for an insertion after line 1 (`u0-insert-after-line1`, `git diff -U0`): its old
    // side is empty, so Git would place it only in an empty file. Read; Git refuses it whole.
    let s = see(&r, &format!("{M}@@ -1,0 +2 @@\n+x = 1\n"));
    assert_eq!(s.status(), ("m".into(), FileStatus::Modified));
    s.git_writes_nothing();
    // A creation whose hunks all start at line 0, a hunk layout no Git writes: read, as 0.8.4
    // read it, because Git places none of it elsewhere. It writes the first hunk into the empty
    // file, then cannot anchor the second at line 0 and refuses the patch while checking it
    // (`see` holds `git apply --check` to the apply, and the tree to the one before).
    let s = see(
        &r,
        "diff --git a/n b/n\nnew file mode 100644\n--- /dev/null\n+++ b/n\n@@ -0,0 +1 @@\n+one\n@@ -0,0 +2 @@\n+two\n",
    );
    assert_eq!(s.status(), ("n".into(), FileStatus::Added));
    assert!(s.reading.is_some(), "Git reads it");
    s.git_writes_nothing();
    // `-U0` of a change to every line: one hunk, the whole file. Read, and written exactly.
    let s = see(
        &r,
        &format!(
            "{M}@@ -1,4 +1,2 @@\n-[prod]\n-allow = false\n-[test]\n-allow = false\n+[stage]\n+allow = true\n"
        ),
    );
    assert_eq!(s.git_tree()["m"], file("[stage]\nallow = true\n", false));
    // A last hunk with leading context only is what Git writes at the end of its base: read.
    let s = see(
        &r,
        &format!("{M}@@ -3,2 +3,2 @@\n [test]\n-allow = false\n+allow = true\n"),
    );
    assert_eq!(s.status(), ("m".into(), FileStatus::Modified));
    // With context on both sides the header's line is the place: read, and written there.
    let s = see(
        &r,
        &format!("{M}@@ -1,3 +1,3 @@\n [prod]\n-allow = false\n+allow = true\n [test]\n"),
    );
    assert_eq!(
        s.git_tree()["m"],
        file("[prod]\nallow = true\n[test]\nallow = false\n", false)
    );
}

/// Patches `git apply` fails to write after it has begun, or writes otherwise than the review
/// shows (S3B-M-1). Each is refused. Git deletes `x` and then fails on an empty or overlong
/// target and on a path that is a file and a directory; it writes a target with a NUL cut at
/// the NUL.
#[test]
fn shapes_git_fails_to_write_are_refused() {
    let r = rig("write-fails");
    let delete_x = "diff --git a/x b/x\ndeleted file mode 100644\nindex 1111111..0000000\n--- a/x\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\n";
    let cannot = |patch: &str| {
        let patch = patch
            .replace("@NEW@", &r.ids[1])
            .replace("@ZERO@", &"0".repeat(r.ids[1].len()));
        let ddf = parse_decoded(&decode(patch.as_bytes()), ParseOptions::default());
        assert!(
            matches!(ddf, Err(PatchParseError::GitCannotWrite { .. })),
            "{patch:?}: {ddf:?}"
        );
        r.apply_outcome(&before(), patch.as_bytes())
    };
    let symlink = |target: &str| {
        format!(
            "{delete_x}diff --git a/s b/s\nnew file mode 120000\nindex 0000000..1234567\n--- /dev/null\n+++ b/s\n@@ -0,0 +1 @@\n+{target}\n\\ No newline at end of file\n"
        )
    };
    // A new symlink with no content: an empty target. Git has deleted `x` when it fails.
    let (applied, _, after) = cannot(&format!(
        "{delete_x}diff --git a/s b/s\nnew file mode 120000\nindex 0000000..e69de29\n"
    ));
    assert!(!applied && !after.contains_key("x"), "{after:?}");
    let (applied, _, after) = cannot(&symlink(&"a".repeat(5000)));
    assert!(!applied && !after.contains_key("x"), "{after:?}");
    // A NUL: Git writes the link to `ab`, where the review showed `ab\0cd`.
    let (applied, _, after) = cannot(&symlink("ab\0cd"));
    assert!(applied, "{after:?}");
    // A symlink in a binary body, whose target DDF cannot read.
    cannot(&format!(
        "{delete_x}diff --git a/s b/s\nnew file mode 120000\nindex @ZERO@..@NEW@\nBinary files /dev/null and b/s differ\n"
    ));
    // One path as a file and as the directory of another: Git writes `a`, then fails.
    let (applied, _, after) = cannot(
        "diff --git a/a b/a\nnew file mode 100644\nindex 0000000..1234567\n--- /dev/null\n+++ b/a\n@@ -0,0 +1 @@\n+a\ndiff --git a/a/b b/a/b\nnew file mode 100644\nindex 0000000..1234567\n--- /dev/null\n+++ b/a/b\n@@ -0,0 +1 @@\n+b\n",
    );
    assert!(!applied && after.contains_key("a"), "{after:?}");
    let (applied, _, _) = cannot(&format!(
        "{X}index 1111111..2222222 100644\n{HUNK}diff --git a/x/b b/x/b\nnew file mode 100644\nindex 0000000..1234567\n--- /dev/null\n+++ b/x/b\n@@ -0,0 +1 @@\n+b\n"
    ));
    assert!(!applied);
    // A copy keeps its source as a file: Git cannot write a path under it, nor a file where its
    // directory is. It has deleted `x` when it fails.
    let copy = |from: &str| format!("similarity index 100%\ncopy from {from}\ncopy to c\n");
    let new = |path: &str| {
        format!(
            "diff --git a/{path} b/{path}\nnew file mode 100644\nindex 0000000..1234567\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1 @@\n+b\n"
        )
    };
    for patch in [
        format!("{delete_x}diff --git a/y b/c\n{}{}", copy("y"), new("y/b")),
        format!(
            "{delete_x}{}diff --git a/d/x b/c\n{}",
            new("d"),
            copy("d/x")
        ),
    ] {
        let (applied, _, after) = cannot(&patch);
        assert!(!applied && !after.contains_key("x"), "{after:?}");
    }
    // Git-written (`git diff -C --find-copies-harder`): `y` copied and renamed away, and `y/b`
    // created. The rename removes `y` first, so Git writes it all: read, and written as reviewed.
    let s = see(
        &r,
        &format!(
            "{}diff --git a/y b/c\n{}diff --git a/y b/r\nsimilarity index 100%\nrename from y\nrename to r\n",
            new("y/b"),
            copy("y")
        ),
    );
    assert_eq!(s.git_tree()["y/b"], file("b\n", false));
    assert_eq!(s.read().len(), 3);
}

/// `git diff -w` (or `-b`, `-I`, `--ignore-blank-lines`, a textconv driver) of a rename, a copy
/// or a mode change whose edits it hides: Git writes the header with `index` and no body, and
/// `git apply` makes the move or the mode change alone (S3B-M-2; 0.8.4 read them, `19a4cc6` and
/// `905e885` refused them). Each file has its own content, so Git pairs each with its source.
#[test]
fn git_written_moves_and_mode_changes_with_no_body_are_read() {
    let r = Repo::with_format("ws-shapes", &[], "sha1");
    let text = |name: &str| format!("{name}\n  two\n    three\nfour\nfive\nsix\n");
    let in_repo = |name: &str| r.root().join(name);
    let exec = |name: &str, on: bool| {
        std::fs::set_permissions(
            in_repo(name),
            std::os::unix::fs::PermissionsExt::from_mode(if on { 0o755 } else { 0o644 }),
        )
        .unwrap();
    };
    // (source, result, executable after): a rename, a rename with a mode change, two copies
    // (one with a mode change) and a mode change alone.
    let cases = [
        ("mv", "moved", false),
        ("both", "both2", true),
        ("src", "copy", false),
        ("srcx", "copyx", true),
        ("run.sh", "run.sh", true),
    ];
    for (from, _, _) in cases {
        std::fs::write(in_repo(from), text(from)).unwrap();
        exec(from, false);
    }
    commit(&r, ".", "base");
    for (from, to, x) in cases {
        let g = |args: &[&str]| assert!(r.git(args).status.success(), "{args:?}");
        if from != to && !from.starts_with("src") {
            g(&["mv", from, to]);
        }
        std::fs::write(in_repo(to), text(from).replace("  ", "\t")).unwrap();
        exec(to, x);
    }
    commit(&r, ".", "moves");
    let patch = r
        .git(&[
            "diff",
            "-w",
            "-M",
            "-C",
            "--find-copies-harder",
            "HEAD~1",
            "HEAD",
        ])
        .stdout;
    let shown = String::from_utf8_lossy(&patch).into_owned();
    assert!(!shown.contains("@@"), "{shown}");
    let files = parse_decoded(&decode(&patch), ParseOptions::default())
        .unwrap_or_else(|e| panic!("DDF refuses Git's own -w diff: {e}\n{shown}"));
    let mut statuses: Vec<_> = files.iter().map(|f| (f.path.clone(), f.status)).collect();
    statuses.sort_by(|a, b| a.0.cmp(&b.0));
    let want = [
        ("both2", FileStatus::Renamed),
        ("copy", FileStatus::Copied),
        ("copyx", FileStatus::Copied),
        ("moved", FileStatus::Renamed),
        ("run.sh", FileStatus::ModeChanged),
    ];
    assert_eq!(statuses, want.map(|(p, s)| (p.to_owned(), s)), "{shown}");
    assert!(r.git(&["checkout", "-q", "HEAD~1"]).status.success());
    let name = r.root().join(".git/ws.patch");
    std::fs::write(&name, &patch).unwrap();
    assert!(r.git(&["apply", name.to_str().unwrap()]).status.success());
    for (from, to, x) in cases {
        assert_eq!(
            std::fs::read_to_string(in_repo(to)).unwrap(),
            text(from),
            "{to}"
        );
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(in_repo(to)).unwrap().permissions(),
        );
        assert_eq!(mode & 0o100 != 0, x, "{to}");
    }
}

/// Git with an identity, for the commits the presentations below are written from.
fn commit(repo: &Repo, dir: &str, message: &str) {
    for args in [
        vec!["-C", dir, "add", "-A"],
        vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "-C",
            dir,
            "commit",
            "-qm",
            message,
        ],
    ] {
        let out = repo.git(&args);
        assert!(out.status.success(), "{args:?}: {out:?}");
    }
}

/// The work tree as `support` reads it, without `.git` and without the nested repository's.
fn tree_of(repo: &Repo) -> Tree {
    let mut tree = Tree::new();
    for path in ["x", "sm/x", "a.bin", "b.bin"] {
        if let Ok(bytes) = std::fs::read(repo.root().join(path)) {
            tree.insert(path.to_owned(), regular(&bytes));
        }
    }
    tree
}

/// Asserts that DDF reads `patch` and that a real `git apply` refuses it whole: it writes
/// nothing, so the review shows a change that never lands, and none that lands otherwise
/// (C1 in its two-outcome form, `CONFORMANCE.md`).
fn read_and_refused_whole(repo: &Repo, patch: &[u8], why: &str) -> Vec<ReviewFile> {
    let files = parse_decoded(&decode(patch), ParseOptions::default())
        .unwrap_or_else(|e| panic!("DDF refuses Git's own {why}: {e}"));
    let before = tree_of(repo);
    let name = repo.root().join(".git/presentation.patch");
    std::fs::write(&name, patch).unwrap();
    let out = repo.git(&["apply", name.to_str().unwrap()]);
    assert!(!out.status.success(), "git apply accepts the {why}");
    assert_eq!(tree_of(repo), before, "git apply wrote part of the {why}");
    files
}

/// Three presentations Git's own diff writes that `git apply` refuses whole (`CONFORMANCE.md`,
/// "Git writes it" against "`git apply` accepts it"): DDF reads each as the change it shows,
/// as 0.8.4 did, and Git writes nothing from it. The generated differential counts each.
#[test]
fn git_written_presentations_git_apply_refuses_whole() {
    // (a) A plain `git diff` of a binary file: abbreviated ids, which `apply_binary` refuses
    // ("without full index line"). In SHA-1 and SHA-256 repositories; with `--full-index` the
    // same change applies, and Git writes the new blob.
    for format in ["sha1", "sha256"] {
        let r = Repo::with_format(&format!("abbrev-{format}"), &[], format);
        std::fs::write(r.root().join("x"), b"old\0content\n").unwrap();
        commit(&r, ".", "old");
        std::fs::write(r.root().join("x"), b"new\0content\n").unwrap();
        let plain = r.git(&["diff"]).stdout;
        let full = r.git(&["diff", "--full-index"]).stdout;
        assert!(r.git(&["hash-object", "-w", "x"]).status.success());
        assert!(r.git(&["checkout", "--", "x"]).status.success());
        let files = read_and_refused_whole(&r, &plain, &format!("{format} binary diff"));
        assert_eq!(files[0].status, FileStatus::BinaryChanged);
        let name = r.root().join(".git/full.patch");
        std::fs::write(&name, &full).unwrap();
        assert!(r.git(&["apply", name.to_str().unwrap()]).status.success());
        assert_eq!(
            std::fs::read(r.root().join("x")).unwrap(),
            b"new\0content\n"
        );
    }
    // (b) `git diff --submodule=diff`: a `Submodule sm …` line, then the submodule's own section
    // under the superproject path. In the repository it describes, `sm/x` already holds the new
    // content, so Git refuses the section.
    let r = Repo::with_format("submodule-diff", &[], "sha1");
    std::fs::create_dir(r.root().join("sm")).unwrap();
    assert!(r.git(&["-C", "sm", "init", "-q"]).status.success());
    std::fs::write(r.root().join("sm/x"), b"a\n").unwrap();
    commit(&r, "sm", "s1");
    commit(&r, ".", "parent");
    std::fs::write(r.root().join("sm/x"), b"b\n").unwrap();
    commit(&r, "sm", "s2");
    let patch = r.git(&["diff", "--submodule=diff"]).stdout;
    assert!(patch.starts_with(b"Submodule sm "), "{patch:?}");
    let files = read_and_refused_whole(&r, &patch, "submodule diff");
    assert_eq!(files[0].path, "sm/x");
    // (c) `git diff --no-index` of two binary files: `diff --git` halves that name two paths,
    // with nothing else naming the files, so Git names no file ("lacks filename information").
    let r = Repo::with_format("no-index", &[], "sha1");
    std::fs::write(r.root().join("a.bin"), b"a\0b").unwrap();
    std::fs::write(r.root().join("b.bin"), b"a\0c").unwrap();
    let patch = r.git(&["diff", "--no-index", "a.bin", "b.bin"]).stdout;
    let files = read_and_refused_whole(&r, &patch, "--no-index pair");
    assert_eq!(files[0].status, FileStatus::BinaryChanged);
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
