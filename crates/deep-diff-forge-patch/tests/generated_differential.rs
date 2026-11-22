// DDF integration: R03, R07, R46. The generated differential: seeded patches built from the sections Git writes and from near misses of them, each read by DDF and applied by real `git apply` over a seeded object store; a patch DDF accepts must be one Git reads and writes as the review describes.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module index: docs/ARCHITECTURE.md#crates-as-built-030
//! Patches are generated from a small grammar: one to three sections drawn from the shapes Git
//! writes (`CONFORMANCE.md`), then up to three near-miss moves, each a change of one kind to one
//! section (a line dropped, doubled or swapped, a space lost, a carriage return, a quoted or
//! changed name, a mode or `index` id that differs, a header of another row put in, a body that
//! does not apply or is binary, the names or the whole header taken away), or to two (a bare
//! header with a binary line before a section with no names). Each is read by DDF,
//! by `git apply --numstat --summary` and `--check`, and applied by a real `git apply` in a
//! repository whose object store holds every blob an `index` line names.
//!
//! The test fails when DDF reads a patch that Git refuses to read, or that Git applies to a tree
//! other than the one the review describes (or whose hunks the review cannot fit to the files it
//! names), and when DDF refuses a patch made of Git's own sections on distinct paths with no
//! move. It counts and prints the patches Git applies and DDF refuses.
//!
//! The seeds are fixed. `DDF_DIFFERENTIAL_SEEDS=N` runs seeds 1 to N instead (a campaign:
//! `DDF_DIFFERENTIAL_SEEDS=2000 cargo test -p deep-diff-forge-patch --test generated_differential
//! -- --nocapture`).
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real git processes, reads the environment and the clock"
)]

mod support;

use deep_diff_forge_patch::{ParseOptions, PatchParseError, decode, parse_decoded};
use std::collections::BTreeMap;
use support::{Repo, Tree, agree, regular, reviewed_tree};

/// The seeds the gate runs, and the patches made from each.
const SEEDS: u64 = 64;
const PER_SEED: usize = 375;
/// Threads, each with its own repository.
const THREADS: u64 = 8;

/// A small `xorshift64`.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap()
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// The tree every patch is applied to: `x` and `w` both hold `a` (so a hunk meant for one also
/// fits the other), `y` holds `y`, `e` is empty, `d/x` holds `dx` (for doubled slashes), `f` ends
/// with two of its earlier lines again (for hunks Git anchors at the end).
fn before() -> Tree {
    [
        ("x", &b"a\n"[..]),
        ("w", b"a\n"),
        ("y", b"y\n"),
        ("e", b""),
        ("d/x", b"dx\n"),
        ("m", b"[prod]\nallow = false\n[test]\nallow = false\n"),
        ("f", b"x\na\nb\nc\na\nb\n"),
    ]
    .into_iter()
    .map(|(path, content)| (path.to_owned(), regular(content)))
    .collect()
}

/// The blobs of the object store, named in the sections as `@OLD@` (`a`), `@NEW@` (`stored`),
/// `@Y@`, `@E@` (empty), `@T@` (`y` with no line feed, a symlink's target), `@DX@` and `@M@` (two
/// sections, for hunks with little context), `@F@`; `…7@` is
/// the seven-digit abbreviation, `@ZERO@` the null id and `@SUB@` a submodule's commit id, each
/// as long as the repository's ids (SHA-1 or SHA-256).
const BLOBS: [(&str, &[u8]); 8] = [
    ("OLD", b"a\n"),
    ("NEW", b"stored\n"),
    ("Y", b"y\n"),
    ("E", b""),
    ("T", b"y"),
    ("DX", b"dx\n"),
    ("M", b"[prod]\nallow = false\n[test]\nallow = false\n"),
    ("F", b"x\na\nb\nc\na\nb\n"),
];

/// Sections Git writes for changes to `before()`, as `git diff` writes them (a change of type is
/// Git's two sections).
const WRITTEN: &[&str] = &[
    "diff --git a/x b/x\nindex @OLD7@..@NEW7@ 100644\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+stored\n",
    "diff --git a/w b/w\nindex @OLD7@..@NEW7@ 100644\n--- a/w\n+++ b/w\n@@ -1 +1 @@\n-a\n+stored\n",
    "diff --git a/y b/y\nindex @Y7@..@NEW7@ 100644\n--- a/y\n+++ b/y\n@@ -1 +1 @@\n-y\n+stored\n",
    "diff --git a/x b/x\nold mode 100644\nnew mode 100755\n",
    "diff --git a/w b/w\nold mode 100644\nnew mode 100755\n",
    "diff --git a/x b/x\nold mode 100644\nnew mode 100755\nindex @OLD7@..@NEW7@\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+stored\n",
    "diff --git a/x b/x\ndeleted file mode 100644\nindex @OLD7@..@ZERO7@\n--- a/x\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\n",
    "diff --git a/w b/w\ndeleted file mode 100644\nindex @OLD7@..@ZERO7@\n--- a/w\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\n",
    "diff --git a/e b/e\ndeleted file mode 100644\nindex @E7@..@ZERO7@\n",
    "diff --git a/n b/n\nnew file mode 100644\nindex @ZERO7@..@NEW7@\n--- /dev/null\n+++ b/n\n@@ -0,0 +1 @@\n+stored\n",
    "diff --git a/n b/n\nnew file mode 100755\nindex @ZERO7@..@NEW7@\n--- /dev/null\n+++ b/n\n@@ -0,0 +1 @@\n+stored\n",
    "diff --git a/n b/n\nnew file mode 100644\nindex @ZERO7@..@E7@\n",
    "diff --git a/x b/r\nsimilarity index 100%\nrename from x\nrename to r\n",
    "diff --git a/x b/r\nsimilarity index 50%\nrename from x\nrename to r\nindex @OLD7@..@NEW7@ 100644\n--- a/x\n+++ b/r\n@@ -1 +1 @@\n-a\n+stored\n",
    "diff --git a/x b/r\nold mode 100644\nnew mode 100755\nsimilarity index 100%\nrename from x\nrename to r\n",
    "diff --git a/y b/c\nsimilarity index 100%\ncopy from y\ncopy to c\n",
    "diff --git a/y b/c\nsimilarity index 50%\ncopy from y\ncopy to c\nindex @Y7@..@NEW7@ 100644\n--- a/y\n+++ b/c\n@@ -1 +1 @@\n-y\n+stored\n",
    "diff --git a/x b/x\nindex @OLD@..@NEW@ 100644\nBinary files a/x and b/x differ\n",
    "diff --git a/x b/x\nold mode 100644\nnew mode 100755\nindex @OLD@..@NEW@\nBinary files a/x and b/x differ\n",
    "diff --git a/x b/x\nindex @OLD@..@E@ 100644\nGIT binary patch\nliteral 0\nHcmV?d00001\n\n",
    "diff --git a/n b/n\nnew file mode 100644\nindex @ZERO@..@NEW@\nBinary files /dev/null and b/n differ\n",
    "diff --git a/x b/x\ndeleted file mode 100644\nindex @OLD@..@ZERO@\nBinary files a/x and /dev/null differ\n",
    "diff --git a/x b/r\nsimilarity index 60%\nrename from x\nrename to r\nindex @OLD@..@NEW@ 100644\nBinary files a/x and b/r differ\n",
    "diff --git a/x b/x\ndeleted file mode 100644\nindex @OLD7@..@ZERO7@\n--- a/x\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\ndiff --git a/x b/x\nnew file mode 120000\nindex @ZERO7@..@T7@\n--- /dev/null\n+++ b/x\n@@ -0,0 +1 @@\n+y\n\\ No newline at end of file\n",
    "diff --git a/x b/x\ndissimilarity index 100%\nindex @OLD7@..@NEW7@ 100644\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+stored\n",
    "diff --git a/m b/m\nnew file mode 160000\nindex 0000000..1234567\n--- /dev/null\n+++ b/m\n@@ -0,0 +1 @@\n+Subproject commit @SUB@\n",
    // A plain `git diff` of a binary file: abbreviated ids, which `git apply` refuses whole.
    "diff --git a/x b/x\nindex @OLD7@..@NEW7@ 100644\nBinary files a/x and b/x differ\n",
    "diff --git a/n b/n\nnew file mode 100644\nindex @ZERO7@..@NEW7@\nBinary files /dev/null and b/n differ\n",
    // A file in a directory, for the doubled-slash move.
    "diff --git a/d/x b/d/x\nindex @DX7@..@NEW7@ 100644\n--- a/d/x\n+++ b/d/x\n@@ -1 +1 @@\n-dx\n+stored\n",
    "diff --git a/d/x b/r\nsimilarity index 100%\nrename from d/x\nrename to r\n",
    "diff --git a/d/x b/c\nsimilarity index 100%\ncopy from d/x\ncopy to c\n",
    // Hunks with little context, as `git diff -U0` and `-U1` write them, that Git places where
    // they say: the whole file, or context on the side Git does not anchor.
    "diff --git a/m b/m\nindex @M7@..@NEW7@ 100644\n--- a/m\n+++ b/m\n@@ -1,2 +1,2 @@\n-[prod]\n+[stage]\n allow = false\n",
    "diff --git a/m b/m\nindex @M7@..@NEW7@ 100644\n--- a/m\n+++ b/m\n@@ -1,3 +1,3 @@\n [prod]\n-allow = false\n+allow = true\n [test]\n",
    "diff --git a/m b/m\nindex @M7@..@NEW7@ 100644\n--- a/m\n+++ b/m\n@@ -3,2 +3,2 @@\n [test]\n-allow = false\n+allow = true\n",
    "diff --git a/m b/m\nindex @M7@..@NEW7@ 100644\n--- a/m\n+++ b/m\n@@ -1,4 +1,4 @@\n-[prod]\n+[stage]\n allow = false\n [test]\n-allow = false\n+allow = true\n",
];

/// Sections `git diff -U0` writes that plain `git apply` places at the end of the file, not
/// where their headers say: refused (`HunkPlacedElsewhere`), though Git writes them.
const WRITTEN_U0: &[&str] = &[
    "diff --git a/m b/m\nindex @M7@..@NEW7@ 100644\n--- a/m\n+++ b/m\n@@ -2 +2 @@\n-allow = false\n+allow = true\n",
    "diff --git a/m b/m\nindex @M7@..@NEW7@ 100644\n--- a/m\n+++ b/m\n@@ -2,0 +3 @@\n+x = 1\n",
];

/// Sections written by hand that Git anchors at the end of the file (no trailing context), the
/// first with a header that names the earlier copy of its lines: Git changes the last copy, and
/// the review must describe that tree (`CONFORMANCE.md`, row `hunk-anchor`).
const END_ANCHORED: &[&str] = &[
    "diff --git a/f b/f\nindex @F7@..@NEW7@ 100644\n--- a/f\n+++ b/f\n@@ -2,2 +2,2 @@\n a\n-b\n+B\n",
    "diff --git a/f b/f\nindex @F7@..@NEW7@ 100644\n--- a/f\n+++ b/f\n@@ -5,2 +5,2 @@\n a\n-b\n+B\n",
];

/// Header lines of other rows, put into a section by the `insert` move.
const FOREIGN: &[&str] = &[
    "similarity index 90%",
    "dissimilarity index 10%",
    "old mode 100644",
    "new mode 100755",
    "new mode 120000",
    "new file mode 100644",
    "deleted file mode 100644",
    "index @OLD@..@NEW@ 100644",
    "index @OLD@..@NEW@",
    "rename from x",
    "rename to s",
    "copy from y",
    "copy to k",
    "rename old x",
    "Files a/x and b/x differ",
    "Binary files a/x and b/x differ",
    "GIT binary patch",
    "",
    "note",
    "--- a/x",
    "+++ b/x",
    "--- /dev/null",
    "+++ /dev/null",
    "@@ -1 +1 @@",
];

/// Changes of a name on one line only.
const RENAMES: &[(&str, &str)] = &[
    (" b/x", " b/z"),
    ("+++ b/", "+++ b/z"),
    ("--- a/x", "--- a/w"),
    ("a/x b", "a/w b"),
    ("rename to r", "rename to s"),
    ("rename from x", "rename from w"),
    ("copy to c", "copy to k"),
    ("copy from y", "copy from x"),
];

/// Bodies a section may be given instead of its own.
const BODIES: &[&str] = &[
    "@@ -1 +1 @@\n-q\n+b\n",
    "@@ -1 +1 @@\n-a\n+b\n",
    "",
    "Binary files a/x and b/x differ\n",
    "GIT binary patch\nliteral 0\nHcmV?d00001\n\n",
];

/// One section: its header lines (`diff --git` to `+++` or the binary line) and its body.
#[derive(Clone)]
struct Section {
    header: Vec<String>,
    body: Vec<String>,
}

fn sections(text: &str) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for line in text.lines() {
        if line.starts_with("diff --git ") {
            out.push(Section {
                header: vec![line.to_owned()],
                body: Vec::new(),
            });
            continue;
        }
        let section = out.last_mut().unwrap();
        let in_header = section.body.is_empty()
            && !section.header.last().is_some_and(|l| {
                l.starts_with("+++ ") || l.starts_with("Binary files ") || l == "GIT binary patch"
            });
        if in_header && !line.starts_with("@@") {
            section.header.push(line.to_owned());
        } else {
            section.body.push(line.to_owned());
        }
    }
    out
}

/// The names a section's `diff --git` line gives.
fn names(section: &Section) -> Vec<String> {
    section.header[0]
        .trim_start_matches("diff --git ")
        .split(' ')
        .map(|half| {
            half.split_once('/')
                .map_or(half, |(_, name)| name)
                .to_owned()
        })
        .collect()
}

/// One section's header lost but for a binary line, then a section with no names of its own:
/// the shape in which a bare header's name carried over to the next section (F3).
fn carry(rng: &mut Rng, sections: &mut [Section]) -> &'static str {
    let at = rng.below(sections.len() - 1);
    let name = names(&sections[at]).pop().unwrap_or_default();
    sections[at].header.truncate(1);
    sections[at]
        .header
        .push(format!("Binary files a/{name} and b/{name} differ"));
    sections[at].body.clear();
    sections[at + 1]
        .header
        .retain(|l| !l.starts_with("--- ") && !l.starts_with("+++ "));
    "carry"
}

/// Applies one near-miss move to a random section; its name, for the report.
fn mutate(rng: &mut Rng, sections: &mut [Section]) -> &'static str {
    if sections.len() > 1 && rng.below(10) == 0 {
        return carry(rng, sections);
    }
    let at = rng.below(sections.len());
    let s = &mut sections[at];
    let line = 1 + rng.below(s.header.len().max(2) - 1);
    let line = line.min(s.header.len() - 1);
    match rng.below(14) {
        0 if s.header.len() > 1 => {
            s.header.remove(line);
            "drop"
        }
        1 => {
            let i = rng.below(s.header.len());
            let copy = s.header[i].clone();
            s.header.insert(i, copy);
            "double"
        }
        2 if s.header.len() > 2 && line + 1 < s.header.len() => {
            s.header.swap(line, line + 1);
            "swap"
        }
        3 => {
            let text = &mut s.header[line];
            let spaces: Vec<usize> = text.match_indices(' ').map(|(i, _)| i).collect();
            if spaces.is_empty() {
                return "space (none)";
            }
            let i = *rng.pick(&spaces);
            text.replace_range(i..=i, if rng.below(2) == 0 { "" } else { "  " });
            "space"
        }
        4 => {
            s.header[line].push('\r');
            "carriage return"
        }
        5 => {
            let text = &mut s.header[line];
            for prefix in [
                "--- ",
                "+++ ",
                "rename from ",
                "rename to ",
                "copy from ",
                "copy to ",
            ] {
                if let Some(name) = text.strip_prefix(prefix).filter(|n| *n != "/dev/null") {
                    *text = format!("{prefix}\"{name}\"");
                    return "quoted name";
                }
            }
            if let Some(rest) = text.strip_prefix("diff --git ") {
                let halves: Vec<String> = rest.split(' ').map(|h| format!("\"{h}\"")).collect();
                *text = format!("diff --git {}", halves.join(" "));
            }
            "quoted name"
        }
        6 => {
            let i = rng.below(s.header.len());
            let text = &mut s.header[i];
            if let Some((from, to)) = RENAMES.iter().find(|(from, _)| text.contains(from)) {
                *text = text.replacen(from, to, 1);
            }
            "other name"
        }
        7 => {
            let i = rng.below(s.header.len());
            let text = &mut s.header[i];
            let to = *rng.pick(&[
                "644", "160000", "100664", "100755", "100644", "120000", "040000",
            ]);
            for mode in ["100644", "100755", "120000", "160000"] {
                if text.ends_with(mode) {
                    *text = format!("{}{to}", &text[..text.len() - mode.len()]);
                    break;
                }
            }
            "mode"
        }
        8 => change_index_id(rng, s),
        _ => mutate_section(rng, s),
    }
}

/// Gives a section's `index` ids another form: abbreviated, null, a stored blob's, not hex.
fn change_index_id(rng: &mut Rng, s: &mut Section) -> &'static str {
    for text in s.header.iter_mut().filter(|l| l.starts_with("index ")) {
        let to = *rng.pick(&["@OLD7@", "@ZERO@", "@NEW@", "zzzzzzz", "@E@", "@OLD@"]);
        let rest = text["index ".len()..].to_owned();
        let (ids, mode) = rest
            .split_once(' ')
            .map_or((rest.as_str(), None), |(ids, mode)| (ids, Some(mode)));
        let (old, new) = ids.split_once("..").unwrap_or((ids, ""));
        let ids = if rng.below(2) == 0 {
            format!("{to}..{new}")
        } else {
            format!("{old}..{to}")
        };
        *text = match mode {
            Some(mode) => format!("index {ids} {mode}"),
            None => format!("index {ids}"),
        };
    }
    "index id"
}

/// The moves that change a section as a whole: a header of another row put in, another body,
/// the binary marker's word changed, the names or the whole header taken away.
fn mutate_section(rng: &mut Rng, s: &mut Section) -> &'static str {
    match rng.below(7) {
        // A path spelled with a doubled slash, which Git squashes (`d//x` is `d/x`).
        5 => {
            if let Some(text) = s.header.iter_mut().find(|l| l.contains("d/x")) {
                *text = text.replacen("d/x", "d//x", 1);
            }
            "double slash"
        }
        0 => {
            let i = 1 + rng.below(s.header.len());
            s.header
                .insert(i.min(s.header.len()), (*rng.pick(FOREIGN)).to_owned());
            "insert"
        }
        1 => {
            s.body = rng.pick(BODIES).lines().map(str::to_owned).collect();
            if rng.below(3) == 0 {
                s.header
                    .retain(|l| !l.starts_with("--- ") && !l.starts_with("+++ "));
            }
            "body"
        }
        2 => {
            for text in &mut s.header {
                if let Some(rest) = text.strip_prefix("Binary files ") {
                    *text = format!("Files {rest}");
                } else if let Some(rest) = text.strip_prefix("Files ") {
                    *text = format!("Binary files {rest}");
                }
            }
            "marker"
        }
        3 => {
            s.header
                .retain(|l| !l.starts_with("--- ") && !l.starts_with("+++ "));
            "no names"
        }
        4 => {
            let name = names(s).pop().unwrap_or_default();
            s.header.truncate(1);
            s.header
                .push(format!("Binary files a/{name} and b/{name} differ"));
            s.body.clear();
            "binary only"
        }
        _ => {
            s.header.push((*rng.pick(FOREIGN)).to_owned());
            "append"
        }
    }
}

/// A generated patch: its text, its moves, and whether it is Git's own (sections Git writes,
/// on distinct paths, with no move).
struct Generated {
    text: String,
    moves: Vec<&'static str>,
    written: bool,
    /// One of its sections is from [`WRITTEN_U0`].
    unanchored: bool,
    /// One of its sections is from [`END_ANCHORED`].
    end_anchored: bool,
}

fn generate(rng: &mut Rng) -> Generated {
    let pool: Vec<&str> = WRITTEN
        .iter()
        .chain(WRITTEN_U0)
        .chain(END_ANCHORED)
        .copied()
        .collect();
    let groups: Vec<&str> = (0..=rng.below(3)).map(|_| *rng.pick(&pool)).collect();
    let unanchored = groups.iter().any(|g| WRITTEN_U0.contains(g));
    let end_anchored = groups.iter().any(|g| END_ANCHORED.contains(g));
    let mut paths: Vec<String> = Vec::new();
    let mut distinct = true;
    for group in &groups {
        let mut mine: Vec<String> = sections(group).iter().flat_map(names).collect();
        mine.sort();
        mine.dedup();
        distinct &= !mine.iter().any(|p| paths.contains(p));
        paths.extend(mine);
    }
    let mut all: Vec<Section> = groups.iter().flat_map(|g| sections(g)).collect();
    let mut moves = Vec::new();
    for _ in 0..[0, 1, 1, 2, 3][rng.below(5)] {
        moves.push(mutate(rng, &mut all));
    }
    let mut text = String::new();
    for section in &all {
        for line in section.header.iter().chain(&section.body) {
            text.push_str(line);
            text.push('\n');
        }
    }
    Generated {
        text,
        written: moves.is_empty() && distinct && !end_anchored,
        moves,
        unanchored,
        end_anchored,
    }
}

/// What one thread found.
#[derive(Default)]
struct Tally {
    patches: usize,
    read: usize,
    applied_as_read: usize,
    /// DDF reads, Git reads and refuses to apply, writing nothing: a binary marker whose ids
    /// are abbreviated (a plain `git diff` of a binary file), a `--submodule=diff` envelope's
    /// inner section, and any other patch Git refuses whole (a hunk that does not fit the tree,
    /// a file that already exists, …).
    abbreviated_binary: usize,
    submodule_diff: usize,
    refused_whole: usize,
    /// Git applies, DDF refuses: by DDF's reason and the moves that made the patch.
    git_only: BTreeMap<String, usize>,
    /// DDF reads, Git refuses to read: a section shaped as `git diff --no-index` writes a binary
    /// or mode-only pair of two files (the stated limit of `CONFORMANCE.md`, row `diff-git`).
    no_index: usize,
    /// Git applies, DDF refuses by design: a `git diff -U0` hunk Git places elsewhere than its
    /// header says ([`WRITTEN_U0`], `HunkPlacedElsewhere`).
    unanchored: usize,
    /// DDF reads and Git applies as reviewed: a hand-written hunk Git anchors at the end
    /// ([`END_ANCHORED`]).
    end_anchored: usize,
    failures: Vec<String>,
}

/// Whether a section's `diff --git` halves name two paths with nothing else naming the files
/// and no file created or deleted: what `git diff --no-index` writes for a binary or mode-only
/// pair, which Git's own `git apply` cannot read.
fn no_index_pair(patch: &str) -> bool {
    sections(patch).iter().any(|s| {
        let named = s.header.iter().any(|l| {
            [
                "--- ",
                "rename ",
                "copy ",
                "new file mode ",
                "deleted file mode ",
            ]
            .iter()
            .any(|p| l.starts_with(p))
        });
        let halves = names(s);
        halves.len() == 2 && halves[0] != halves[1] && !named
    })
}

fn fill(text: &str, ids: &BTreeMap<&str, String>) -> String {
    let mut out = text.to_owned();
    for (name, id) in ids {
        out = out.replace(&format!("@{name}7@"), &id[..7]);
        out = out.replace(&format!("@{name}@"), id);
    }
    let len = ids.values().next().map_or(40, String::len);
    out.replace("@ZERO7@", "0000000")
        .replace("@ZERO@", &"0".repeat(len))
        .replace("@SUB@", &"1".repeat(len))
}

fn run(seeds: &[u64], tag: &str, format: &str) -> Tally {
    let blobs: Vec<&[u8]> = BLOBS.iter().map(|(_, b)| *b).collect();
    let repo = Repo::with_format(tag, &blobs, format);
    let ids: BTreeMap<&str, String> = BLOBS
        .iter()
        .map(|(name, _)| *name)
        .zip(repo.ids.iter().cloned())
        .collect();
    let before = before();
    let mut tally = Tally::default();
    for &seed in seeds {
        let mut rng = Rng::new(seed);
        for n in 0..PER_SEED {
            let generated = generate(&mut rng);
            let patch = fill(&generated.text, &ids);
            tally.patches += 1;
            let ddf = parse_decoded(&decode(patch.as_bytes()), ParseOptions::default());
            let reading = repo.reading(patch.as_bytes());
            let case = format!(
                "seed {seed} patch {n} moves {:?}:\n{patch}",
                generated.moves
            );
            match &ddf {
                Ok(files) => {
                    tally.read += 1;
                    if reading.is_none() {
                        if no_index_pair(&patch) {
                            tally.no_index += 1;
                        } else {
                            tally
                                .failures
                                .push(format!("DDF reads what Git refuses to read: {case}"));
                        }
                        continue;
                    }
                    let (applied, stderr, git) = repo.apply_outcome(&before, patch.as_bytes());
                    if !applied {
                        // C1, second outcome: Git refuses the whole patch and writes nothing.
                        if git != before {
                            tally.failures.push(format!(
                                "Git refused part of a patch DDF reads: {case}\n git: {git:?}"
                            ));
                        } else if stderr.contains("without full index line") {
                            tally.abbreviated_binary += 1;
                        } else if patch.lines().any(|l| l.starts_with("Submodule ")) {
                            tally.submodule_diff += 1;
                        } else {
                            tally.refused_whole += 1;
                        }
                        continue;
                    }
                    match reviewed_tree(&before, files, &repo.contents) {
                        Some(reviewed) if agree(&git, &reviewed) => {
                            tally.applied_as_read += 1;
                            tally.end_anchored += usize::from(generated.end_anchored);
                        }
                        reviewed => tally.failures.push(format!(
                            "git apply writes otherwise than DDF reads: {case}\n git: {git:?}\n ddf: {reviewed:?}"
                        )),
                    }
                }
                Err(e) => {
                    repo.reset(&before);
                    if reading.is_some() && repo.checks(patch.as_bytes()) {
                        let reason = format!("{e:?}");
                        let reason = reason.split([' ', '{']).next().unwrap_or("").to_owned();
                        *tally
                            .git_only
                            .entry(format!("{reason} after {:?}", generated.moves))
                            .or_default() += 1;
                        let by_design = generated.unanchored
                            && matches!(e, PatchParseError::HunkPlacedElsewhere { .. });
                        tally.unanchored += usize::from(generated.written && by_design);
                        if generated.written && !by_design {
                            tally
                                .failures
                                .push(format!("DDF refuses what Git writes ({e}): {case}"));
                        }
                    }
                }
            }
        }
    }
    tally
}

#[test]
fn every_generated_patch_ddf_reads_is_applied_by_git_as_reviewed() {
    let seeds: Vec<u64> = match std::env::var("DDF_DIFFERENTIAL_SEEDS") {
        Ok(n) => (1..=n.parse::<u64>().expect("DDF_DIFFERENTIAL_SEEDS is a count")).collect(),
        Err(_) => (1..=SEEDS).collect(),
    };
    let started = std::time::Instant::now();
    let tallies: Vec<Tally> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..THREADS)
            .map(|t| {
                let mine: Vec<u64> = seeds.iter().copied().filter(|s| s % THREADS == t).collect();
                let format = if t % 2 == 1 { "sha256" } else { "sha1" };
                scope.spawn(move || run(&mine, &format!("gen{t}"), format))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut total = Tally::default();
    for tally in tallies {
        total.patches += tally.patches;
        total.read += tally.read;
        total.applied_as_read += tally.applied_as_read;
        total.no_index += tally.no_index;
        total.abbreviated_binary += tally.abbreviated_binary;
        total.submodule_diff += tally.submodule_diff;
        total.refused_whole += tally.refused_whole;
        total.unanchored += tally.unanchored;
        total.end_anchored += tally.end_anchored;
        for (shape, n) in tally.git_only {
            *total.git_only.entry(shape).or_default() += n;
        }
        total.failures.extend(tally.failures);
    }
    let git_only: usize = total.git_only.values().sum();
    eprintln!(
        "generated differential: {} seeds, {} patches in {:.1} s; DDF read {}, Git applied {} of them as reviewed; Git applies and DDF refuses {git_only}",
        seeds.len(),
        total.patches,
        started.elapsed().as_secs_f64(),
        total.read,
        total.applied_as_read,
    );
    eprintln!(
        "  DDF reads, git apply refuses whole and writes nothing: abbreviated binary ids {}, --submodule=diff {}, --no-index pairs (Git cannot read them) {}, other {}",
        total.abbreviated_binary, total.submodule_diff, total.no_index, total.refused_whole,
    );
    eprintln!(
        "  Git writes, git apply places elsewhere, DDF refuses by design: -U0 hunks {}",
        total.unanchored
    );
    eprintln!(
        "  written by hand, Git anchors at the end, applied as reviewed: {}",
        total.end_anchored
    );
    for failure in total.failures.iter().take(40) {
        eprintln!("  FAIL {}", failure.lines().next().unwrap_or_default());
    }
    let mut shapes: Vec<_> = total.git_only.iter().collect();
    shapes.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    for (shape, n) in shapes.iter().take(25) {
        eprintln!("  {n:5}  {shape}");
    }
    // Failures first, so a plant or a control names them, not a count it starves.
    assert!(
        total.failures.is_empty(),
        "{} failures; the first:\n{}",
        total.failures.len(),
        total
            .failures
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n---\n")
    );
    // Not vacuous: the plain binary group is reached, and most of Git's own patches and many
    // near misses are read and applied.
    assert!(
        total.abbreviated_binary > 0,
        "no abbreviated binary diff was generated"
    );
    assert!(total.unanchored > 0, "no -U0 hunk was generated");
    assert!(total.end_anchored > 0, "no end-anchored hunk was applied");
    assert!(
        total.applied_as_read * 4 >= total.patches,
        "only {} of {} patches were read and applied",
        total.applied_as_read,
        total.patches
    );
}
