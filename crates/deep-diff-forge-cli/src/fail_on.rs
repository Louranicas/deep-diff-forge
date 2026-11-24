// Module index: docs/reference/commands.md#review-a-change
//! `--rank --fail-on LIST` (F7.i): a gate on the ranking. The list names classes (`security`,
//! `integrity`, `shape`) and labels of `schema signals`. The verdict reads each ranked file's
//! `signals[]` (never its evidence rows, so a label with no row, or one whose rows were cut, still
//! counts) and its `evidence_truncated` bit, whatever the list. It is decided before the document
//! is written and leaves through the one door to exit 7 ([`crate::refusal`]); the document is the
//! one the run without the flag prints.

use crate::hook::shown_path;
use deep_diff_forge_core::{display_safe, exit};
use deep_diff_forge_graph::{MAX_EVIDENCE_PER_FILE, RankedFile, SignalFacts, vocabulary};

/// The most files one refusal names, one line each; the rest are counted in one line.
const FILES_NAMED: usize = 32;

/// A `--fail-on` list whose every token is a class or a label.
pub(crate) struct FailOn {
    /// The tokens as given, in order: the reason line repeats them, and they hold no input text.
    tokens: Vec<String>,
    /// The labels the tokens select.
    labels: Vec<&'static str>,
}

impl FailOn {
    /// The list `list`, or the first token that is neither a class nor a label (an empty one too).
    fn parse(list: &str) -> Result<Self, String> {
        let known = vocabulary();
        let mut fail_on = Self {
            tokens: Vec::new(),
            labels: Vec::new(),
        };
        for token in list.split(',') {
            let selected: Vec<&'static str> = known
                .iter()
                .filter(|facts| facts.class.label() == token || facts.label() == token)
                .map(SignalFacts::label)
                .collect();
            if selected.is_empty() {
                return Err(token.to_owned());
            }
            fail_on.tokens.push(token.to_owned());
            fail_on.labels.extend(selected);
        }
        Ok(fail_on)
    }

    /// The list `list`, or exit 2 naming the first token that is neither a class nor a label.
    pub(crate) fn parse_or_exit(list: &str) -> Self {
        Self::parse(list).unwrap_or_else(|token| {
            let shown = if token.is_empty() {
                "\"\"".to_owned()
            } else {
                display_safe(&token).into_owned()
            };
            eprintln!(
                "error: --fail-on: unknown class or label: {shown} (deep-diff-forge schema signals lists them)"
            );
            std::process::exit(exit::USAGE);
        })
    }

    /// Record with the one door the refusals that hold for `ranked`, before its document is
    /// written: `fail_on_label` when a file's `signals[]` holds a selected label, and
    /// `fail_on_truncated` when a file's evidence list was cut. Each reason names its files.
    pub(crate) fn decide(&self, ranked: &[RankedFile]) {
        let carrying: Vec<(&str, String)> = ranked
            .iter()
            .filter_map(|file| {
                let labels: Vec<&str> = file
                    .signals
                    .iter()
                    .map(|signal| signal.label())
                    .filter(|label| self.labels.contains(label))
                    .collect();
                (!labels.is_empty()).then(|| (file.path.as_str(), labels.join(", ")))
            })
            .collect();
        let cut: Vec<(&str, String)> = ranked
            .iter()
            .filter(|file| file.evidence_truncated)
            .map(|file| (file.path.as_str(), "evidence_truncated".to_owned()))
            .collect();
        let mut reasons = Vec::new();
        if !carrying.is_empty() {
            let text = format!(
                "{} files carry a label of {}",
                carrying.len(),
                self.tokens.join(",")
            );
            reasons.push((exit::refusal("fail_on_label"), text, named(&carrying)));
        }
        if !cut.is_empty() {
            let text = format!(
                "the evidence of {} files was cut at {MAX_EVIDENCE_PER_FILE} rows",
                cut.len()
            );
            reasons.push((exit::refusal("fail_on_truncated"), text, named(&cut)));
        }
        crate::refusal::pending_with(&reasons);
    }
}

/// One line per file, its path as a message shows it (quoted, escaped, cut in the middle), then
/// what the reason found in it; past [`FILES_NAMED`], one line counts the rest.
fn named(files: &[(&str, String)]) -> Vec<String> {
    let mut lines: Vec<String> = files
        .iter()
        .take(FILES_NAMED)
        .map(|(path, found)| format!("{}: {found}", shown_path(path)))
        .collect();
    if files.len() > FILES_NAMED {
        lines.push(format!("and {} more files", files.len() - FILES_NAMED));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_a_class_or_a_label() {
        let fail_on = FailOn::parse("security,new_file,security").unwrap();
        assert!(fail_on.labels.contains(&"unsafe_added") && fail_on.labels.contains(&"new_file"));
        assert!(!fail_on.labels.contains(&"test_removed"));
        assert_eq!(fail_on.tokens.join(","), "security,new_file,security");
        for bad in [
            "",
            "security,",
            "Security",
            " shape",
            "evidence_truncated",
            "unsafe",
        ] {
            assert!(FailOn::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn past_the_named_files_the_rest_are_counted() {
        let files: Vec<(&str, String)> = (0..34).map(|_| ("a", "new_file".to_owned())).collect();
        let lines = named(&files);
        assert_eq!(lines.len(), FILES_NAMED + 1);
        assert_eq!(lines[0], "\"a\": new_file");
        assert_eq!(lines[FILES_NAMED], "and 2 more files");
    }
}
