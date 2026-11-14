#![no_main]
// The integrity signals of a Rust hunk, read from its tokens (agent batteries; the graph
// crate's `rust_tokens`). Until 2026-11-14 a parser read each hunk and this target
// (`fragment_scan`) held it to one rule, that it removed no evidence. The parser is out of
// the evidence path; what is held here is the reader of tokens, for any text, as a new file
// and as a change with added, removed and unchanged lines:
//
// 1. Ranking never panics, and the same patch gives the same evidence twice.
// 2. Every item is on a line the change made: an added line of its hunk, or for
//    `test_removed` a removed one.
// 3. A hunk's labels are the signals of the file's evidence in that hunk.
// 4. Respacing changes nothing. The white space between two tokens of an added line is
//    replaced by a line end, so that what was written on one added line is written over two
//    (once at one place the input picks, and once at every place, which leaves a token a
//    line), and the set of signals the reader of tokens gives for the file must stay what it
//    was: `panic_path_added`, `trivial_assert`, `stub_added`, `test_skipped`, `test_removed`.
//    The other Rust rules read one line by design and are not compared.
//
//    Rule 4 is checked only for a text with no `"`, `'`, `/` or `#` byte, so that no
//    string, character, comment or raw-string state is involved: a line end does end a line
//    comment and can stand inside a string, and there respacing is a change. (So the
//    attribute signals, which need a `#`, are held by rules 1 to 3 and by the unit tests,
//    not by rule 4.)
//
// Input: the first byte picks the shape (bit 0: a new file, or a change), a path (bits 1 and
// 2) and the place of the one respacing (the bits above); the rest is the text.

use deep_diff_forge_core::PatchLineKind;
use deep_diff_forge_graph::{EvidenceSignal, Side, file_evidence, hunk_signals};
use libfuzzer_sys::fuzz_target;

const PATHS: &[&str] = &[
    "src/lib.rs",
    "tests/it.rs",
    "src/generated/x.rs",
    "build.rs",
];

/// The signals the reader of tokens gives.
const READ: [EvidenceSignal; 5] = [
    EvidenceSignal::PanicPathAdded,
    EvidenceSignal::TrivialAssert,
    EvidenceSignal::StubAdded,
    EvidenceSignal::TestSkipped,
    EvidenceSignal::TestRemoved,
];

/// A patch of one hunk for `path` from `lines`, each with its sign.
fn patch(path: &str, new_file: bool, lines: &[(char, String)]) -> Option<String> {
    let mut body = String::new();
    let (mut old, mut new) = (0u32, 0u32);
    for (sign, line) in lines {
        body.push(*sign);
        body.push_str(line);
        body.push('\n');
        old += u32::from(*sign != '+');
        new += u32::from(*sign != '-');
    }
    if new == 0 {
        return None;
    }
    Some(if new_file {
        format!(
            "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{new} @@\n{body}"
        )
    } else {
        format!("--- a/{path}\n+++ b/{path}\n@@ -1,{old} +1,{new} @@\n{body}")
    })
}

/// The signals of the reader of tokens in `text` (a patch), in order and without repeats, after
/// rules 1 to 3 are checked on it. `None` when it is no patch.
fn checked(text: &str) -> Option<Vec<EvidenceSignal>> {
    let files = deep_diff_forge_patch::parse(text).ok()?;
    let mut read = Vec::new();
    for file in &files {
        let evidence = file_evidence(file);
        assert_eq!(evidence, file_evidence(file), "two scans differ");
        for item in &evidence {
            let hunk = file
                .patch_twin
                .hunks
                .iter()
                .find(|hunk| hunk.id == item.hunk)
                .expect("evidence names a hunk of its file");
            let (wanted, on_side): (PatchLineKind, fn(&deep_diff_forge_core::PatchLine) -> Option<u32>) =
                match item.side {
                    Side::New => (PatchLineKind::Added, |line| line.new_line),
                    Side::Old => (PatchLineKind::Removed, |line| line.old_line),
                };
            assert!(
                hunk.lines
                    .iter()
                    .any(|line| line.kind == wanted && on_side(line) == Some(item.line)),
                "{} at {} line {} of {} is not on a line the change made",
                item.signal.label(),
                item.side.label(),
                item.line,
                file.path
            );
            assert_eq!(
                item.side == Side::Old,
                item.signal == EvidenceSignal::TestRemoved,
                "{} is on the {} side",
                item.signal.label(),
                item.side.label()
            );
            assert!(
                !item.test_likely || item.signal == EvidenceSignal::PanicPathAdded,
                "{} carries the hint of a panic path",
                item.signal.label()
            );
        }
        for hunk in &file.patch_twin.hunks {
            let mut in_hunk: Vec<EvidenceSignal> = evidence
                .iter()
                .filter(|item| item.hunk == hunk.id)
                .map(|item| item.signal)
                .collect();
            in_hunk.sort();
            in_hunk.dedup();
            assert_eq!(hunk_signals(file, hunk), in_hunk, "a hunk's labels differ");
        }
        read.extend(
            evidence
                .iter()
                .map(|item| item.signal)
                .filter(|signal| READ.contains(signal)),
        );
    }
    read.sort();
    read.dedup();
    Some(read)
}

/// The places in `line` where white space stands between two tokens: the byte ranges of each
/// run of spaces and tabs that has something before it and something after it.
fn gaps(line: &str) -> Vec<(usize, usize)> {
    let bytes = line.as_bytes();
    let blank = |byte: u8| byte == b' ' || byte == b'\t';
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if !blank(bytes[at]) {
            at += 1;
            continue;
        }
        let start = at;
        while at < bytes.len() && blank(bytes[at]) {
            at += 1;
        }
        if start > 0 && at < bytes.len() {
            found.push((start, at));
        }
    }
    found
}

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else {
        return;
    };
    if rest.len() > 48 * 1024 {
        return;
    }
    let text: String = String::from_utf8_lossy(rest)
        .chars()
        .filter(|c| *c != '\r')
        .collect();
    let path = PATHS[usize::from(pick >> 1) % PATHS.len()];
    let new_file = pick & 1 == 1;
    let lines: Vec<(char, String)> = text
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let sign = match (new_file, i % 3) {
                (true, _) | (false, 0) => '+',
                (false, 1) => '-',
                (false, _) => ' ',
            };
            (sign, line.to_owned())
        })
        .collect();
    let Some(read) = patch(path, new_file, &lines).and_then(|text| checked(&text)) else {
        return;
    };

    // Rule 4.
    if text.bytes().any(|byte| matches!(byte, b'"' | b'\'' | b'/' | b'#')) {
        return;
    }
    let spots: Vec<(usize, usize, usize)> = lines
        .iter()
        .enumerate()
        .filter(|(_, (sign, _))| *sign == '+')
        .flat_map(|(at, (_, line))| gaps(line).into_iter().map(move |(from, to)| (at, from, to)))
        .collect();
    if spots.is_empty() {
        return;
    }
    // once, at the place the input picks
    let (at, from, to) = spots[usize::from(pick >> 3) % spots.len()];
    let mut once = lines.clone();
    let tail = once[at].1.split_off(to);
    once[at].1.truncate(from);
    once.insert(at + 1, ('+', tail));
    // at every place: each added line is cut at each of its gaps
    let every: Vec<(char, String)> = lines
        .iter()
        .flat_map(|(sign, line)| {
            if *sign == '+' {
                line.split([' ', '\t'])
                    .filter(|part| !part.is_empty())
                    .map(|part| ('+', part.to_owned()))
                    .chain(line.trim_matches([' ', '\t']).is_empty().then(|| ('+', String::new())))
                    .collect::<Vec<_>>()
            } else {
                vec![(*sign, line.clone())]
            }
        })
        .collect();
    for (name, respaced) in [("one gap", once), ("every gap", every)] {
        let again = patch(path, new_file, &respaced)
            .and_then(|text| checked(&text))
            .expect("a respaced patch is a patch");
        assert_eq!(
            again
                .iter()
                .map(|signal| signal.label())
                .collect::<Vec<_>>(),
            read.iter().map(|signal| signal.label()).collect::<Vec<_>>(),
            "respacing ({name}) changed the signals of {path}"
        );
    }
});
