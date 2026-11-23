//! Test only (ruling 1 of the 0.10.0 integration). Since 0.10.0 the parser refuses a hunk that
//! plain `git apply` places elsewhere than its header says: a hunk with no context past line 1,
//! and a hunk after one with no trailing context (`HunkPlacedElsewhere`). The readings of 0.9.0
//! that join such hunks stay under test on files built here without that rule: each hunk is
//! parsed alone with its file's headers, at line 1 (0 for an empty side), then given back its
//! own numbers, its lines numbered as the parser numbers them and its id in patch order. Only the
//! public API of the patch and core crates is used. Shared with the daemon's tests by `#[path]`.
//!
//! Every patch built here also goes through the parser, which must refuse it with the reason the
//! rule gives for its first such hunk, or read it as exactly the files built here.

use deep_diff_forge_core::{HunkId, PatchLineKind, ReviewFile};
use deep_diff_forge_patch::{PatchParseError, parse};

/// The parser's reasons (`PatchParseError::HunkPlacedElsewhere`).
pub(crate) const FOLLOWS: &str =
    "it follows a hunk with no trailing context, which Git places at the end";
pub(crate) const NO_CONTEXT: &str = "it has no context, so Git places it at the end of the file";

/// `-12,3` as (12, 3); `-12` as (12, 1).
fn side(range: &str) -> (u32, u32) {
    let (start, count) = range[1..].split_once(',').unwrap_or((&range[1..], "1"));
    (start.parse().unwrap(), count.parse().unwrap())
}

/// The files of `patch`, each hunk read alone and put back at its own numbers, after the parser
/// has refused `patch` for the reason the rule gives, or read it as exactly these files.
pub(crate) fn unplaced(patch: &str) -> Vec<ReviewFile> {
    let (built, why) = built(patch);
    match (parse(patch), why) {
        (Ok(files), None) => assert_eq!(files, built, "the parser reads {patch:?} otherwise"),
        (Err(PatchParseError::HunkPlacedElsewhere { reason, .. }), Some(why)) => {
            assert_eq!(reason, why, "{patch:?}");
        }
        (answer, why) => panic!("{patch:?}: the rule says {why:?}, the parser {answer:?}"),
    }
    built
}

/// The files, and the reason the parser's rule refuses the first misplaced hunk, if any.
fn built(patch: &str) -> (Vec<ReviewFile>, Option<&'static str>) {
    // Each file: its headers, and its hunks as (old (start, count), new start, header, body).
    type Hunk = ((u32, u32), u32, String, String);
    let mut sections: Vec<(String, Vec<Hunk>)> = Vec::new();
    let mut left = (0u32, 0u32);
    for line in patch.split_inclusive('\n') {
        if left != (0, 0) || line.starts_with('\\') {
            match line.as_bytes()[0] {
                b' ' => left = (left.0 - 1, left.1 - 1),
                b'-' => left.0 -= 1,
                b'+' => left.1 -= 1,
                _ => {}
            }
            let hunks = &mut sections.last_mut().unwrap().1;
            hunks.last_mut().unwrap().3.push_str(line);
        } else if let Some(rest) = line.strip_prefix("@@ ") {
            let mut words = rest.splitn(3, ' ');
            let (old, new) = (side(words.next().unwrap()), side(words.next().unwrap()));
            let tail = words.next().unwrap().strip_prefix("@@").unwrap();
            left = (old.1, new.1);
            let (o, n) = (u32::from(old.1 > 0), u32::from(new.1 > 0));
            let header = format!("@@ -{o},{} +{n},{} @@{tail}", old.1, new.1);
            let hunks = &mut sections.last_mut().unwrap().1;
            hunks.push((old, new.0, header, String::new()));
        } else {
            let starts = line.starts_with("diff --git ")
                || (line.starts_with("--- ") && sections.last().is_none_or(|s| !s.1.is_empty()));
            if starts || sections.is_empty() {
                sections.push((String::new(), Vec::new()));
            }
            sections.last_mut().unwrap().0.push_str(line);
        }
    }
    let (mut files, mut why, mut next) = (Vec::new(), None, 0);
    for (head, hunks) in sections {
        if hunks.is_empty() {
            files.extend(parse(&head).unwrap());
            continue;
        }
        let (mut file, mut open_end): (Option<ReviewFile>, Option<bool>) = (None, None);
        for ((old_start, old_count), new_start, header, body) in hunks {
            // The parser's rule (`close_hunk`), restated.
            let signs: Vec<u8> = body
                .lines()
                .map(|l| l.as_bytes()[0])
                .filter(|&b| b != b'\\')
                .collect();
            let leading = signs.iter().take_while(|&&b| b == b' ').count();
            let trailing = signs.iter().rev().take_while(|&&b| b == b' ').count();
            let at_zero = old_start == 0 && old_count == 0;
            if open_end.is_some_and(|zero| !(zero && at_zero)) {
                why = why.or(Some(FOLLOWS));
            } else if leading == 0 && trailing == 0 && old_start > 1 {
                why = why.or(Some(NO_CONTEXT));
            }
            open_end = (trailing == 0).then_some(at_zero);
            let mut alone = parse(&format!("{head}{header}{body}")).unwrap().remove(0);
            let mut hunk = alone.patch_twin.hunks.remove(0);
            hunk.id = HunkId(next);
            (hunk.old_start, hunk.new_start) = (Some(old_start), Some(new_start));
            next += 1;
            let (mut old, mut new) = (old_start, new_start);
            for line in &mut hunk.lines {
                let (o, n) = match line.kind {
                    PatchLineKind::Context => (true, true),
                    PatchLineKind::Removed => (true, false),
                    PatchLineKind::Added => (false, true),
                };
                line.old_line = o.then_some(old);
                line.new_line = n.then_some(new);
                (old, new) = (old + u32::from(o), new + u32::from(n));
            }
            file.get_or_insert(alone).patch_twin.hunks.push(hunk);
        }
        files.extend(file);
    }
    (files, why)
}

/// The rule restated here gives the parser's reasons, and a patch it does not refuse is read as
/// the files built here.
#[test]
fn the_rule_here_is_the_parsers() {
    let head = "--- a/f.rs\n+++ b/f.rs\n";
    for (hunks, why) in [
        ("@@ -5,0 +6,1 @@\n+x\n", Some(NO_CONTEXT)),
        (
            "@@ -1,1 +1,2 @@\n a\n+x\n@@ -9,1 +10,2 @@\n i\n+y\n",
            Some(FOLLOWS),
        ),
        (
            "@@ -1,2 +1,3 @@\n a\n+x\n b\n@@ -9,1 +10,2 @@\n i\n+y\n",
            None,
        ),
    ] {
        let patch = format!("{head}{hunks}");
        assert_eq!(built(&patch).1, why, "{patch}");
        let files = unplaced(&patch);
        assert_eq!(
            files[0].patch_twin.hunks.len(),
            2.min(hunks.matches("@@ -").count())
        );
    }
}
