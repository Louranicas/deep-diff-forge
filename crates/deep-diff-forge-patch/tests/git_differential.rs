//! Differential oracle: a patch DDF accepts is the patch `git apply` reads. For every accepted
//! input, Git's `--numstat` (each file Git would write, with its added and deleted line counts)
//! must equal the same projection of DDF's review. A reading that differs shows a reviewer one
//! change while Git applies another, which is the one failure patch truth exists to prevent.
//!
//! Production readiness 2026-11-04: `@@-1,1 +1,1 @@` (no space after `@@`) was read by DDF as
//! a hunk of the current file, so the `--- a/evil` / `+++ b/evil` lines after it were reviewed as
//! that hunk's body, while Git ended the file there and wrote `+pwned` into `evil`. The cases
//! below are generated rather than listed, so a later leniency anywhere in the header grammar or
//! in hunk placement fails here whether or not anyone thought of it.
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real git processes and reads the environment"
)]

use deep_diff_forge_core::{FileStatus, PatchLineKind, ReviewFile};
use deep_diff_forge_patch::{ParseOptions, PatchParseError, decode, parse_decoded};
use std::{fs, path::Path, process::Command};

/// A scratch Git repository, removed when dropped (also when an assertion fails).
struct Scratch(deep_diff_forge_testkit::Scratch);

impl Scratch {
    fn new(tag: &str) -> Self {
        let root = deep_diff_forge_testkit::Scratch::new(&format!("git-diff-{tag}"));
        let init = Command::new("git")
            .current_dir(root.path())
            .args(["init", "-q"])
            .output()
            .unwrap();
        assert!(init.status.success(), "git init failed");
        Self(root)
    }
}

/// One file as `git apply --numstat -z` reports it: the added and deleted line counts (`None`
/// for a binary file, which Git prints as `-`) and the path Git writes, as raw bytes.
type Numstat = (Option<(usize, usize)>, Vec<u8>);

/// Git's reading of `patch`, or `None` when Git refuses it.
fn git_numstat(root: &Path, patch: &[u8]) -> Option<Vec<Numstat>> {
    fs::write(root.join("probe.patch"), patch).unwrap();
    let out = Command::new("git")
        .current_dir(root)
        .args(["apply", "--numstat", "-z", "probe.patch"])
        .output()
        .unwrap();
    if !out.status.success() {
        return None;
    }
    let records = out
        .stdout
        .split(|&b| b == 0)
        .filter(|record| !record.is_empty())
        .map(|record| {
            let mut fields = record.splitn(3, |&b| b == b'\t');
            let mut count = || {
                let field = fields.next().expect("numstat field");
                (field != b"-").then(|| std::str::from_utf8(field).unwrap().parse().unwrap())
            };
            let (added, deleted) = (count(), count());
            let written = fields.next().expect("numstat path").to_vec();
            (added.zip(deleted), written)
        })
        .collect();
    Some(records)
}

/// DDF's review projected as Git's numstat.
fn ddf_numstat(files: &[ReviewFile]) -> Vec<Numstat> {
    files
        .iter()
        .map(|file| {
            let binary =
                file.status == FileStatus::BinaryChanged || !file.patch_twin.binary_body.is_empty();
            let lines = || file.patch_twin.hunks.iter().flat_map(|h| &h.lines);
            let count = |kind| lines().filter(|l| l.kind == kind).count();
            let counts =
                (!binary).then(|| (count(PatchLineKind::Added), count(PatchLineKind::Removed)));
            (counts, file.path.as_bytes().to_vec())
        })
        .collect()
}

/// The property. Returns whether DDF accepted `patch`; when it did, Git must read the same files
/// with the same counts.
fn reads_as_git_does(root: &Path, patch: &[u8]) -> bool {
    let decoded = decode(patch);
    let Ok(files) = parse_decoded(&decoded, ParseOptions::default()) else {
        return false;
    };
    assert_eq!(
        git_numstat(root, patch),
        Some(ddf_numstat(&files)),
        "DDF accepted a patch that Git reads differently: {:?}",
        String::from_utf8_lossy(patch)
    );
    true
}

/// Every committed patch fixture and fuzz regression input.
#[test]
fn every_patch_fixture_reads_as_git_reads_it() {
    let scratch = Scratch::new("fixtures");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/patch");
    let mut pending = vec![fixtures];
    let (mut seen, mut accepted) = (0, 0);
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "patch") {
                seen += 1;
                accepted += usize::from(reads_as_git_does(
                    scratch.0.path(),
                    &fs::read(&path).unwrap(),
                ));
            }
        }
    }
    // Not vacuous: the fixtures are read, and most are patches DDF reviews.
    assert!(
        seen >= 20 && accepted >= 8,
        "seen {seen}, accepted {accepted}"
    );
}

/// The header that opens the hidden file's change, and the hunk of `x` before it.
const VISIBLE: &str = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";
const HIDDEN: &str = "--- a/evil\n+++ b/evil\n@@ -1 +1 @@\n-a\n+pwned\n";

/// Every one-character edit of a hunk header: an insertion of each character that matters to the
/// grammar at each position, each deletion, and each replacement by a space or a tab.
fn header_mutants(header: &str) -> Vec<String> {
    let chars: Vec<char> = header.chars().collect();
    let mut out = Vec::new();
    for at in 0..=chars.len() {
        for c in [' ', '\t', '@', '-', '+', ',', '0', '9', 'x'] {
            let mut edited = chars.clone();
            edited.insert(at, c);
            out.push(edited.into_iter().collect());
        }
    }
    for at in 0..chars.len() {
        for replacement in [None, Some(' '), Some('\t')] {
            let mut edited = chars.clone();
            match replacement {
                None => {
                    edited.remove(at);
                }
                Some(c) => edited[at] = c,
            }
            out.push(edited.into_iter().collect());
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn no_hunk_header_mutant_hides_a_file_from_the_review() {
    let scratch = Scratch::new("mutants");
    // Control: the unmutated header is a hunk of `x` to both readers, so `evil` is read as
    // removed and added lines of `x` and no file is hidden.
    let control = format!("{VISIBLE}@@ -1,1 +1,1 @@\n{HIDDEN}");
    assert!(reads_as_git_does(scratch.0.path(), control.as_bytes()));
    // The finding itself: Git writes `evil` from this patch, DDF must not accept it.
    let finding = format!("{VISIBLE}@@-1,1 +1,1 @@\n{HIDDEN}");
    let git = git_numstat(scratch.0.path(), finding.as_bytes()).expect("git applies the finding");
    assert!(git.iter().any(|(_, path)| path == b"evil"), "{git:?}");
    assert!(!reads_as_git_does(scratch.0.path(), finding.as_bytes()));

    let mutants = header_mutants("@@ -1,1 +1,1 @@");
    let (mut accepted, mut refused) = (0, 0);
    for mutant in &mutants {
        // As the second hunk of `x`, in front of the hidden file, and as the first hunk.
        for patch in [
            format!("{VISIBLE}{mutant}\n{HIDDEN}"),
            format!("diff --git a/x b/x\n--- a/x\n+++ b/x\n{mutant}\n-a\n+b\n{HIDDEN}"),
        ] {
            if reads_as_git_does(scratch.0.path(), patch.as_bytes()) {
                accepted += 1;
            } else {
                refused += 1;
            }
        }
    }
    // Not vacuous: some edits are still valid headers (`@@ -10,1 …`) and most are not.
    assert!(mutants.len() > 150, "{}", mutants.len());
    assert!(
        accepted > 10 && refused > 100,
        "accepted {accepted}, refused {refused}"
    );
}

/// A file's hunks are one unbroken run, as Git reads them; Git's verdict is asked for each case.
#[test]
fn hunks_follow_their_file_header_or_the_previous_hunk() {
    let scratch = Scratch::new("placement");
    let hunk = "@@ -1 +1 @@\n-a\n+b\n";
    let later = "@@ -3 +3 @@\n-c\n+d\n";
    let git_refuses = [
        format!("--- a/x\n+++ b/x\n\n{hunk}"),
        format!("--- a/x\n+++ b/x\ngarbage\n{hunk}"),
        format!("diff --git a/x b/x\n--- a/x\n+++ b/x\n{hunk}\n{later}"),
        format!("diff --git a/x b/x\n--- a/x\n+++ b/x\n{hunk}not a header\n{later}"),
        format!("diff --git a/x b/x\n{hunk}"),
        format!("commit message\n{hunk}"),
    ];
    for patch in &git_refuses {
        assert_eq!(
            git_numstat(scratch.0.path(), patch.as_bytes()),
            None,
            "git: {patch:?}"
        );
        let decoded = decode(patch.as_bytes());
        assert!(
            matches!(
                parse_decoded(&decoded, ParseOptions::default()),
                Err(PatchParseError::MisplacedHunk { .. })
            ),
            "ddf: {patch:?}"
        );
    }
    let git_accepts = [
        format!("diff --git a/x b/x\n--- a/x\n+++ b/x\n{hunk}{later}"),
        format!("diff --git a/x b/x\nindex 1..2 100644\n{hunk}"),
        format!("diff --git a/x b/x\nold mode 100644\nnew mode 100755\n{hunk}"),
        // A hunk may follow the `\ No newline` marker that ends the previous one (here it
        // ends the new side, and the next hunk only removes).
        "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n\\ No newline at end of file\n@@ -3 +2,0 @@\n-c\n"
            .to_owned(),
        format!("--- a/x\n+++ b/x\n{hunk}--- a/y\n+++ b/y\n{hunk}"),
    ];
    for patch in &git_accepts {
        assert!(
            reads_as_git_does(scratch.0.path(), patch.as_bytes()),
            "ddf refused {patch:?}"
        );
    }
}

/// A section from which Git reads no patch is refused; the header-only sections Git does read
/// (an extended header, or a Git section's `---`/`+++`) are reviewed as no change.
#[test]
fn a_section_git_reads_no_patch_from_is_refused() {
    let scratch = Scratch::new("sections");
    for patch in [
        "diff --git a/x b/x\n",
        "--- a/x\n+++ b/x\n",
        "diff --git a/x b/x\ndiff --git a/y b/y\nindex 1..2 100644\n--- a/y\n+++ b/y\n@@ -1 +1 @@\n-a\n+b\n",
    ] {
        assert_eq!(
            git_numstat(scratch.0.path(), patch.as_bytes()),
            None,
            "git: {patch:?}"
        );
        assert!(
            matches!(
                parse_decoded(&decode(patch.as_bytes()), ParseOptions::default()),
                Err(PatchParseError::SectionWithoutPatch { .. })
            ),
            "ddf: {patch:?}"
        );
    }
    assert!(matches!(
        parse_decoded(
            &decode(b"diff --git a/x b/x\n--- a/x\n+++ b/x\n"),
            ParseOptions::default()
        ),
        Err(PatchParseError::SectionWithoutPatch { .. })
    ));
    for patch in [
        "diff --git a/x b/x\nindex 1..2 100644\n",
        "diff --git a/e b/e\nnew file mode 100644\nindex 0000000..e69de29\n",
        "diff --git a/x b/x\nindex 1..2 100644\n--- a/y\n+++ b/y\n@@ -1 +1 @@\n-a\n+b\n",
    ] {
        // Accepted or refused, never read differently from Git.
        reads_as_git_does(scratch.0.path(), patch.as_bytes());
    }
    assert!(reads_as_git_does(
        scratch.0.path(),
        b"diff --git a/x b/x\nindex 1..2 100644\n"
    ));
}

#[test]
fn hunks_that_change_nothing_are_refused_as_git_refuses_them() {
    let scratch = Scratch::new("nochange");
    for patch in [
        "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n a\n",
        "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -0,0 +0,0 @@\n",
        "--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n a\n b\n",
    ] {
        assert_eq!(
            git_numstat(scratch.0.path(), patch.as_bytes()),
            None,
            "git: {patch:?}"
        );
        let decoded = decode(patch.as_bytes());
        assert!(
            matches!(
                parse_decoded(&decoded, ParseOptions::default()),
                Err(PatchParseError::HunkWithoutChange { .. })
            ),
            "ddf: {patch:?}"
        );
    }
    // Control: one changed line among context is a change.
    let changed = "--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n a\n-b\n+c\n";
    assert!(reads_as_git_does(scratch.0.path(), changed.as_bytes()));
}

#[test]
fn a_header_that_is_not_utf8_is_refused_but_preamble_and_content_are_not() {
    let scratch = Scratch::new("lossy");
    // Two Latin-1 names that decode to one `caf\u{fffd}`: Git writes different files.
    for patch in [
        &b"--- a/caf\xe9\n+++ b/caf\xe9\n@@ -1 +1 @@\n-a\n+b\n"[..],
        &b"diff --git a/caf\xe8 b/caf\xe8\nindex 1..2 100644\n@@ -1 +1 @@\n-a\n+b\n"[..],
        &b"diff --git a/x b/y\nsimilarity index 90%\nrename from x\nrename to caf\xe9\n"[..],
    ] {
        assert!(
            matches!(
                parse_decoded(&decode(patch), ParseOptions::default()),
                Err(PatchParseError::LossyHeader { .. })
            ),
            "ddf: {:?}",
            String::from_utf8_lossy(patch)
        );
    }
    // A Latin-1 commit message and Latin-1 content are reviewed (content marked lossy), and Git
    // reads the same files.
    let mail = b"From: Ren\xe9\nSubject: caf\xe9\n\n---\ndiff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-caf\xe9\n+cafe\n";
    assert!(reads_as_git_does(scratch.0.path(), mail));
}
