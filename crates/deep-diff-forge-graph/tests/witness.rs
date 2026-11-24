//! F7.c1 (contract design section 3.4): a substitute is shown to an agent only in its `agent`
//! sentence, and that sentence stands on a committed witness patch,
//! `fixtures/diagnostics/witness/<id>.patch`, on which this engine raises the label the
//! substitute is said to raise. Each spelling the sentence quotes in backticks stands verbatim in
//! a changed line of the witness on a side the raised label reads, so a sentence cannot name a
//! spelling its witness does not prove.

use deep_diff_forge_core::PatchLineKind;
use deep_diff_forge_graph::{Side, facts, rank, remedy, signal_named, vocabulary};
use deep_diff_forge_patch::parse;

const WITNESS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/diagnostics/witness"
);

/// The spans of `sentence` between backticks.
fn quoted(sentence: &str) -> Vec<&str> {
    sentence.split('`').skip(1).step_by(2).collect()
}

#[test]
fn every_agent_sentence_has_a_witness_that_raises_its_label() {
    let mut witnessed = Vec::new();
    for signal_facts in vocabulary() {
        for not in remedy(signal_facts.signal).not_accepted {
            let Some(agent) = not.agent else { continue };
            let id = not.id;
            let raised = not.raises.unwrap_or_else(|| {
                panic!("{id}: an agent sentence on a substitute that raises nothing")
            });
            let path = format!("{WITNESS}/{id}.patch");
            let witness = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{id}: no witness patch {path}: {e}"));
            let files = parse(&witness)
                .unwrap_or_else(|e| panic!("{id}: the witness does not parse: {e:?}"));
            let ranked = rank(&files);
            assert!(
                ranked
                    .iter()
                    .any(|f| f.signals.iter().any(|s| s.label() == raised)),
                "{id}: the witness does not raise {raised}: {:?}",
                ranked
                    .iter()
                    .map(|f| (
                        f.path.as_str(),
                        f.signals.iter().map(|s| s.label()).collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
            let sides = facts(signal_named(raised).unwrap()).sides();
            let changed: Vec<&str> = files
                .iter()
                .flat_map(|f| &f.patch_twin.hunks)
                .flat_map(|h| &h.lines)
                .filter(|l| match l.kind {
                    PatchLineKind::Added => sides.contains(&Side::New),
                    PatchLineKind::Removed => sides.contains(&Side::Old),
                    PatchLineKind::Context => false,
                })
                .map(|l| l.text.as_str())
                .collect();
            let spellings = quoted(agent);
            assert!(
                !spellings.is_empty(),
                "{id}: the agent sentence quotes no spelling"
            );
            for spelling in spellings {
                assert!(
                    changed.iter().any(|line| line.contains(spelling)),
                    "{id}: `{spelling}` is in no changed line its witness gives {raised} to read"
                );
            }
            witnessed.push(format!("{id}.patch"));
        }
    }
    // a witness without a sentence proves nothing an agent is told: none is kept
    let mut present: Vec<String> = std::fs::read_dir(WITNESS)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    present.sort_unstable();
    witnessed.sort_unstable();
    assert_eq!(
        present, witnessed,
        "a witness patch belongs to no agent sentence"
    );
}
