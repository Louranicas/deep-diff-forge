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
// 5. A comment between two tokens of a construct, and complete string literals on the lines
//    before it, change nothing (fifth independent review, IR5-H-1). The target writes
//    constructs it knows (`x.unwrap()`, `todo!()`, `#[ignore]`, `assert!(true)` and their
//    kin), each on a line of its own. Then it writes them again with a block comment between
//    two tokens of each, holding any text that does not end the comment (quotes included,
//    over lines too), and with string literals of any text on the lines before them, one line
//    or several. The reader's signals must be the same set. Until 2026-11-14 a line after a
//    string over a line end was read two ways and the bytes of both readings were united:
//    `x.unwrap/*""*/()` then had `""` between `unwrap` and `(`, and nothing was found.
//    Checked as a file the patch creates, where the signals must be the same set, and as a
//    hunk below line 1 of an existing file (the literals unchanged lines, the constructs
//    added), where none may be missing. Such a hunk does not say how it starts and is also
//    read from inside a block comment, a string and a raw string: there a `*/` or a quote in
//    the text of a string or of a comment shows what follows it as code, so those readings
//    can add a signal from that text, which is their declared cost (a comment over lines
//    that holds one quote, and after it `todo!()`). They never take one away.
//
// Input: the first byte picks the shape (bit 0: a new file, or a change), a path (bits 1 and
// 2) and the place of the one respacing (the bits above); the rest is the text. For rule 5
// the first line of the text picks constructs and places (a letter `a` to `n` and a digit,
// again and again), the lines up to a line `--` are the comment's text, and the lines after
// it the strings' text.

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
        for (index, hunk) in file.patch_twin.hunks.iter().enumerate() {
            let mut in_hunk: Vec<EvidenceSignal> = evidence
                .iter()
                .filter(|item| item.hunk == hunk.id)
                .map(|item| item.signal)
                .collect();
            in_hunk.sort();
            in_hunk.dedup();
            assert_eq!(hunk_signals(file, index), Some(in_hunk), "a hunk's labels differ");
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

/// The constructs of rule 5: the tokens of one line, and the signal they are.
const CONSTRUCTS: &[(&[&str], EvidenceSignal)] = &[
    (&["x", ".", "unwrap", "(", ")", ";"], EvidenceSignal::PanicPathAdded),
    (&["x", ".", "expect", "(", "0", ")", ";"], EvidenceSignal::PanicPathAdded),
    (&["Option", "::", "unwrap", "(", "x", ")", ";"], EvidenceSignal::PanicPathAdded),
    (&["Result", "::", "<", "u8", ",", "u8", ">", "::", "unwrap", "(", "x", ")", ";"], EvidenceSignal::PanicPathAdded),
    (&["todo", "!", "(", ")", ";"], EvidenceSignal::StubAdded),
    (&["unimplemented", "!", "(", ")", ";"], EvidenceSignal::StubAdded),
    (&["assert", "!", "(", "true", ")", ";"], EvidenceSignal::TrivialAssert),
    (&["assert_eq", "!", "(", "true", ",", "true", ")", ";"], EvidenceSignal::TrivialAssert),
    (&["debug_assert", "!", "(", "true", ")", ";"], EvidenceSignal::TrivialAssert),
    (&["#", "[", "ignore", "]", "fn", "t", "(", ")", "{", "}"], EvidenceSignal::TestSkipped),
    (&["#", "[", "cfg_attr", "(", "all", "(", ")", ",", "ignore", ")", "]", "fn", "t", "(", ")", "{", "}"], EvidenceSignal::TestSkipped),
    (&["#", "[", "cfg_attr", "(", "not", "(", "feature", "=", "\"never)\"", ")", ",", "ignore", ")", "]", "fn", "t", "(", ")", "{", "}"], EvidenceSignal::TestSkipped),
    // `#[ignore(…)]`, a lint a crate can allow (tenth independent review, IR10-M-1)
    (&["#", "[", "ignore", "(", "not_ready", ")", "]", "fn", "t", "(", ")", "{", "}"], EvidenceSignal::TestSkipped),
    (&["#", "[", "cfg_attr", "(", "all", "(", ")", ",", "ignore", "(", "flaky", ")", ")", "]", "fn", "t", "(", ")", "{", "}"], EvidenceSignal::TestSkipped),
];

/// Rule 5 for one shape (a new file, or a hunk below line 1 of an existing one): the lines of
/// `picked` constructs alone, and the same with `comment` between two tokens of each and
/// `literals` on the lines before them, give the same signals of the reader in a new file, and
/// in a hunk lose none.
fn comments_and_earlier_strings_change_nothing(
    path: &str,
    new_file: bool,
    picked: &[(usize, usize)],
    comment: &str,
    literals: &[String],
) {
    let plain: Vec<String> = picked
        .iter()
        .map(|(which, _)| CONSTRUCTS[*which].0.join(" "))
        .collect();
    let commented: Vec<String> = picked
        .iter()
        .map(|(which, gap)| {
            let tokens = CONSTRUCTS[*which].0;
            let gap = 1 + gap % (tokens.len() - 1);
            format!("{}/*{comment}*/{}", tokens[..gap].join(" "), tokens[gap..].join(" "))
        })
        .collect();
    // One hunk: `before` unchanged (or added, in a new file), then `added`.
    let hunk = |before: &[String], added: &[String]| {
        let mut body = String::new();
        let (mut old, mut new) = (0u32, 0u32);
        // a line of text can hold line ends: each is a line of the patch
        for (sign, text) in before
            .iter()
            .map(|text| (if new_file { '+' } else { ' ' }, text))
            .chain(added.iter().map(|text| ('+', text)))
        {
            for line in text.split('\n') {
                body.push(sign);
                body.push_str(line);
                body.push('\n');
                old += u32::from(sign == ' ');
                new += 1;
            }
        }
        if new_file {
            format!(
                "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{new} @@\n{body}"
            )
        } else {
            // below line 1, after one unchanged line
            format!("--- a/{path}\n+++ b/{path}\n@@ -7,{} +7,{} @@\n fn above() {{}}\n{body}", old + 1, new + 1)
        }
    };
    let base = checked(&hunk(&[], &plain)).expect("the constructs are a patch");
    if path == PATHS[0] {
        for (which, _) in picked {
            assert!(
                base.contains(&CONSTRUCTS[*which].1),
                "`{}` is not read as {}",
                plain.join("\n"),
                CONSTRUCTS[*which].1.label()
            );
        }
    }
    let with = checked(&hunk(literals, &commented)).expect("the constructs are a patch");
    let labels = |signals: &[EvidenceSignal]| -> Vec<&str> {
        signals.iter().map(|signal| signal.label()).collect()
    };
    if new_file {
        assert_eq!(
            labels(&with),
            labels(&base),
            "a comment between tokens or strings on earlier lines changed the signals of {path} (a new file):\n{}\n{}",
            literals.join("\n"),
            commented.join("\n")
        );
    } else {
        assert!(
            base.iter().all(|signal| with.contains(signal)),
            "a comment between tokens or strings on earlier lines lost a signal of {path} (a hunk below line 1): {:?} became {:?}:\n{}\n{}",
            labels(&base),
            labels(&with),
            literals.join("\n"),
            commented.join("\n")
        );
    }
}

/// Rule 5, from the text of an input: its first line picks the constructs and the place of the
/// comment in each, the lines up to a line `--` are the comment's text, the rest the strings'.
fn rule_five(path: &str, text: &str) {
    let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
    let picked: Vec<(usize, usize)> = first
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .take(6)
        .filter(|[letter, digit]| letter.is_ascii_lowercase() && digit.is_ascii_digit())
        .map(|[letter, digit]| {
            (
                usize::from(letter - b'a') % CONSTRUCTS.len(),
                usize::from(digit - b'0'),
            )
        })
        .collect();
    if picked.is_empty() {
        return;
    }
    let (comment, strings) = rest.split_once("\n--\n").unwrap_or((rest, ""));
    // Any text that does not end the comment: no `*` (so no `*/` and no `/*` in it), and no
    // `/` last, which with the closing `*` would open another.
    let comment: String = comment.chars().filter(|c| *c != '*').collect();
    let comment = comment.trim_end_matches('/');
    for new_file in [true, false] {
        // Complete literals of the strings' text, a few kinds: what would end one is taken
        // out.
        let body: String = strings
            .chars()
            .filter(|c| *c != '"' && *c != '\\')
            .collect();
        let literals = if strings.is_empty() {
            Vec::new()
        } else {
            vec![
                format!("const A: &str = \"{body}\";"),
                format!("const B: &[u8] = br#\"{body}\"#; const C: &str = r\"{body}\";"),
            ]
        };
        comments_and_earlier_strings_change_nothing(path, new_file, &picked, comment, &literals);
    }
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
    rule_five(path, &text);
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
