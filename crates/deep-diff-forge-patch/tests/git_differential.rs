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
/// created or deleted file, or a mode change (`<move>` for a moved file's, which Git prints with
/// no name). Paths are as Git prints them with `core.quotepath=false`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Summary {
    Copy { from: String, to: String },
    Create(String),
    Delete(String),
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
            lines.push(Summary::Create(rest.split_once(' ')?.1.to_owned()));
        } else if let Some(rest) = line.strip_prefix("delete mode ") {
            lines.push(Summary::Delete(rest.split_once(' ')?.1.to_owned()));
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
        if file.status == FileStatus::Added || says("new file mode ") {
            lines.push(Summary::Create(file.path.clone()));
        }
        if file.status == FileStatus::Deleted || says("deleted file mode ") {
            lines.push(Summary::Delete(file.path.clone()));
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
