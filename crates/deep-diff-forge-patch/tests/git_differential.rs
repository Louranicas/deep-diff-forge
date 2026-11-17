// DDF integration: R03, R07, R46. Differential oracle for the parser: every patch DDF accepts must read as `git apply` reads it (paths, operations and modes), which round-trip equality does not show.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-patch--tests--git_differential
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

/// One line of `git apply --summary` that numstat cannot show: a copy and the path it writes, a
/// created or deleted file with its mode (a symlink or an executable is created by its mode
/// alone), or a mode change (`<move>` for a moved file's, which Git prints with
/// no name). Paths are as Git prints them with `core.quotepath=false`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Summary {
    Copy { from: String, to: String },
    Create { mode: u32, path: String },
    Delete { mode: u32, path: String },
    Mode { old: u32, new: u32, path: String },
}

/// `pprint_rename`'s `pre{old => new}post`, or `old => new`, as the two whole paths.
fn expand_move(text: &str) -> (String, String) {
    if let (Some(open), Some(close)) = (text.find('{'), text.rfind('}'))
        && let Some((old, new)) = text[open + 1..close].split_once(" => ")
    {
        let (pre, post) = (&text[..open], &text[close + 1..]);
        let join = |mid: &str| format!("{pre}{mid}{post}").replace("//", "/");
        return (join(old), join(new));
    }
    let (old, new) = text.split_once(" => ").unwrap_or((text, text));
    (old.to_owned(), new.to_owned())
}

/// Git's summary of `patch`, or `None` when Git refuses it or quotes a path (whose quoting this
/// projection does not reproduce; numstat still compares those).
fn git_summary(root: &Path, patch: &[u8]) -> Option<Vec<Summary>> {
    fs::write(root.join("probe.patch"), patch).unwrap();
    let out = Command::new("git")
        .current_dir(root)
        .args([
            "-c",
            "core.quotepath=false",
            "apply",
            "--summary",
            "probe.patch",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).ok()?;
    if !out.status.success() || text.contains('"') {
        return None;
    }
    let mut lines = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("copy ") {
            let rest = rest.rsplit_once(" (").map_or(rest, |(moved, _)| moved);
            let (from, to) = expand_move(rest);
            lines.push(Summary::Copy { from, to });
        } else if let Some(rest) = line.strip_prefix("create mode ") {
            let (mode, name) = rest.split_once(' ')?;
            lines.push(Summary::Create {
                mode: u32::from_str_radix(mode, 8).ok()?,
                path: name.to_owned(),
            });
        } else if let Some(name) = line.strip_prefix("create ") {
            lines.push(Summary::Create {
                mode: 0,
                path: name.to_owned(),
            });
        } else if let Some(rest) = line.strip_prefix("delete mode ") {
            let (mode, name) = rest.split_once(' ')?;
            lines.push(Summary::Delete {
                mode: u32::from_str_radix(mode, 8).ok()?,
                path: name.to_owned(),
            });
        } else if let Some(name) = line.strip_prefix("delete ") {
            lines.push(Summary::Delete {
                mode: 0,
                path: name.to_owned(),
            });
        } else if let Some(rest) = line.strip_prefix("mode change ") {
            let (old, rest) = rest.split_once(" => ")?;
            let (new, named) = rest.split_once(' ').unwrap_or((rest, "<move>"));
            let octal = |mode: &str| u32::from_str_radix(mode, 8).ok();
            lines.push(Summary::Mode {
                old: octal(old)?,
                new: octal(new)?,
                path: named.to_owned(),
            });
        }
    }
    lines.sort();
    Some(lines)
}

/// DDF's review projected as Git's summary.
fn ddf_summary(files: &[ReviewFile]) -> Vec<Summary> {
    let mut lines = Vec::new();
    for file in files {
        let meta = &file.patch_twin.metadata;
        let says = |prefix: &str| meta.iter().any(|m| m.starts_with(prefix));
        let mode = |prefix: &str| {
            meta.iter()
                .find_map(|m| m.strip_prefix(prefix))
                .and_then(|m| u32::from_str_radix(m.trim(), 8).ok())
        };
        // `summary_patch_list` names no path for a moved file's mode change, nor for one in a
        // section with a similarity score (it prints a rewrite, then the mode change).
        let moved = says("rename from ")
            || says("copy from ")
            || says("similarity index ")
            || says("dissimilarity index ");
        // A binary copy keeps the `binary_changed` status, with the copy in its metadata and
        // `old_path`.
        if file.status == FileStatus::Copied
            || (file.status == FileStatus::BinaryChanged && says("copy from "))
        {
            lines.push(Summary::Copy {
                from: deep_diff_forge_patch::old_path(file).unwrap_or_default(),
                to: file.path.clone(),
            });
        }
        // The mode is the one on the file's own line, as Git canonicalises it (`canon_mode`:
        // the owner's execute bit picks 755 or 644 for a regular file). A traditional diff has
        // no mode line, and Git prints ` create x` with no mode; that is `0` on both sides.
        let canon = |mode: u32| match mode & 0o170_000 {
            0o100_000 if mode & 0o100 != 0 => 0o100_755,
            0o100_000 => 0o100_644,
            0o120_000 => 0o120_000,
            0o040_000 => 0o040_000,
            _ => 0o160_000,
        };
        if file.status == FileStatus::Added || says("new file mode ") {
            lines.push(Summary::Create {
                mode: mode("new file mode ").map_or(0, canon),
                path: file.path.clone(),
            });
        }
        if file.status == FileStatus::Deleted || says("deleted file mode ") {
            lines.push(Summary::Delete {
                mode: mode("deleted file mode ").map_or(0, canon),
                path: file.path.clone(),
            });
        }
        if let (Some(old), Some(new)) = (mode("old mode "), mode("new mode "))
            && old != new
        {
            let path = if moved {
                "<move>".to_owned()
            } else {
                file.path.clone()
            };
            lines.push(Summary::Mode { old, new, path });
        }
    }
    lines.sort();
    lines
}

/// The property. Returns whether DDF accepted `patch`; when it did, Git must read the same files
/// with the same counts, and the same copies, creations, deletions and mode changes.
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
    if let Some(git) = git_summary(root, patch) {
        assert_eq!(
            git,
            ddf_summary(&files),
            "DDF accepted a patch whose copies, creations or modes Git reads differently: {:?}",
            String::from_utf8_lossy(patch)
        );
    }
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

/// Git sections whose extended headers the mutants below break: a pure rename, a mode change
/// with a hunk, a new file and an indexed modification.
const GIT_SECTIONS: [&str; 4] = [
    "diff --git a/old/name.rs b/old/name.rs\nsimilarity index 100%\nrename from old/name.rs\nrename to src/x.rs\n",
    "diff --git a/x b/x\nold mode 100644\nnew mode 100755\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n",
    "diff --git a/n b/n\nnew file mode 100644\nindex 0000000..1111111\n--- /dev/null\n+++ b/n\n@@ -0,0 +1 @@\n+n\n",
    "diff --git a/x b/x\nindex 1111111..2222222 100644\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n",
];

/// Lines that end a Git extended header, and header lines placed after the end.
const BREAKS: [&str; 3] = ["", "See notes", "---"];
const LATE_HEADERS: [&str; 6] = [
    "rename to .github/workflows/pwn.yml",
    "rename from elsewhere",
    "new mode 100755",
    "deleted file mode 100644",
    "--- a/evil\n+++ b/evil",
    "@@ -1 +1 @@\n-a\n+b",
];

/// Independent rescore of v0.8.0 (F1 to F3): Git ends a `diff --git` section's extended header at
/// the first line it does not read as one (`parse_git_diff_header`), and at a last line with no
/// line feed. DDF read on, so a `rename to` after a blank line or a note named the reviewed
/// destination while Git wrote the earlier one. Every break of every header, followed by each
/// kind of late header line, must be refused or read exactly as Git reads it.
#[test]
fn no_extended_header_mutant_reads_differently_from_git() {
    let scratch = Scratch::new("ext-header");
    let root = scratch.0.path();
    // The findings, each applied by Git to a file the review did not show.
    let rename = "diff --git a/old/name.rs b/old/name.rs\nsimilarity index 100%\nrename from old/name.rs\nrename to .github/workflows/pwn.yml\n";
    let findings = [
        (format!("{rename}See notes\nrename to docs/harmless.md\n"), &b".github/workflows/pwn.yml"[..]),
        (format!("{rename}\nrename to docs/harmless.md\n"), b".github/workflows/pwn.yml"),
        (
            "diff --git a/old/name.rs b/old/name.rs\nsimilarity index 100%\nrename from old/name.rs\nrename to .github/workflows/pwn.yml\nrename to docs/harmless.md"
                .to_owned(),
            b".github/workflows/pwn.yml",
        ),
        (
            "diff --git a/tool.py b/tool.py\n\n--- a/too\t.py\n+++ b/tool.py\n@@ -1 +1,2 @@\n import os\n+evil()\n"
                .to_owned(),
            b"too",
        ),
        (
            "diff --git a/gone.txt b/gone.txt\n--- a/gone.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-obsolete\n"
                .to_owned(),
            b"dev/null",
        ),
    ];
    for (patch, written) in &findings {
        let git = git_numstat(root, patch.as_bytes()).expect("git applies the finding");
        assert!(git.iter().any(|(_, path)| path == written), "{git:?}");
        assert!(!reads_as_git_does(root, patch.as_bytes()), "{patch:?}");
    }
    let (mut accepted, mut refused, mut cases) = (0, 0, 0);
    for section in GIT_SECTIONS {
        let lines: Vec<&str> = section.lines().collect();
        // The header is the lines before the first `---` or hunk.
        let header_end = lines
            .iter()
            .position(|l| l.starts_with("--- ") || l.starts_with("@@"))
            .unwrap_or(lines.len());
        for at in 1..=header_end {
            for brk in BREAKS {
                for late in std::iter::once(None).chain(LATE_HEADERS.map(Some)) {
                    let mut mutant: Vec<&str> = lines[..at].to_vec();
                    mutant.push(brk);
                    mutant.extend(late);
                    mutant.extend(&lines[at..]);
                    let patch = mutant.join("\n") + "\n";
                    cases += 1;
                    if reads_as_git_does(root, patch.as_bytes()) {
                        accepted += 1;
                    } else {
                        refused += 1;
                    }
                }
            }
        }
        // The section's last line without its line feed.
        cases += 1;
        if reads_as_git_does(root, section.trim_end_matches('\n').as_bytes()) {
            accepted += 1;
        } else {
            refused += 1;
        }
    }
    // Not vacuous: hundreds of cases, with Git-identical acceptances and refusals both seen.
    assert!(cases > 250, "{cases}");
    assert!(
        accepted > 0 && refused > 100,
        "accepted {accepted}, refused {refused}"
    );
}

/// Header lines a mutant adds to a Git section: each kind of extended header, `---`/`+++` names
/// that `-p1` leaves nothing of, and names that differ from the section's own.
const EXTRA_HEADERS: [&str; 18] = [
    "Binary files a/x and b/x differ",
    "GIT binary patch",
    "rename to build.rs\rdocs/notes.md",
    "copy to x\x0bdocs/y",
    "copy to .github/workflows/pwn.yml",
    "copy from elsewhere",
    "rename to .github/workflows/pwn.yml",
    "old mode 100644",
    "old mode 100755",
    "new mode 100755",
    "new mode 100644",
    "index 1111111..2222222 100644",
    "index 1111111..2222222 100755",
    "similarity index 100%",
    "--- README\n+++ README",
    "--- a/x\n+++ README",
    "--- a/x\n+++ b/elsewhere",
    "--- a/elsewhere\n+++ /dev/null",
];

/// Independent rescore of v0.8.1 (N1 to N3): Git takes a copy's new path from its last `copy to`
/// line, a `---`/`+++` name with nothing left after `-p1` is no name to Git (it keeps the
/// `diff --git` one), and Git keeps the last of each mode header, an `index` mode being an old
/// mode. DDF read the `diff --git` name for the copy, the bare `README` for the file, and the
/// first mode, so each showed a different file or no mode change while Git wrote the workflow or
/// made the script executable. Every extra header at every position of every section must be
/// refused or read exactly as Git reads it, copies and modes included.
#[test]
fn no_duplicated_or_copy_header_mutant_reads_differently_from_git() {
    let scratch = Scratch::new("copy-mode");
    let root = scratch.0.path();
    let findings = [
        // N1: the copy writes the workflow, with the content of the hunk.
        "diff --git a/README.md b/docs/README-copy.md\nsimilarity index 50%\ncopy from README.md\ncopy to .github/workflows/pwn.yml\n@@ -1 +1,2 @@\n-hello\n+on: push\n+jobs: {}\n",
        "diff --git a/x b/docs/a.md\nsimilarity index 100%\ncopy from x\ncopy to docs/a.md\ncopy to .github/workflows/pwn.yml\n",
        // N2: Git changes the workflow named by `diff --git`.
        "diff --git a/.github/workflows/ci.yml b/.github/workflows/ci.yml\n--- README\n+++ README\n@@ -1 +1 @@\n-jobs: {}\n+jobs: {pwn: 1}\n",
        // N3: Git makes `run.sh` executable; the first mode lines say it already was.
        "diff --git a/run.sh b/run.sh\nold mode 100755\nnew mode 100755\nindex 1111111..2222222 100644\n--- a/run.sh\n+++ b/run.sh\n@@ -1 +1 @@\n-echo hi\n+echo bye\n",
        "diff --git a/run.sh b/run.sh\nold mode 100644\nnew mode 100644\nnew mode 100755\n--- a/run.sh\n+++ b/run.sh\n@@ -1 +1 @@\n-echo hi\n+echo bye\n",
    ];
    // Each is read as Git reads it (the copies, whose last `copy to` is now the reviewed path)
    // or refused (the nameless `---`/`+++` and the repeated modes, forms no Git writes). The
    // v0.8.1 parser read every one differently, which `reads_as_git_does` fails on.
    let mut copies = 0;
    for patch in findings {
        assert!(
            git_numstat(root, patch.as_bytes()).is_some(),
            "git applies the finding: {patch:?}"
        );
        let copy = patch.contains("copy to ");
        assert_eq!(reads_as_git_does(root, patch.as_bytes()), copy, "{patch:?}");
        if copy {
            let files = parse_decoded(&decode(patch.as_bytes()), ParseOptions::default()).unwrap();
            assert_eq!(files[0].path, ".github/workflows/pwn.yml");
            copies += 1;
        }
    }
    assert_eq!(copies, 2);
    // Controls Git writes itself are still reviewed, copies and mode changes included, and so
    // is a new file whose `diff --git` halves differ: Git has no `diff --git` name to hold the
    // `+++` to, and creates `y`.
    for patch in [
        "diff --git a/x b/y\nsimilarity index 100%\ncopy from x\ncopy to y\n",
        "diff --git a/x b/x\nold mode 100644\nnew mode 100755\n",
        "diff --git a/x b/x\nindex 1111111..2222222 100755\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n",
        "diff --git a/x b/y\nnew file mode 100644\n--- /dev/null\n+++ b/y\n@@ -0,0 +1 @@\n+a\n",
    ] {
        assert!(reads_as_git_does(root, patch.as_bytes()), "{patch:?}");
    }
    // Inconsistent headers Git refuses, each on its own: a rename (named by `rename to` alone)
    // that is also a copy, a new file's `+++` and a deleted file's `---` other than the
    // `diff --git` name.
    for patch in [
        "diff --git a/x b/z\nrename to z\ncopy from x\ncopy to z\n--- a/x\n+++ b/z\n@@ -1 +1 @@\n-a\n+b\n",
        "diff --git a/x b/x\nnew file mode 100644\n--- /dev/null\n+++ b/y\n@@ -0,0 +1 @@\n+a\n",
        "diff --git a/x b/x\ndeleted file mode 100644\n--- a/y\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\n",
    ] {
        assert_eq!(git_numstat(root, patch.as_bytes()), None, "git: {patch:?}");
        assert!(!reads_as_git_does(root, patch.as_bytes()), "ddf: {patch:?}");
    }
    let sections = GIT_SECTIONS
        .iter()
        .copied()
        .chain(["diff --git a/x b/y\nsimilarity index 100%\ncopy from x\ncopy to y\n"]);
    let (mut accepted, mut refused, mut cases) = (0, 0, 0);
    for section in sections {
        let lines: Vec<&str> = section.lines().collect();
        for at in 1..=lines.len() {
            for extra in EXTRA_HEADERS {
                let mut mutant: Vec<&str> = lines[..at].to_vec();
                mutant.push(extra);
                mutant.extend(&lines[at..]);
                let patch = mutant.join("\n") + "\n";
                cases += 1;
                if reads_as_git_does(root, patch.as_bytes()) {
                    accepted += 1;
                } else {
                    refused += 1;
                }
            }
        }
    }
    // Not vacuous: hundreds of cases, with Git-identical acceptances and refusals both seen.
    assert!(cases > 250, "{cases}");
    assert!(
        accepted > 10 && refused > 100,
        "accepted {accepted}, refused {refused}"
    );
}

/// Independent rescore of `af5c3d1` (F1, F2): Git ends a section's extended header at a
/// `Binary files … differ` or `GIT binary patch` line, and reads an unquoted move name only up
/// to a carriage return (or other whitespace that is not a space or tab). DDF read headers after
/// a binary line as the section's own, so a `copy to` or `rename to` there showed a harmless file
/// while Git rewrote the `diff --git` file (the workflow) from its `index` blob; and it showed
/// `build.rs\rdocs/notes.md` where Git wrote `build.rs`. Git applies every finding below; DDF
/// must refuse each one.
#[test]
fn no_header_after_a_binary_line_or_control_in_a_move_name_is_read() {
    let scratch = Scratch::new("binary-header");
    let root = scratch.0.path();
    let workflow = "diff --git a/.github/workflows/ci.yml b/.github/workflows/ci.yml\nindex 1111111..2222222 100644\nBinary files a/.github/workflows/ci.yml and b/.github/workflows/ci.yml differ\n";
    let findings = [
        // F1: Git rewrites the workflow; DDF showed `docs/logo-copy.png` or `docs/logo-v2.png`.
        format!("{workflow}copy from docs/logo.png\ncopy to docs/logo-copy.png\n"),
        format!("{workflow}rename from docs/logo.png\nrename to docs/logo-v2.png\n"),
        // F1, mode: DDF showed a mode change Git never makes.
        format!("{workflow}old mode 100644\nnew mode 100755\n"),
        // F2: Git writes `build.rs`, with the hunk's content.
        "diff --git a/docs/a.md b/docs/a.md\nsimilarity index 50%\nrename from docs/a.md\nrename to build.rs\rdocs/notes.md\n@@ -1 +1 @@\n-hello\n+fn main() {}\n".to_owned(),
        "diff --git a/docs/a.md b/docs/b.md\nsimilarity index 100%\ncopy from docs/a.md\ncopy to build.rs\rdocs/b.md\n".to_owned(),
    ];
    for patch in &findings {
        assert!(
            git_numstat(root, patch.as_bytes()).is_some(),
            "git applies the finding: {patch:?}"
        );
        assert!(!reads_as_git_does(root, patch.as_bytes()), "ddf: {patch:?}");
    }
    // Lines Git would not take for a binary marker are refused too, not read as one.
    for patch in [
        "diff --git a/x b/x\nGIT binary patchX\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n",
        "diff --git a/x b/x\nindex 1111111..2222222 100644\nBinary files a/x and b/x\n",
    ] {
        assert!(!reads_as_git_does(root, patch.as_bytes()), "ddf: {patch:?}");
    }
    // Controls: the binary sections Git writes, and a quoted move name, are still reviewed.
    for patch in [
        workflow.to_owned(),
        "diff --git a/x b/y\nsimilarity index 100%\nrename from x\nrename to y\n".to_owned(),
        "diff --git a/x b/y\nsimilarity index 100%\nrename from x\nrename to \"y z\"\n".to_owned(),
    ] {
        assert!(reads_as_git_does(root, patch.as_bytes()), "{patch:?}");
    }
}

/// Found while the 0.8.3 fix was reviewed. Git reads a mode with no file-type bits as a
/// submodule (`canon_mode`), and for a submodule it makes a directory and writes nothing from
/// the section: not a hunk's lines, not a binary body. DDF reviewed each as a new file or a
/// binary change with that content. Git's own reading is asserted for each finding.
#[test]
fn no_content_is_reviewed_under_a_mode_git_reads_as_a_submodule() {
    let scratch = Scratch::new("submodule-mode");
    let root = scratch.0.path();
    let new_file = "--- /dev/null\n+++ b/docs/notes.txt\n@@ -0,0 +1 @@\n+a\n";
    // Each of these was reviewed as a new file holding `a`; Git says `create mode 160000`.
    for mode in ["644", "755", "0", "000644", "140000", "160000"] {
        let patch = format!(
            "diff --git a/docs/notes.txt b/docs/notes.txt\nnew file mode {mode}\n{new_file}"
        );
        assert_eq!(
            git_summary(root, patch.as_bytes()),
            Some(vec![Summary::Create {
                mode: 0o160_000,
                path: "docs/notes.txt".to_owned()
            }]),
            "git: {patch:?}"
        );
        assert!(!reads_as_git_does(root, patch.as_bytes()), "ddf: {patch:?}");
    }
    // A binary body under a submodule mode: numstat and the summary agree with a review that
    // says "binary change", yet Git makes the directory and writes none of the bytes. Only a
    // refusal is right, so it is asserted here; the projection cannot show it.
    let binary_submodule = "diff --git a/x b/x\nnew file mode 160000\nindex 0000000000000000000000000000000000000000..f00a2af6cdfe5a97ff6e2afb34c1dcb18b084c10\nGIT binary patch\nliteral 4\nLcmd1E<YE8-0`mae\n\nliteral 0\nHcmV?d00001\n\n";
    assert_eq!(
        git_summary(root, binary_submodule.as_bytes()),
        Some(vec![Summary::Create {
            mode: 0o160_000,
            path: "x".to_owned()
        }]),
    );
    assert!(!reads_as_git_does(root, binary_submodule.as_bytes()));
    // An `index` mode Git canonicalises to a submodule's.
    for mode in ["644", "140000", "160000"] {
        let patch = format!(
            "diff --git a/x b/x\nindex 1111111..2222222 {mode}\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n"
        );
        assert!(!reads_as_git_does(root, patch.as_bytes()), "ddf: {patch:?}");
    }
    // A submodule as Git writes it is still reviewed, and so are the modes Git canonicalises.
    let id = "1".repeat(40);
    for patch in [
        format!(
            "diff --git a/sub b/sub\nnew file mode 160000\nindex 0000000..1111111\n--- /dev/null\n+++ b/sub\n@@ -0,0 +1 @@\n+Subproject commit {id}\n"
        ),
        format!(
            "diff --git a/sub b/sub\ndeleted file mode 160000\nindex 1111111..0000000\n--- a/sub\n+++ /dev/null\n@@ -1 +0,0 @@\n-Subproject commit {id}\n"
        ),
        format!(
            "diff --git a/sub b/sub\nindex 1111111..2222222 160000\n--- a/sub\n+++ b/sub\n@@ -1 +1 @@\n-Subproject commit {id}\n+Subproject commit {id}-dirty\n"
        ),
        format!("diff --git a/docs/notes.txt b/docs/notes.txt\nnew file mode 100664\n{new_file}"),
        format!("diff --git a/docs/notes.txt b/docs/notes.txt\nnew file mode 100700\n{new_file}"),
    ] {
        assert!(reads_as_git_does(root, patch.as_bytes()), "{patch:?}");
    }
}

/// Independent rescore of v0.8.2 (F3 and F2). F3: a `diff --git` line with no extended header
/// is no header to Git, so a binary line after it is no patch, but the line's name stays for the
/// next section that brings no names of its own: DDF reviewed a text change to `second` where
/// Git changed `first`. F2: `new file mode` and `new mode` set the same mode in Git and the
/// later wins, so a "new file" was created as a symlink or an executable. Git applies every
/// finding below; DDF must refuse each one.
#[test]
fn no_name_carries_over_from_a_bare_header_and_no_mode_overrides_a_new_file_mode() {
    let scratch = Scratch::new("bare-header");
    let root = scratch.0.path();
    let second =
        "diff --git a/second b/second\nindex 1111111..2222222 100644\n@@ -1 +1 @@\n-a\n+b\n";
    let new_file = "--- /dev/null\n+++ b/docs/notes.txt\n@@ -0,0 +1 @@\n+a\n";
    let findings = [
        // F3: Git applies the hunk to `first`.
        format!("diff --git a/first b/first\nBinary files a/first and b/first differ\n{second}"),
        format!("diff --git a/first b/first\nGIT binary patch\nliteral 0\nHcmV?d00001\n\n{second}"),
        // F2: Git creates an executable, then a symlink.
        format!(
            "diff --git a/docs/notes.txt b/docs/notes.txt\nnew file mode 100644\nnew mode 100755\n{new_file}"
        ),
        format!(
            "diff --git a/docs/notes.txt b/docs/notes.txt\nnew file mode 100644\nnew mode 120000\n{new_file}"
        ),
        format!(
            "diff --git a/docs/notes.txt b/docs/notes.txt\nnew mode 120000\nnew file mode 100644\n{new_file}"
        ),
    ];
    for patch in &findings {
        assert!(
            git_numstat(root, patch.as_bytes()).is_some(),
            "git applies the finding: {patch:?}"
        );
        assert!(!reads_as_git_does(root, patch.as_bytes()), "ddf: {patch:?}");
    }
    // The F3 finding names `first`, not `second`, in Git's own reading.
    assert_eq!(
        git_numstat(root, findings[0].as_bytes()),
        Some(vec![(Some((1, 1)), b"first".to_vec())]),
    );
    // Refused too, whatever Git makes of them: the bare header with a binary line and nothing
    // after it, and a whole-file mode beside `old mode`.
    for patch in [
        "diff --git a/first b/first\nBinary files a/first and b/first differ\n".to_owned(),
        format!("diff --git a/docs/notes.txt b/docs/notes.txt\nnew file mode 100644\nold mode 100755\n{new_file}"),
        "diff --git a/x b/x\ndeleted file mode 100644\nold mode 100755\n--- a/x\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\n".to_owned(),
    ] {
        assert!(!reads_as_git_does(root, patch.as_bytes()), "ddf: {patch:?}");
    }
    // Generated: a bare `diff --git a/first b/first`, one line of any kind, then each section,
    // and a section with no `---`/`+++` of its own. Whatever the line is, the review must be
    // refused or name the files Git names. The single-section mutants above never put one
    // section's leftovers in front of another, which is how F3 went unseen.
    let (mut accepted, mut refused, mut cases) = (0, 0, 0);
    for section in GIT_SECTIONS.iter().copied().chain([second]) {
        for between in EXTRA_HEADERS
            .iter()
            .copied()
            .chain(BREAKS)
            .chain(["literal 0", "HcmV?d00001"])
            // Headers that make the first section a patch of its own, so that some cases are
            // accepted: an `index` or similarity line alone no longer does (rescore of v0.8.3,
            // G1).
            .chain(["old mode 100644\nnew mode 100755", "new file mode 100644"])
        {
            let patch = format!("diff --git a/first b/first\n{between}\n{section}");
            cases += 1;
            if reads_as_git_does(root, patch.as_bytes()) {
                accepted += 1;
            } else {
                refused += 1;
            }
        }
    }
    assert!(
        cases >= 100 && accepted >= 5 && refused >= 50,
        "{cases} cases: {accepted} accepted, {refused} refused"
    );
    println!("bare-header cases: {cases}, accepted {accepted}, refused {refused}");
    // Controls: the forms Git writes are still reviewed as Git reads them.
    for patch in [
        format!(
            "diff --git a/first b/first\nindex 1111111..2222222 100644\nBinary files a/first and b/first differ\n{second}"
        ),
        format!("diff --git a/docs/notes.txt b/docs/notes.txt\nnew file mode 100755\n{new_file}"),
        format!("diff --git a/docs/notes.txt b/docs/notes.txt\nnew file mode 120000\n{new_file}"),
        "diff --git a/x b/x\nold mode 100644\nnew mode 100755\n".to_owned(),
    ] {
        assert!(reads_as_git_does(root, patch.as_bytes()), "{patch:?}");
    }
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

/// A section from which Git reads no patch is refused. That includes a Git section whose only
/// header is an `index` line: `git apply --numstat` lists it, but a real apply refuses it
/// ("patch with only garbage", see `real_apply`), and it was reviewed as `modified` with nothing
/// in it (rescore of v0.8.3, G1).
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
    assert!(matches!(
        parse_decoded(
            &decode(b"diff --git a/x b/x\nindex 1..2 100644\n"),
            ParseOptions::default()
        ),
        Err(PatchParseError::SectionWithoutPatch { line_number: 1 })
    ));
    // The header-only section Git does apply, an empty file's creation, is still reviewed.
    assert!(reads_as_git_does(
        scratch.0.path(),
        b"diff --git a/e b/e\nnew file mode 100644\nindex 0000000..e69de29\n"
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

/// Rescore of v0.8.3 (G1, G2, L1). `git apply --numstat` and `--summary` only read a patch; they
/// do not say what an apply writes. Two things a real apply does went unseen: Git takes a binary
/// section's content from its object store, and it applies a patch as a set (every removal, then
/// every creation). The tests here apply each patch for real, in a scratch repository whose
/// object store holds the blobs the patch names, and compare the tree Git leaves with the tree
/// the review's sections give when read in order.
#[cfg(unix)]
mod real_apply {
    use super::Scratch;
    use deep_diff_forge_core::{FileStatus, PatchLineKind, ReviewFile};
    use deep_diff_forge_patch::{ParseOptions, PatchParseError, decode, old_path, parse_decoded};
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;
    use std::{fs, path::Path, process::Command};

    /// One file of a tree. `content` is `None` where the review shows a binary change: it says
    /// the file is rewritten and not with what, so any content agrees.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Entry {
        executable: bool,
        symlink: bool,
        content: Option<Vec<u8>>,
    }

    type Tree = BTreeMap<String, Entry>;

    fn regular(content: &str) -> Entry {
        Entry {
            executable: false,
            symlink: false,
            content: Some(content.as_bytes().to_vec()),
        }
    }

    fn tree(files: &[(&str, &str)]) -> Tree {
        files
            .iter()
            .map(|(path, content)| ((*path).to_owned(), regular(content)))
            .collect()
    }

    /// Git with no user or system configuration (`apply.whitespace`, `core.autocrlf`).
    fn git(root: &Path, args: &[&str]) -> std::process::Output {
        Command::new("git")
            .current_dir(root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(args)
            .output()
            .unwrap()
    }

    /// Stores `content` as a blob in the repository's object store and returns its id.
    fn store(root: &Path, content: &str) -> String {
        fs::write(root.join(".git/blob.tmp"), content).unwrap();
        let out = git(root, &["hash-object", "-w", ".git/blob.tmp"]);
        assert!(out.status.success(), "git hash-object failed");
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    /// The work tree under `dir`, without `.git`.
    fn read_tree(root: &Path, dir: &Path, into: &mut Tree) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            if name == ".git" {
                continue;
            }
            if meta.is_dir() {
                read_tree(root, &path, into);
            } else if meta.file_type().is_symlink() {
                let target = fs::read_link(&path).unwrap();
                into.insert(
                    name,
                    Entry {
                        executable: false,
                        symlink: true,
                        content: Some(target.to_str().unwrap().as_bytes().to_vec()),
                    },
                );
            } else {
                into.insert(
                    name,
                    Entry {
                        executable: meta.permissions().mode() & 0o100 != 0,
                        symlink: false,
                        content: Some(fs::read(&path).unwrap()),
                    },
                );
            }
        }
    }

    /// A scratch repository holding `before`, with each of `blobs` in its object store. `@OLD@`
    /// in `patch` becomes the id of the blob of `before`'s first file, `@NEW@` the id of the
    /// first of `blobs`, and `@ZERO@` the null id.
    struct Case {
        scratch: Scratch,
        patch: String,
    }

    impl Case {
        fn new(tag: &str, before: &Tree, blobs: &[&str], patch: &str) -> Self {
            let scratch = Scratch::new(tag);
            let root = scratch.0.path();
            let mut ids = Vec::new();
            for (path, entry) in before {
                let content = entry.content.as_deref().unwrap();
                scratch.0.write(path, content);
                ids.push(store(root, std::str::from_utf8(content).unwrap()));
            }
            let stored: Vec<String> = blobs.iter().map(|blob| store(root, blob)).collect();
            let zero = "0".repeat(ids.first().or(stored.first()).map_or(40, String::len));
            let patch = patch
                .replace("@OLD@", ids.first().map_or("", String::as_str))
                .replace("@NEW@", stored.first().map_or("", String::as_str))
                .replace("@ZERO@", &zero);
            Self { scratch, patch }
        }

        /// The tree a real `git apply` leaves, or `None` when Git refuses the patch (it then
        /// writes nothing).
        fn git_apply(&self) -> Option<Tree> {
            let root = self.scratch.0.path();
            fs::write(root.join(".git/probe.patch"), &self.patch).unwrap();
            let applied = git(root, &["apply", ".git/probe.patch"]).status.success();
            let mut after = Tree::new();
            read_tree(root, root, &mut after);
            applied.then_some(after)
        }

        fn review(&self) -> Result<Vec<ReviewFile>, PatchParseError> {
            parse_decoded(&decode(self.patch.as_bytes()), ParseOptions::default())
        }
    }

    /// `old` with the file's hunks applied at the lines they name.
    fn patched(old: &[u8], file: &ReviewFile) -> Vec<u8> {
        let old: Vec<&[u8]> = old.split_inclusive(|&b| b == b'\n').collect();
        let (mut at, mut out) = (0, Vec::new());
        for hunk in &file.patch_twin.hunks {
            let start = (hunk.old_start.unwrap_or(0) as usize).saturating_sub(1);
            while at < start && at < old.len() {
                out.extend_from_slice(old[at]);
                at += 1;
            }
            for line in &hunk.lines {
                match line.kind {
                    PatchLineKind::Context => {
                        out.extend_from_slice(old.get(at).copied().unwrap_or_default());
                        at += 1;
                    }
                    PatchLineKind::Removed => at += 1,
                    PatchLineKind::Added => {
                        out.extend_from_slice(line.text.as_bytes());
                        if !line.no_newline {
                            out.push(b'\n');
                        }
                    }
                }
            }
        }
        for line in old.iter().skip(at) {
            out.extend_from_slice(line);
        }
        out
    }

    /// The tree the review describes: its sections carried out one after the other on `before`,
    /// each as its status, paths, mode headers and hunks say. A rename or copy takes its source
    /// from `before`, which is what its hunks are written against.
    fn reviewed_tree(before: &Tree, files: &[ReviewFile]) -> Tree {
        let mut after = before.clone();
        for file in files {
            let meta = &file.patch_twin.metadata;
            let says = |prefix: &str| meta.iter().any(|m| m.starts_with(prefix));
            let mode = ["new file mode ", "new mode "]
                .iter()
                .find_map(|p| meta.iter().find_map(|m| m.strip_prefix(p)));
            if file.status == FileStatus::Deleted || says("deleted file mode ") {
                after.remove(&file.path);
                continue;
            }
            let moved_from = old_path(file);
            let mut entry = match &moved_from {
                Some(from) => before.get(from).cloned(),
                None => after.get(&file.path).cloned(),
            }
            .unwrap_or_else(|| regular(""));
            if says("rename from ") {
                after.remove(moved_from.as_deref().unwrap());
            }
            if let Some(mode) = mode {
                entry.executable = mode == "100755";
                entry.symlink = mode == "120000";
            }
            entry.content = if file.status == FileStatus::BinaryChanged {
                None
            } else {
                Some(patched(entry.content.as_deref().unwrap_or_default(), file))
            };
            after.insert(file.path.clone(), entry);
        }
        after
    }

    /// Whether Git's tree is the reviewed one: the same paths with the same type and mode, and
    /// the same content wherever the review shows content.
    fn agree(git: &Tree, reviewed: &Tree) -> bool {
        git.len() == reviewed.len()
            && git.iter().zip(reviewed).all(|((gp, g), (rp, r))| {
                gp == rp
                    && g.executable == r.executable
                    && g.symlink == r.symlink
                    && (r.content.is_none() || g.content == r.content)
            })
    }

    /// The property: DDF refuses the patch, or Git refuses it (and writes nothing), or the tree
    /// Git writes is the tree the review describes. Returns Git's tree and DDF's verdict.
    fn applies_as_reviewed(
        tag: &str,
        before: &Tree,
        blobs: &[&str],
        patch: &str,
    ) -> (Option<Tree>, Result<Vec<ReviewFile>, PatchParseError>) {
        let case = Case::new(tag, before, blobs, patch);
        let review = case.review();
        let git = case.git_apply();
        if let (Some(git), Ok(files)) = (&git, &review) {
            let reviewed = reviewed_tree(before, files);
            assert!(
                agree(git, &reviewed),
                "DDF accepted a patch that git apply writes differently: {:?}\n git: {git:?}\n ddf: {reviewed:?}",
                case.patch
            );
        }
        (git, review)
    }

    const POLICY: &str = "allow admin\ndeny all\n";
    const STORED: &str = "allow all\n";

    /// G1: `Files … differ` after a full `index` line. Git writes the blob the line names from
    /// its object store, or empties the file for the null id. v0.8.3 showed `modified` (or
    /// `renamed`, `added`) with no hunk and no binary flag.
    #[test]
    fn a_files_differ_section_is_reviewed_as_the_binary_change_git_applies() {
        let before = tree(&[("x", POLICY)]);
        let stored = Some(STORED.as_bytes().to_vec());
        let findings = [
            // G1a: the content is replaced.
            (
                "diff --git a/x b/x\nindex @OLD@..@NEW@ 100644\nFiles a/x and b/x differ\n",
                "x",
                stored.clone(),
                false,
            ),
            (
                "diff --git a/x b/x\nindex @OLD@..@NEW@ 100644\nFiles differ\n",
                "x",
                stored.clone(),
                false,
            ),
            // G1b: the file is emptied; no stored blob is needed.
            (
                "diff --git a/x b/x\nindex @OLD@..@ZERO@ 100644\nFiles a/x and b/x differ\n",
                "x",
                Some(Vec::new()),
                false,
            ),
            // G1c: a rename whose new file gets the stored blob.
            (
                "diff --git a/x b/y\nsimilarity index 100%\nrename from x\nrename to y\nindex @OLD@..@NEW@ 100644\nFiles a/x and b/y differ\n",
                "y",
                stored.clone(),
                false,
            ),
            // G1d: a new executable holding the stored blob.
            (
                "diff --git a/n b/n\nnew file mode 100755\nindex @ZERO@..@NEW@\nFiles /dev/null and b/n differ\n",
                "n",
                stored.clone(),
                true,
            ),
        ];
        for (i, (patch, path, content, executable)) in findings.iter().enumerate() {
            let (git, review) = applies_as_reviewed(&format!("g1-{i}"), &before, &[STORED], patch);
            // Git's own result, measured: the path holds the stored blob (or nothing).
            let git = git.unwrap_or_else(|| panic!("git applies the finding: {patch:?}"));
            assert_eq!(git[*path].content, *content, "{patch:?}");
            assert_eq!(git[*path].executable, *executable, "{patch:?}");
            assert_eq!(git.contains_key("x"), *path != "y", "{patch:?}");
            // The review says what Git does: a binary change of that path.
            let files = review.unwrap_or_else(|e| panic!("{patch:?}: {e}"));
            assert_eq!(files.len(), 1);
            assert_eq!(
                (files[0].path.as_str(), files[0].status),
                (*path, FileStatus::BinaryChanged),
                "{patch:?}"
            );
        }
        // Git honours the line after the section's `---`/`+++` too; there DDF refuses it, as it
        // refuses `Binary files … differ` there.
        for marker in [
            "Files a/x and b/x differ",
            "Binary files a/x and b/x differ",
        ] {
            let patch = format!(
                "diff --git a/x b/x\nindex @OLD@..@NEW@ 100644\n--- a/x\n+++ b/x\n{marker}\n"
            );
            let (git, review) = applies_as_reviewed("g1-pair", &before, &[STORED], &patch);
            assert_eq!(git.unwrap()["x"].content, stored, "{marker}");
            assert!(
                matches!(
                    review,
                    Err(PatchParseError::MisplacedHeader { line_number: 5 })
                ),
                "{marker}"
            );
        }
    }

    /// Where Git does not read `Files … differ` as the marker, DDF does not either: after a
    /// hunk the hunk is applied, and after a line that ended the header, or with a CR before the
    /// line feed, the section is judged by its other headers. Under a submodule mode, and
    /// straight after `diff --git`, the marker is refused as `Binary files … differ` is.
    #[test]
    fn a_files_differ_line_is_a_marker_only_where_git_reads_one() {
        let before = tree(&[("x", POLICY)]);
        let rename = "diff --git a/x b/y\nsimilarity index 100%\nrename from x\nrename to y\n";
        let agreed = [
            // After a hunk: Git applies the hunk and ignores the line.
            "diff --git a/x b/x\nindex @OLD@..@NEW@ 100644\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n allow admin\n-deny all\n+deny most\nFiles a/x and b/x differ\n".to_owned(),
            // After a blank line, a note, or with a CR: Git renames and writes no blob.
            format!("{rename}\nFiles a/x and b/y differ\n"),
            format!("{rename}See notes\nFiles a/x and b/y differ\n"),
            format!("{rename}Files a/x and b/y differ\r\n"),
            // After `Binary files … differ` it is ignored text; the section is binary already.
            "diff --git a/x b/x\nindex @OLD@..@NEW@ 100644\nBinary files a/x and b/x differ\nFiles a/x and b/x differ\n".to_owned(),
        ];
        for (i, patch) in agreed.iter().enumerate() {
            let (git, review) =
                applies_as_reviewed(&format!("g1-pos-{i}"), &before, &[STORED], patch);
            assert!(git.is_some(), "git applies {patch:?}");
            assert!(review.is_ok(), "ddf reads {patch:?}: {:?}", review.err());
        }
        // With the `index` line as the section's only header, the ignored line leaves a section
        // Git refuses ("patch with only garbage"), and so does DDF.
        for tail in [
            "\nFiles a/x and b/x differ\n",
            "note\nFiles a/x and b/x differ\n",
            "Files a/x and b/x differ\r\n",
            "Files a/x and b/x\n",
            "",
        ] {
            let patch = format!("diff --git a/x b/x\nindex @OLD@..@NEW@ 100644\n{tail}");
            let (git, review) = applies_as_reviewed("g1-garbage", &before, &[STORED], &patch);
            assert_eq!(git, None, "git: {patch:?}");
            assert!(
                matches!(
                    review,
                    Err(PatchParseError::SectionWithoutPatch { line_number: 1 })
                ),
                "ddf: {patch:?}"
            );
        }
        // Git makes an empty directory `s` and writes none of the blob: refused.
        let submodule = "diff --git a/s b/s\nnew file mode 160000\nindex @ZERO@..@NEW@\nFiles /dev/null and b/s differ\n";
        let (git, review) = applies_as_reviewed("g1-submodule", &before, &[STORED], submodule);
        assert_eq!(git, Some(before.clone()), "git writes no file");
        assert!(matches!(review, Err(PatchParseError::InvalidMode { .. })));
        // No header to Git; refused so that no name carries over (F3).
        let bare = "diff --git a/x b/x\nFiles a/x and b/x differ\n";
        let (git, review) = applies_as_reviewed("g1-bare", &before, &[STORED], bare);
        assert_eq!(git, None);
        assert!(matches!(
            review,
            Err(PatchParseError::SectionWithoutPatch { .. })
        ));
    }

    /// The sections with no hunk that Git writes itself are applied by Git and still reviewed;
    /// the ones with no change in their headers are refused by both.
    #[test]
    fn a_section_with_no_hunk_is_reviewed_only_where_git_applies_it() {
        let before = tree(&[("x", POLICY), ("z", "")]);
        for (i, patch) in [
            "diff --git a/x b/x\nold mode 100644\nnew mode 100755\n",
            "diff --git a/x b/y\nsimilarity index 100%\nrename from x\nrename to y\n",
            "diff --git a/x b/y\nsimilarity index 100%\ncopy from x\ncopy to y\n",
            "diff --git a/e b/e\nnew file mode 100644\nindex 0000000..e69de29\n",
            "diff --git a/e b/e\nnew file mode 100755\n",
            "diff --git a/z b/z\ndeleted file mode 100644\nindex e69de29..0000000\n",
            "diff --git a/z b/z\ndeleted file mode 100644\n",
        ]
        .iter()
        .enumerate()
        {
            let (git, review) = applies_as_reviewed(&format!("nohunk-{i}"), &before, &[], patch);
            assert!(
                git.is_some_and(|after| after != before),
                "git applies {patch:?}"
            );
            assert!(review.is_ok(), "ddf reads {patch:?}: {:?}", review.err());
        }
        for patch in [
            "diff --git a/x b/x\nindex 1111111..2222222 100644\n",
            "diff --git a/x b/x\nindex @OLD@..@OLD@ 100644\n",
            "diff --git a/x b/x\nindex 1111111..2222222 100644\n--- a/x\n+++ b/x\n",
            "diff --git a/x b/x\nsimilarity index 100%\n",
            "diff --git a/x b/y\n--- a/x\n+++ b/y\n",
        ] {
            let (git, review) = applies_as_reviewed("nohunk-garbage", &before, &[], patch);
            assert_eq!(git, None, "git: {patch:?}");
            assert!(
                matches!(
                    review,
                    Err(PatchParseError::SectionWithoutPatch { line_number: 1 })
                ),
                "ddf: {patch:?}"
            );
        }
    }

    const MODIFY_X: &str = "diff --git a/x b/x\nindex 1111111..2222222 100644\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";
    const MODIFY_X_AGAIN: &str = "diff --git a/x b/x\nindex 2222222..3333333 100644\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-b\n+c\n";
    const DELETE_X: &str = "diff --git a/x b/x\ndeleted file mode 100644\nindex 1111111..0000000\n--- a/x\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\n";
    const DELETE_X_B: &str = "diff --git a/x b/x\ndeleted file mode 100644\nindex 2222222..0000000\n--- a/x\n+++ /dev/null\n@@ -1 +0,0 @@\n-b\n";
    const NEW_X: &str = "diff --git a/x b/x\nnew file mode 100755\nindex 0000000..1111111\n--- /dev/null\n+++ b/x\n@@ -0,0 +1 @@\n+payload\n";
    const DELETE_X_PAYLOAD: &str = "diff --git a/x b/x\ndeleted file mode 100755\nindex 1111111..0000000\n--- a/x\n+++ /dev/null\n@@ -1 +0,0 @@\n-payload\n";
    const MODIFY_X_PAYLOAD: &str = "diff --git a/x b/x\nindex 1111111..2222222 100755\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-payload\n+second\n";
    const RENAME_X_Y: &str =
        "diff --git a/x b/y\nsimilarity index 100%\nrename from x\nrename to y\n";
    const RENAME_X_Z: &str =
        "diff --git a/x b/z\nsimilarity index 100%\nrename from x\nrename to z\n";
    const RENAME_Q_Z: &str =
        "diff --git a/q b/z\nsimilarity index 100%\nrename from q\nrename to z\n";
    const RENAME_Q_X: &str =
        "diff --git a/q b/x\nsimilarity index 100%\nrename from q\nrename to x\n";
    const COPY_X_Y: &str = "diff --git a/x b/y\nsimilarity index 100%\ncopy from x\ncopy to y\n";
    const MODIFY_Y: &str = "diff --git a/y b/y\nindex 1111111..2222222 100644\n--- a/y\n+++ b/y\n@@ -1 +1 @@\n-a\n+b\n";

    /// Asserts DDF refuses `patch` for `path` in two sections, and returns Git's tree.
    fn refused_for_path(tag: &str, before: &Tree, patch: &str, named: &str) -> Option<Tree> {
        let (git, review) = applies_as_reviewed(tag, before, &[], patch);
        assert!(
            matches!(&review, Err(PatchParseError::PathInSeveralSections { path, .. }) if path == named),
            "ddf: {patch:?}: {:?}",
            review.map(|files| files.len())
        );
        git
    }

    /// G2: Git applies a patch as a set, removing first and creating after, and a rename reads
    /// its source as it was before the patch. Each finding is applied by Git and leaves the tree
    /// asserted here, which is not the tree its sections describe in order. v0.8.3 showed the
    /// sequence.
    #[test]
    fn a_path_one_section_writes_and_another_removes_is_refused() {
        let x = tree(&[("x", "a\n")]);
        let exec = |content: &str| Entry {
            executable: true,
            ..regular(content)
        };
        let mut payload = Tree::new();
        payload.insert("x".to_owned(), exec("payload\n"));
        let cases = [
            // G2e: "modified x", "deleted x"; `x` stays, holding `b`.
            (
                "g2e",
                x.clone(),
                format!("{MODIFY_X}{DELETE_X_B}"),
                "x",
                tree(&[("x", "b\n")]),
            ),
            // G2c: "added x", "deleted x"; the new executable stays.
            (
                "g2c",
                Tree::new(),
                format!("{NEW_X}{DELETE_X_PAYLOAD}"),
                "x",
                payload,
            ),
            // G2a: "modified x", "renamed x to y"; `x` stays modified, `y` gets the old content.
            (
                "g2a",
                x.clone(),
                format!("{MODIFY_X}{RENAME_X_Y}"),
                "x",
                tree(&[("x", "b\n"), ("y", "a\n")]),
            ),
            // G2d: "renamed q to x", "renamed x to z"; `x` gets the old `q`, `z` the old `x`.
            (
                "g2d",
                tree(&[("q", "Q\n"), ("x", "a\n")]),
                format!("{RENAME_Q_X}{RENAME_X_Z}"),
                "x",
                tree(&[("x", "Q\n"), ("z", "a\n")]),
            ),
        ];
        for (tag, before, patch, path, written) in cases {
            let git = refused_for_path(tag, &before, &patch, path);
            assert_eq!(git, Some(written), "git: {patch:?}");
        }
        // The swap, which `git diff -B -M` can write, is two such renames and is refused too.
        let swap = "diff --git a/y b/x\nsimilarity index 100%\nrename from y\nrename to x\n";
        let both = tree(&[("x", "a\n"), ("y", "Y\n")]);
        let git = refused_for_path("g2-swap", &both, &format!("{RENAME_X_Y}{swap}"), "y");
        assert_eq!(git, Some(tree(&[("x", "Y\n"), ("y", "a\n")])));
        // Refused whether Git applies them or not: a path removed and then written in place.
        for (i, (patch, path)) in [
            (format!("{DELETE_X}{MODIFY_X}"), "x"),
            (format!("{RENAME_X_Y}{MODIFY_X}"), "x"),
            (format!("{RENAME_X_Y}{NEW_X}"), "x"),
            (
                format!("{MODIFY_X}--- a/x\n+++ /dev/null\n@@ -1 +0,0 @@\n-b\n"),
                "x",
            ),
        ]
        .iter()
        .enumerate()
        {
            refused_for_path(&format!("g2-rw-{i}"), &x, patch, path);
        }
    }

    /// G2, the same root: a path two sections write other than in place, or two remove. Git
    /// keeps the second of two creations, and gives both new names of two renames the old file.
    #[test]
    fn a_path_two_sections_create_or_remove_is_refused() {
        let x = tree(&[("x", "a\n")]);
        let new_plain = "diff --git a/x b/x\nnew file mode 100644\nindex 0000000..1111111\n--- /dev/null\n+++ b/x\n@@ -0,0 +1 @@\n+other\n";
        let git = refused_for_path(
            "g2-new-new",
            &Tree::new(),
            &format!("{NEW_X}{new_plain}"),
            "x",
        );
        assert_eq!(git, Some(tree(&[("x", "other\n")])), "git keeps the second");
        let git = refused_for_path("g2-ren-ren", &x, &format!("{RENAME_X_Y}{RENAME_X_Z}"), "x");
        assert_eq!(
            git,
            Some(tree(&[("y", "a\n"), ("z", "a\n")])),
            "git writes both"
        );
        let xy = tree(&[("x", "a\n"), ("y", "a\n")]);
        for (i, (before, patch, path)) in [
            (&x, format!("{DELETE_X}{DELETE_X}"), "x"),
            (&x, format!("{DELETE_X}{RENAME_X_Y}"), "x"),
            (&x, format!("{MODIFY_X}{NEW_X}"), "x"),
            (&x, format!("{COPY_X_Y}{COPY_X_Y}"), "y"),
            (&xy, format!("{MODIFY_Y}{RENAME_X_Y}"), "y"),
            (&xy, format!("{MODIFY_Y}{COPY_X_Y}"), "y"),
            (
                &tree(&[("q", "Q\n"), ("x", "a\n")]),
                format!("{RENAME_X_Z}{RENAME_Q_Z}"),
                "z",
            ),
        ]
        .iter()
        .enumerate()
        {
            refused_for_path(&format!("g2-ww-{i}"), before, patch, path);
        }
    }

    /// The shapes that stay reviewed, each applied for real: Git's tree is the tree the sections
    /// describe in order. A change in place follows any section that wrote the path (Git patches
    /// the earlier result), `new file` follows `deleted file` (a change of type), and a copy or
    /// rename only reads its source, as it was before the patch.
    #[test]
    fn sections_git_applies_in_order_to_one_path_agree_with_a_real_apply() {
        let x = tree(&[("x", "a\n")]);
        let symlink = "diff --git a/x b/x\nnew file mode 120000\nindex 0000000..1111111\n--- /dev/null\n+++ b/x\n@@ -0,0 +1 @@\n+target\n\\ No newline at end of file\n";
        let mode_only = "diff --git a/x b/x\nold mode 100644\nnew mode 100755\n";
        let copy_changed = "diff --git a/x b/y\nsimilarity index 50%\ncopy from x\ncopy to y\n--- a/x\n+++ b/y\n@@ -1 +1 @@\n-a\n+c\n";
        let traditional =
            "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-b\n+c\n";
        let cases = [
            (
                &x,
                format!("{MODIFY_X}{MODIFY_X_AGAIN}"),
                tree(&[("x", "c\n")]),
            ),
            (&x, traditional.to_owned(), tree(&[("x", "c\n")])),
            (&x, format!("{RENAME_X_Y}{MODIFY_Y}"), tree(&[("y", "b\n")])),
            (
                &x,
                format!("{COPY_X_Y}{MODIFY_Y}"),
                tree(&[("x", "a\n"), ("y", "b\n")]),
            ),
            // A copy reads the source as it was: G2b's shape, which `git diff -C` writes.
            (
                &x,
                format!("{MODIFY_X}{COPY_X_Y}"),
                tree(&[("x", "b\n"), ("y", "a\n")]),
            ),
            (
                &x,
                format!("{COPY_X_Y}{MODIFY_X}"),
                tree(&[("x", "b\n"), ("y", "a\n")]),
            ),
            (
                &x,
                format!("{MODIFY_X}{copy_changed}"),
                tree(&[("x", "b\n"), ("y", "c\n")]),
            ),
            (
                &x,
                format!("{COPY_X_Y}{RENAME_X_Z}"),
                tree(&[("y", "a\n"), ("z", "a\n")]),
            ),
        ];
        for (i, (before, patch, written)) in cases.iter().enumerate() {
            let (git, review) = applies_as_reviewed(&format!("g2-ok-{i}"), before, &[], patch);
            assert_eq!(git.as_ref(), Some(written), "git: {patch:?}");
            assert!(review.is_ok(), "ddf reads {patch:?}: {:?}", review.err());
        }
        // With a mode or a type in play, the tree is compared by `applies_as_reviewed` alone.
        for (i, (before, patch)) in [
            (&Tree::new(), format!("{NEW_X}{MODIFY_X_PAYLOAD}")),
            (&x, format!("{MODIFY_X}{mode_only}")),
            (&x, format!("{mode_only}{MODIFY_X}")),
            (&x, format!("{DELETE_X}{symlink}")),
            (&x, format!("{DELETE_X}{NEW_X}{MODIFY_X_PAYLOAD}")),
        ]
        .iter()
        .enumerate()
        {
            let (git, review) = applies_as_reviewed(&format!("g2-mode-{i}"), before, &[], patch);
            assert!(
                git.is_some_and(|after| after != **before),
                "git applies {patch:?}"
            );
            assert!(review.is_ok(), "ddf reads {patch:?}: {:?}", review.err());
        }
    }

    /// Runs Git in `root`, as a committer with a fixed identity, and returns its output.
    fn git_ok(root: &Path, args: &[&str]) -> Vec<u8> {
        let mut all = vec![
            "-c",
            "user.name=ddf",
            "-c",
            "user.email=ddf@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ];
        all.extend(args);
        let out = git(root, &all);
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    }

    /// What Git itself writes when a patch names a path in more than one section, generated by
    /// a real Git and applied by one: a change of type (`git diff`), a copy whose source also
    /// changes (`git diff -C`), and a series that touches one file in three commits
    /// (`git format-patch --stdout`). All stay reviewed, and agree with the apply.
    #[test]
    fn what_git_writes_for_one_path_in_several_sections_stays_reviewed() {
        let origin = Scratch::new("g2-written");
        let root = origin.0.path();
        let body = "one line of thirty\n".repeat(30);
        origin.0.write("kept", &body);
        origin.0.write("typed", "a file\n");
        git_ok(root, &["add", "-A"]);
        git_ok(root, &["commit", "-q", "-m", "base"]);
        let before = tree(&[("kept", body.as_str()), ("typed", "a file\n")]);
        // A type change and a copy of a file that also changes.
        fs::remove_file(root.join("typed")).unwrap();
        std::os::unix::fs::symlink("kept", root.join("typed")).unwrap();
        origin.0.write("copied", &body);
        origin.0.write("kept", format!("{body}one more\n"));
        git_ok(root, &["add", "-A"]);
        let diff = String::from_utf8(git_ok(root, &["diff", "--cached", "-C"])).unwrap();
        assert!(diff.contains("deleted file mode 100644") && diff.contains("new file mode 120000"));
        assert!(diff.contains("copy from kept"), "{diff}");
        git_ok(root, &["commit", "-q", "-m", "type and copy"]);
        // Two more commits to `kept`, for the series.
        origin.0.write("kept", format!("first\n{body}one more\n"));
        git_ok(root, &["commit", "-q", "-a", "-m", "second"]);
        origin
            .0
            .write("kept", format!("first\n{body}one more\nlast\n"));
        git_ok(root, &["commit", "-q", "-a", "-m", "third"]);
        let series =
            String::from_utf8(git_ok(root, &["format-patch", "--stdout", "-C", "HEAD~3"])).unwrap();
        assert_eq!(series.matches("diff --git a/kept b/kept\n").count(), 3);
        let mut written = Tree::new();
        read_tree(root, root, &mut written);
        for (tag, patch) in [("g2-written-diff", &diff), ("g2-written-series", &series)] {
            let (git, review) = applies_as_reviewed(tag, &before, &[], patch);
            assert!(git.is_some(), "git applies what it wrote: {patch}");
            assert!(review.is_ok(), "ddf reads {patch}: {:?}", review.err());
            if tag == "g2-written-series" {
                assert_eq!(
                    git,
                    Some(written.clone()),
                    "the series gives the final commit"
                );
            }
        }
    }

    /// L1: a traditional diff with `/dev/null` on both sides. Git creates a file named by the
    /// whole second line; v0.8.3 showed `dev/null`.
    #[test]
    fn a_traditional_diff_with_dev_null_on_both_sides_is_refused() {
        for (i, (second, name)) in [
            ("/dev/null run.sh", "dev/null run.sh"),
            ("/dev/null", "dev/null"),
        ]
        .iter()
        .enumerate()
        {
            let patch = format!("--- /dev/null\n+++ {second}\n@@ -0,0 +1 @@\n+payload\n");
            let (git, review) = applies_as_reviewed(&format!("l1-{i}"), &Tree::new(), &[], &patch);
            assert_eq!(git, Some(tree(&[(*name, "payload\n")])), "git: {patch:?}");
            assert!(
                matches!(
                    review,
                    Err(PatchParseError::InconsistentHeader { line_number: 2 })
                ),
                "ddf: {patch:?}"
            );
        }
        // Control: one `/dev/null` side is a creation, applied and reviewed alike.
        let new_file = "--- /dev/null\n+++ b/run.sh\n@@ -0,0 +1 @@\n+payload\n";
        let (git, review) = applies_as_reviewed("l1-control", &Tree::new(), &[], new_file);
        assert_eq!(git, Some(tree(&[("run.sh", "payload\n")])));
        assert!(review.is_ok());
    }
}
