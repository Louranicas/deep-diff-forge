#![no_main]
// Where a Rust hunk is and how it starts (agent batteries; the graph crate's `evidence`).
//
// Nothing about a hunk's place or its start is taken on the patch writer's word: the top of
// a file is where Git must put the hunk, hunks Git must put together are one text, and a
// fragment is read from every start it can have. This target holds that: it builds a whole
// Rust file whose every line is in a KNOWN state, takes the evidence of the file read whole
// (which the lexer oracle of `rust_code_view` holds), writes the same change as a patch in
// other ways Git accepts, and requires that no item of the whole reading is missing.
//
// The file is built from units, so that what each line leaves open is known by construction:
// code lines that are constructs (`x.unwrap();`, `todo!();`, `#[ignore]`, `unsafe { f() }`,
// a spawn, ...), a call written over two lines, strings and raw strings over lines, block
// comments over lines (nested ones too), line comments, and a line `#! ...` that is code in
// the middle of a file and a shebang on line 1. The text inside strings and comments comes
// from the input and is anything that does not end them: quotes in comments, `/*`, `*/` and
// `//` in strings, code in both.
//
// 1. CREATION IN SEVERAL HUNKS (sixth independent review, IR6-H-3). The file as one created
//    hunk, and as two to four, cut at lines the input picks, with honest headers
//    (`@@ -2,0 +3,N @@`) or false ones (`@@ -50,0 +90,N @@`): every item of the one hunk is
//    an item of the several, on the same line of the file. Git applies both the same way.
// 2. A FRAGMENT WITH UNSEEN LINES ABOVE AND BELOW (IR6-H-2; review V3 round 12, VD3R12-M-1;
//    the review of the port, LF-H-1). Some lines of the file are the change, and the patch
//    shows it twice: with the whole file as context (a hunk at the top: nothing is unknown),
//    and as a hunk cut out at lines the input picks. Every item of the first is an item of
//    the second, whatever the cut left open: code, a block comment, a string or a raw
//    string. Two starts are declared limits and are not asked for: a cut two or more
//    comments deep, and a cut inside a string or raw string whose reading from code carries
//    nothing over a line end. A call written over two lines is never cut in two (a hunk
//    cannot show what it does not hold).
// 3. THE NEW NUMBER OF THE HEADER (IR6-H-1). The same fragment with the new position of its
//    header set to 1, and to a number the input picks: the same items on the same lines of
//    the hunk. Git places a hunk by its old position.
// 5. NO JOIN TAKES A READING AWAY. Hunks Git must put together, or whose positions say they
//    touch, are read as one text, and each of them is still read on its own, as every
//    version before read it and as an applier that trusts positions places it
//    (`git apply --unidiff-zero`, which a `git diff -U0` patch needs). For the creation in
//    several hunks, and for two runs of the file's lines written as hunks of added lines
//    only in an existing file with unseen lines between them: what each hunk gives as a
//    patch of its own is among what the file gives for it. (As first built, round 14 read
//    a hunk that Git must put after another only as its continuation: a comment opened in
//    the first of two such hunks hid a call in the second.)
// 4. NOTHING THE RELEASED VERSION REPORTS IS LOST (the review of the port, LF-H-1). For the
//    built file as a created file and as the fragment, and for the input's own bytes as
//    lines (any text at all) as a created file, as added lines below line 1 and as removed
//    lines: what the scanner and the Rust token rules of release 0.8.3 find on a changed
//    line (`unsafe`, a spawn, a fetch, a removed `#[test]`) is found.
//
// Input: six bytes of choices (the cut, the changed lines, how a creation is cut and
// whether its headers are honest, the new number), then lines. The first byte of a line
// picks the unit, the rest is its text (`|` parts it over the unit's lines).

use deep_diff_forge_core::{PatchLineKind, ReviewFile};
use deep_diff_forge_graph::lexical::{
    RustStart, has_token, rust_legacy_views, rust_side, rust_views,
};
use deep_diff_forge_graph::{EvidenceSignal, Side, file_evidence};
use libfuzzer_sys::fuzz_target;
use std::collections::BTreeSet;

const PATH: &str = "src/lib.rs";

/// What a line of the built file leaves open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Open {
    Code,
    Comment(usize),
    Str,
    Raw,
}

/// The built file: its lines, what each leaves open, and whether a line and the next are one
/// construct (no cut between them).
#[derive(Default)]
struct Built {
    lines: Vec<String>,
    after: Vec<Open>,
    glued: Vec<bool>,
}

impl Built {
    fn push(&mut self, line: String, after: Open) {
        self.lines.push(line);
        self.after.push(after);
        self.glued.push(false);
    }
}

/// Code lines that are constructs or their surroundings.
const CODE: &[&str] = &[
    "    x.unwrap();",
    "    let v = y.expect(\"set\");",
    "    todo!();",
    "    unimplemented!();",
    "#[ignore]",
    "#[test]",
    "#[cfg(test)]",
    "fn check() {",
    "}",
    "    assert!(true);",
    "    unsafe { f() }",
    "    std::process::Command::new(\"sh\").status().ok();",
    "    let n = reqwest::get(u);",
    "    let c = '\"';",
    "    let l: &'a str = s;",
    "#[allow(dead_code)]",
    "",
];

/// A call written over two lines.
const SPLIT: &[(&str, &str)] = &[
    ("    x.unwrap", "    ();"),
    ("    todo", "    !();"),
    ("    Option::unwrap", "    (x);"),
    ("#[", "ignore]"),
];

/// The text of a unit: `raw` less the characters `drop` names and control characters, at most
/// 200 characters.
fn text(raw: &str, drop: &[char]) -> String {
    raw.chars()
        .filter(|c| !c.is_control() && !drop.contains(c))
        .take(200)
        .collect()
}

/// The file the input's lines describe.
fn build(script: &str) -> Built {
    let mut built = Built::default();
    for line in script.lines().take(60) {
        let Some(kind) = line.bytes().next() else {
            built.push(String::new(), Open::Code);
            continue;
        };
        let rest = line.get(1..).unwrap_or("");
        // (a digit picks the unit of its value, and a letter the entry of its place in the
        // alphabet, so that a seed can be written by hand)
        let pick = usize::from(rest.bytes().next().unwrap_or(b'a').wrapping_sub(b'a'));
        match kind.wrapping_sub(b'0') % 10 {
            0 => built.push(CODE[pick % CODE.len()].to_owned(), Open::Code),
            1 => {
                let (first, second) = SPLIT[pick % SPLIT.len()];
                built.push(first.to_owned(), Open::Code);
                *built.glued.last_mut().expect("a line was pushed") = true;
                built.push(second.to_owned(), Open::Code);
            }
            // a string over lines: no quote and no backslash in its text
            2 => {
                let parts: Vec<String> = rest.split('|').map(|p| text(p, &['"', '\\'])).collect();
                built.push(format!("    let s = \"{}", parts[0]), Open::Str);
                for part in &parts[1..] {
                    built.push(part.clone(), Open::Str);
                }
                built.push("\"; g();".to_owned(), Open::Code);
            }
            // a raw string over lines, with one or two `#`: no quote in its text. It closes
            // on a line of its own, or (for an odd pick) at the end of its last line of text,
            // where a backslash can stand before the quote.
            3 => {
                let hashes = "#".repeat(1 + pick % 2);
                let mut parts: Vec<String> = rest.split('|').map(|p| text(p, &['"'])).collect();
                built.push(
                    format!("    let r = r{hashes}\"{}", parts.remove(0)),
                    Open::Raw,
                );
                let last = if pick % 4 < 2 {
                    String::new()
                } else {
                    parts.pop().unwrap_or_default()
                };
                for part in parts {
                    built.push(part, Open::Raw);
                }
                built.push(format!("{last}\"{hashes}; g();"), Open::Code);
            }
            // a block comment over lines: no `*` in its text
            4 => {
                let parts: Vec<String> = rest.split('|').map(|p| text(p, &['*'])).collect();
                built.push(format!("/* {}", parts[0]), Open::Comment(1));
                for part in &parts[1..] {
                    built.push(part.clone(), Open::Comment(1));
                }
                built.push(" */ g();".to_owned(), Open::Code);
            }
            // a nested block comment; for an odd pick, one whose closers stand behind `//`
            // and whose last closer has a call after it (review V3 round 13, VD3R13-M-1)
            5 => {
                let t = text(rest, &['*']);
                built.push(format!("/* {t} /* {t}"), Open::Comment(2));
                if pick % 2 == 0 {
                    built.push(format!("{t} */ inner"), Open::Comment(1));
                    built.push("outer */ g();".to_owned(), Open::Code);
                } else {
                    built.push("// */ //".to_owned(), Open::Comment(1));
                    built.push("// */ x.unwrap(); /*".to_owned(), Open::Comment(1));
                    built.push(" */ g();".to_owned(), Open::Code);
                }
            }
            6 => built.push(format!("// {}", text(rest, &[])), Open::Code),
            7 => built.push(
                format!("    let s = \"{}\"; g();", text(rest, &['"', '\\'])),
                Open::Code,
            ),
            // plain code of the input's own: nothing that opens anything. It may be half of
            // a construct, so no cut is made beside it.
            8 => {
                if let Some(before) = built.glued.last_mut() {
                    *before = true;
                }
                built.push(text(rest, &['"', '\'', '/', '*', '\\', '#']), Open::Code);
                *built.glued.last_mut().expect("a line was pushed") = true;
            }
            _ => built.push(
                format!("#! {}", CODE[pick % CODE.len()].trim_start()),
                Open::Code,
            ),
        }
    }
    built
}

fn parsed(patch: &str) -> Option<ReviewFile> {
    let mut files = deep_diff_forge_patch::parse(patch).ok()?;
    (files.len() == 1).then(|| files.remove(0))
}

/// The items of `file`, each as its signal, its side and the place of its line among the
/// lines of the file's hunks (so that a header's numbers do not matter).
fn places(file: &ReviewFile) -> BTreeSet<(&'static str, bool, usize)> {
    let evidence = file_evidence(file);
    let mut found = BTreeSet::new();
    let mut before = 0;
    for hunk in &file.patch_twin.hunks {
        for item in evidence.iter().filter(|item| item.hunk == hunk.id) {
            let at = hunk.lines.iter().position(|line| match item.side {
                Side::New => line.kind == PatchLineKind::Added && line.new_line == Some(item.line),
                Side::Old => {
                    line.kind == PatchLineKind::Removed && line.old_line == Some(item.line)
                }
            });
            let at = at.expect("an item stands on a changed line of its hunk");
            found.insert((item.signal.label(), item.side == Side::New, before + at));
        }
        before += hunk.lines.len();
    }
    found
}

/// Whether an item of the file read whole (`signal` on line `at` of the file) is one the
/// compiler's reading of the file gives: a word of the construct stands in the code view of
/// that line (for `#[` and `ignore]` on two lines, of the next). The reading
/// of release 0.8.3 is one of the readings of every hunk, and what only it finds (text of a
/// string after a string over lines, say) depends on where a hunk begins: that is no item
/// another way of writing the patch has to give. A rule that reads the raw line gives the
/// same answer however the patch is written.
fn real(views: &[String], signal: &str, at: usize) -> bool {
    let words: &[&str] = match signal {
        "unsafe_added" => &["unsafe"],
        "process_spawn" => &["Command"],
        "network_fetch" => &["reqwest"],
        "panic_path_added" => &["unwrap", "expect"],
        "stub_added" => &["todo", "unimplemented"],
        "test_skipped" => &["ignore"],
        "trivial_assert" => &["assert"],
        _ => return true,
    };
    let has = |at: usize| {
        views
            .get(at)
            .is_some_and(|view| words.iter().any(|word| view.contains(word)))
    };
    has(at) || (views[at].trim_end().ends_with("#[") && has(at + 1))
}

/// Rule 4 for `file`: what release 0.8.3 found on each changed line of each hunk is found.
fn released_is_kept(file: &ReviewFile, what: &str) {
    let now = file_evidence(file);
    for hunk in &file.patch_twin.hunks {
        for (side, other, changed) in [
            (Side::New, PatchLineKind::Removed, PatchLineKind::Added),
            (Side::Old, PatchLineKind::Added, PatchLineKind::Removed),
        ] {
            let lines: Vec<_> = hunk.lines.iter().filter(|l| l.kind != other).collect();
            let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
            for (line, view) in lines.iter().zip(rust_legacy_views(&texts)) {
                if line.kind != changed {
                    continue;
                }
                let any = |tokens: &[&str]| tokens.iter().any(|t| has_token(&view, t));
                let signals: &[(EvidenceSignal, bool)] = match side {
                    Side::New => &[
                        (EvidenceSignal::UnsafeAdded, any(&["unsafe"])),
                        (
                            EvidenceSignal::ProcessSpawn,
                            any(&["Command::new", "process::Command"]),
                        ),
                        (
                            EvidenceSignal::NetworkFetch,
                            any(&[
                                "reqwest::",
                                "ureq::",
                                "TcpStream::connect",
                                "UdpSocket::bind",
                            ]),
                        ),
                    ],
                    Side::Old => &[(
                        EvidenceSignal::TestRemoved,
                        any(&["#[test]", "#[tokio::test]", "#[rstest]"]),
                    )],
                };
                let number = match side {
                    Side::New => line.new_line,
                    Side::Old => line.old_line,
                };
                for (signal, released) in signals {
                    assert!(
                        !released
                            || now.iter().any(|item| item.hunk == hunk.id
                                && item.signal == *signal
                                && item.side == side
                                && Some(item.line) == number),
                        "{what}: release 0.8.3 reports {} on line {number:?} ({:?}), and it is lost",
                        signal.label(),
                        line.text
                    );
                }
            }
        }
    }
}

/// Rule 5 for `file`, whose section creates its file when `created`: each hunk, written as a
/// patch of its own (the first hunk of a creation as a creation, any other as a hunk of an
/// existing file, which is what it is alone), gives nothing the file does not give for it.
fn alone_is_kept(file: &ReviewFile, created: bool, what: &str) {
    let of_file = places(file);
    let mut before = 0;
    for (index, part) in file.patch_twin.hunks.iter().enumerate() {
        let (old, new) = part.counts();
        let header = format!(
            "@@ -{},{old} +{},{new} @@",
            part.old_start.unwrap_or(0),
            part.new_start.unwrap_or(0)
        );
        let lines: Vec<(char, &str)> = part
            .lines
            .iter()
            .map(|line| {
                let sign = match line.kind {
                    PatchLineKind::Added => '+',
                    PatchLineKind::Removed => '-',
                    PatchLineKind::Context => ' ',
                };
                (sign, line.text.as_str())
            })
            .collect();
        let body = hunk(&header, &lines);
        let alone = if created && index == 0 {
            creation(&body)
        } else {
            change(&body)
        };
        let alone = parsed(&alone).expect("a hunk alone is a patch");
        for (signal, new_side, at) in places(&alone) {
            assert!(
                of_file.contains(&(signal, new_side, before + at)),
                "{what}: {signal} on line {at} of hunk {index} read alone is missing from the file's answer"
            );
        }
        before += part.lines.len();
    }
}

/// One hunk: `lines` with their signs, under `header`.
fn hunk(header: &str, lines: &[(char, &str)]) -> String {
    let mut out = format!("{header}\n");
    for (sign, line) in lines {
        out.push(*sign);
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn creation(hunks: &str) -> String {
    format!(
        "diff --git a/{PATH} b/{PATH}\nnew file mode 100644\n--- /dev/null\n+++ b/{PATH}\n{hunks}"
    )
}

fn change(hunks: &str) -> String {
    format!("--- a/{PATH}\n+++ b/{PATH}\n{hunks}")
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 7 || data.len() > 16 * 1024 {
        return;
    }
    let (choice, rest) = data.split_at(6);
    let script: String = String::from_utf8_lossy(rest)
        .chars()
        .filter(|c| *c != '\r')
        .collect();

    // Rule 4 on the input's own bytes, as lines.
    let own: Vec<&str> = script.lines().take(200).collect();
    if !own.is_empty() {
        let n = own.len();
        let added: Vec<(char, &str)> = own.iter().map(|line| ('+', *line)).collect();
        let removed: Vec<(char, &str)> = own.iter().map(|line| ('-', *line)).collect();
        for (what, patch) in [
            (
                "any text, created",
                creation(&hunk(&format!("@@ -0,0 +1,{n} @@"), &added)),
            ),
            (
                "any text, added below",
                change(&hunk(&format!("@@ -7,0 +8,{n} @@"), &added)),
            ),
            (
                "any text, removed",
                change(&hunk(&format!("@@ -8,{n} +7,0 @@"), &removed)),
            ),
        ] {
            if let Some(file) = parsed(&patch) {
                released_is_kept(&file, what);
            }
        }
    }

    let built = build(&script);
    let n = built.lines.len();
    if n < 2 {
        return;
    }
    let all: Vec<(char, &str)> = built.lines.iter().map(|l| ('+', l.as_str())).collect();

    // Rule 1: the file as one created hunk, and as several.
    let Some(one) = parsed(&creation(&hunk(&format!("@@ -0,0 +1,{n} @@"), &all))) else {
        return;
    };
    released_is_kept(&one, "the file, created");
    let texts: Vec<&str> = built.lines.iter().map(String::as_str).collect();
    let views = rust_views(&texts, RustStart::FileStart);
    let whole: BTreeSet<_> = places(&one)
        .into_iter()
        .filter(|(signal, _, at)| real(&views, signal, *at))
        .collect();
    let mut cuts: Vec<usize> = (0..=usize::from(choice[4] >> 1) % 3)
        .map(|k| 1 + (usize::from(choice[k]) + 7 * k) % (n - 1))
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let honest = choice[4] & 1 == 0;
    let mut several = String::new();
    let mut from = 0;
    for to in cuts.iter().copied().chain([n]) {
        let header = match (from, honest) {
            (0, _) => format!("@@ -0,0 +1,{} @@", to - from),
            (_, true) => format!("@@ -{from},0 +{},{} @@", from + 1, to - from),
            (_, false) => format!("@@ -{},0 +{},{} @@", from + 50, 90 + from, to - from),
        };
        several.push_str(&hunk(&header, &all[from..to]));
        from = to;
    }
    let several = parsed(&creation(&several)).expect("a creation in several hunks is a patch");
    alone_is_kept(&several, true, "the file created as several hunks");
    let found = places(&several);
    for item in &whole {
        assert!(
            found.contains(item),
            "{item:?} of the file as one hunk is missing when it is created as hunks cut at {cuts:?} (honest headers: {honest}): {:?}",
            built.lines
        );
    }

    // Rule 2: a change of some lines, with the whole file as context and as a fragment.
    // The cut: lines `first..last`; the change: `from..to` inside it.
    let boundary = |at: usize| at == 0 || at >= n || !built.glued[at - 1];
    let first = 1 + usize::from(choice[0]) % (n - 1);
    let last = (first + 1 + usize::from(choice[1]) % (n - first)).min(n);
    if !boundary(first) || !boundary(last) {
        return;
    }
    let from = first + usize::from(choice[2]) % (last - first);
    let to = from + 1 + usize::from(choice[3]) % (last - from);
    let signed = |range: std::ops::Range<usize>| -> Vec<(char, &str)> {
        range
            .map(|at| {
                let sign = if (from..to).contains(&at) { '+' } else { ' ' };
                (sign, built.lines[at].as_str())
            })
            .collect()
    };
    // Rule 5 for an existing file: the lines before the cut and the lines from the change
    // on, as two hunks of added lines only, the lines between them unseen.
    let apart = format!(
        "{}{}",
        hunk(&format!("@@ -4,0 +5,{first} @@"), &all[..first]),
        hunk(
            &format!("@@ -{},0 +{},{} @@", 9 + from, 10 + from + first, n - from),
            &all[from..]
        )
    );
    let apart = parsed(&change(&apart)).expect("two hunks of added lines are a patch");
    alone_is_kept(
        &apart,
        false,
        "two hunks of added lines in an existing file",
    );
    released_is_kept(&apart, "two hunks of added lines");

    let changed = to - from;
    let Some(full) = parsed(&change(&hunk(
        &format!("@@ -1,{} +1,{n} @@", n - changed),
        &signed(0..n),
    ))) else {
        return;
    };
    let truth: BTreeSet<_> = places(&full)
        .into_iter()
        .filter(|(signal, _, at)| (first..last).contains(at) && real(&views, signal, *at))
        .map(|(signal, new, at)| (signal, new, at - first))
        .collect();
    let fragment = signed(first..last);
    let start = built.after[first - 1];
    let side: Vec<&str> = fragment.iter().map(|(_, line)| *line).collect();
    let declared = match start {
        Open::Code | Open::Comment(1) => false,
        Open::Comment(_) => true,
        Open::Str | Open::Raw => !rust_side(&side, RustStart::Code).open_over_lines,
    };
    let size = last - first;
    for (what, new_start) in [
        ("with honest numbers", first + 1),
        ("with the new number 1", 1),
        (
            "with a new number of the input's",
            1 + usize::from(choice[5]) * 7,
        ),
    ] {
        let header = format!(
            "@@ -{},{} +{new_start},{size} @@",
            first + 1,
            size - changed
        );
        let file = parsed(&change(&hunk(&header, &fragment))).expect("a fragment is a patch");
        released_is_kept(&file, "the fragment");
        if declared {
            continue;
        }
        let found = places(&file);
        for item in &truth {
            assert!(
                found.contains(item),
                "{item:?} of the change read with the whole file is missing from the fragment of lines {first}..{last} {what} (it begins in {start:?}; changed lines {from}..{to}): {:?}",
                built.lines
            );
        }
    }
});
