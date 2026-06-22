//! The command palette: in-cockpit access to the whole engine.
//!
//! Each [`Command`] runs a real deep-diff-forge capability against the loaded
//! review — graph ranking, dimensional clustering, the `review.v0` JSON
//! document, the agent-note inventory, the L9 learning store, and the maturity
//! ladder — and returns a [`CommandOutput`] (a title plus text lines) for the
//! result panel. Everything runs in-process and read-only; no files are written
//! and patch truth is never mutated.

use crate::state::ReviewApp;
use deep_diff_forge_agent::{anchor_path, grounding_of, source_of};
use deep_diff_forge_cluster::{dimension_label, join_label, parallelism_label, run_risk_cluster};
use deep_diff_forge_core::{JoinPolicy, MaturityLevel, Parallelism};
use deep_diff_forge_graph::change_counts;
use deep_diff_forge_learning::{LearningReport, store};

/// The maturity level this build declares (kept in step with the CLI).
const CURRENT_MATURITY: MaturityLevel = MaturityLevel::L9;

/// The maturity ladder, lowest to highest.
const LADDER: [MaturityLevel; 10] = [
    MaturityLevel::L0,
    MaturityLevel::L1,
    MaturityLevel::L2,
    MaturityLevel::L3,
    MaturityLevel::L4,
    MaturityLevel::L5,
    MaturityLevel::L6,
    MaturityLevel::L7,
    MaturityLevel::L8,
    MaturityLevel::L9,
];

/// A capability runnable from the cockpit palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Ranked file list with scores and risk signals.
    Rank,
    /// Dimensional cluster run summary.
    Cluster,
    /// Review statistics (files, ±lines, hunks, notes, by status).
    Summary,
    /// The full engine-annotation inventory.
    Notes,
    /// The `deep-diff-forge.review.v0` JSON document.
    ReviewJson,
    /// The L9 learning store's per-strategy scores.
    Learning,
    /// The maturity ladder and the current level.
    Maturity,
}

impl Command {
    /// Every command, in palette order.
    #[must_use]
    pub fn all() -> [Command; 7] {
        [
            Self::Rank,
            Self::Cluster,
            Self::Summary,
            Self::Notes,
            Self::ReviewJson,
            Self::Learning,
            Self::Maturity,
        ]
    }

    /// Short palette label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Rank => "rank",
            Self::Cluster => "cluster",
            Self::Summary => "summary",
            Self::Notes => "notes",
            Self::ReviewJson => "review json",
            Self::Learning => "learning",
            Self::Maturity => "maturity",
        }
    }

    /// One-line description shown next to the label.
    #[must_use]
    pub fn hint(self) -> &'static str {
        match self {
            Self::Rank => "files by review priority, with risk signals",
            Self::Cluster => "parallel dimensional lanes + receipt",
            Self::Summary => "counts: files, ± lines, hunks, notes",
            Self::Notes => "every engine annotation + grounding",
            Self::ReviewJson => "the review.v0 machine document",
            Self::Learning => "L9 strategy scores from the local store",
            Self::Maturity => "the L0–L9 ladder and current level",
        }
    }
}

/// The text result of running a command: a title and body lines.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandOutput {
    /// Panel title.
    pub title: String,
    /// Panel body, one entry per line. Rendered terminal-safe by the caller.
    pub lines: Vec<String>,
}

/// Run `command` against the loaded review.
#[must_use]
pub fn run(command: Command, app: &ReviewApp) -> CommandOutput {
    match command {
        Command::Rank => rank_output(app),
        Command::Cluster => cluster_output(app),
        Command::Summary => summary_output(app),
        Command::Notes => notes_output(app),
        Command::ReviewJson => review_json_output(app),
        Command::Learning => learning_output(),
        Command::Maturity => maturity_output(),
    }
}

fn signals_of(rf: &deep_diff_forge_graph::RankedFile) -> String {
    rf.signals
        .iter()
        .map(|s| s.label())
        .collect::<Vec<_>>()
        .join(",")
}

fn rank_output(app: &ReviewApp) -> CommandOutput {
    let mut lines = Vec::with_capacity(app.file_count() + 1);
    lines.push("score  status     ±lines  path  [signals]".to_string());
    for (i, rf) in app.files().iter().enumerate() {
        let (add, del) = change_counts(&app.content()[i]);
        lines.push(format!(
            "{:>5}  {:<9}  +{:<4} -{:<4} {}  [{}]",
            rf.score,
            rf.status.label(),
            add,
            del,
            rf.path,
            signals_of(rf),
        ));
    }
    CommandOutput {
        title: format!("rank · {} file(s) by review priority", app.file_count()),
        lines,
    }
}

fn cluster_output(app: &ReviewApp) -> CommandOutput {
    let run = run_risk_cluster(
        app.content(),
        Parallelism::Auto,
        JoinPolicy::RankedReviewOrder,
    );
    let dims = run
        .receipt
        .dimensions
        .iter()
        .map(|d| dimension_label(*d))
        .collect::<Vec<_>>()
        .join(" ");
    let mut lines = vec![
        format!("dimensions:  {dims}"),
        format!(
            "parallelism: {} · workers: {} · join: {}",
            parallelism_label(run.receipt.parallelism),
            run.receipt.worker_count,
            join_label(run.receipt.join_policy),
        ),
        format!("files:       {}", run.receipt.file_count),
        String::new(),
    ];
    for rf in &run.ranked {
        lines.push(format!(
            "{:>5}  {}  [{}]",
            rf.score,
            rf.path,
            signals_of(rf)
        ));
    }
    CommandOutput {
        title: format!("cluster · {} dimension(s)", run.receipt.dimensions.len()),
        lines,
    }
}

fn summary_output(app: &ReviewApp) -> CommandOutput {
    let mut add_total = 0;
    let mut del_total = 0;
    let mut hunks = 0;
    for file in app.content() {
        let (a, d) = change_counts(file);
        add_total += a;
        del_total += d;
        hunks += file.patch_twin.hunks.len();
    }
    let lines = vec![
        format!("files:    {}", app.file_count()),
        format!("additions: +{add_total}"),
        format!("deletions: -{del_total}"),
        format!("hunks:    {hunks}"),
        format!("notes:    {}", app.annotations().len()),
    ];
    CommandOutput {
        title: "summary · review statistics".to_string(),
        lines,
    }
}

fn notes_output(app: &ReviewApp) -> CommandOutput {
    let mut lines = Vec::new();
    for note in app.annotations() {
        lines.push(format!(
            "{} · {} · {}",
            source_of(note).label(),
            grounding_of(note).label(),
            anchor_path(&note.anchor),
        ));
        lines.push(format!("    {}", note.body));
    }
    if lines.is_empty() {
        lines.push("no engine notes for this review".to_string());
    }
    CommandOutput {
        title: format!("notes · {} annotation(s)", app.annotations().len()),
        lines,
    }
}

fn review_json_output(app: &ReviewApp) -> CommandOutput {
    let json = deep_diff_forge_patch::to_json(app.content());
    CommandOutput {
        title: "review json · deep-diff-forge.review.v0".to_string(),
        lines: json.lines().map(str::to_string).collect(),
    }
}

fn learning_output() -> CommandOutput {
    let title = "learning · L9 strategy scores".to_string();
    let report = match store::learning_dir().and_then(|dir| LearningReport::from_dir(&dir)) {
        Ok(report) => report,
        Err(err) => {
            return CommandOutput {
                title,
                lines: vec![
                    format!("no readable learning store: {err}"),
                    "feed it with `deep-diff-forge learn record --stdin`.".to_string(),
                ],
            };
        }
    };
    let mut lines = vec![
        format!("receipts: {}", report.total_receipts),
        format!(
            "trusted:  {}",
            report
                .trusted_default
                .map_or("none (insufficient evidence)", |s| s.label())
        ),
        String::new(),
        "strategy  samples  accept  helpful  fallback  revisit".to_string(),
    ];
    for s in &report.scores {
        lines.push(format!(
            "{:<8}  {:>7}  {:>6.2}  {:>7.2}  {:>8.2}  {:>7.2}",
            s.strategy.label(),
            s.samples,
            s.acceptance_rate,
            s.helpful_rate,
            s.fallback_rate,
            s.revisit_rate,
        ));
    }
    CommandOutput { title, lines }
}

fn maturity_output() -> CommandOutput {
    let mut lines = Vec::with_capacity(LADDER.len());
    for level in LADDER {
        let marker = if level == CURRENT_MATURITY {
            "▶"
        } else {
            " "
        };
        lines.push(format!("{marker} {:<3} {}", level.as_str(), level.name()));
    }
    CommandOutput {
        title: format!(
            "maturity · current {} ({})",
            CURRENT_MATURITY.as_str(),
            CURRENT_MATURITY.name()
        ),
        lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_patch::parse;

    const DIFF: &str = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@\n keep\n-old\n+new\n";

    fn app() -> ReviewApp {
        let files = parse(DIFF).unwrap();
        let notes = crate::notes::engine_annotations(&files, &deep_diff_forge_graph::rank(&files));
        ReviewApp::from_review_with_annotations(&files, notes)
    }

    fn body(out: &CommandOutput) -> String {
        out.lines.join("\n")
    }

    #[test]
    fn all_commands_have_label_and_hint() {
        for c in Command::all() {
            assert!(!c.label().is_empty());
            assert!(!c.hint().is_empty());
        }
        assert_eq!(Command::all().len(), 7);
    }

    #[test]
    fn rank_lists_the_file_with_score() {
        let out = run(Command::Rank, &app());
        assert!(out.title.contains("rank"));
        assert!(body(&out).contains("src/lib.rs"));
        assert!(body(&out).contains("public_api_surface"));
    }

    #[test]
    fn cluster_reports_dimensions_and_workers() {
        let out = run(Command::Cluster, &app());
        assert!(body(&out).contains("dimensions:"));
        assert!(body(&out).contains("workers:"));
        assert!(body(&out).contains("src/lib.rs"));
    }

    #[test]
    fn summary_counts_files_and_lines() {
        let out = run(Command::Summary, &app());
        let b = body(&out);
        assert!(b.contains("files:    1"));
        assert!(b.contains("+1"));
        assert!(b.contains("-1"));
    }

    #[test]
    fn notes_lists_engine_annotations() {
        let out = run(Command::Notes, &app());
        assert!(body(&out).contains("system"));
        assert!(body(&out).contains("grounded"));
        assert!(body(&out).contains("Public API"));
    }

    #[test]
    fn notes_empty_review_is_graceful() {
        let files = parse("--- a/x.txt\n+++ b/x.txt\n@@ -1,1 +1,1 @@\n-a\n+b\n").unwrap();
        let plain = ReviewApp::from_review(&files);
        let out = run(Command::Notes, &plain);
        assert!(body(&out).contains("no engine notes"));
    }

    #[test]
    fn review_json_is_the_v0_document() {
        let out = run(Command::ReviewJson, &app());
        let b = body(&out);
        assert!(b.contains("deep-diff-forge.review.v0"));
        assert!(b.contains("\"path\""));
    }

    #[test]
    fn learning_is_fail_soft() {
        // Whether or not a store exists, this must never panic and must title.
        let out = run(Command::Learning, &app());
        assert!(out.title.contains("learning"));
        assert!(!out.lines.is_empty());
    }

    #[test]
    fn maturity_shows_ladder_and_marks_current() {
        let out = run(Command::Maturity, &app());
        let b = body(&out);
        assert!(b.contains("L0"));
        assert!(b.contains("L9"));
        assert!(b.contains('▶'), "current level should be marked");
        assert!(out.title.contains("L9"));
    }

    #[test]
    fn every_command_runs_without_panicking() {
        let a = app();
        for c in Command::all() {
            let out = run(c, &a);
            assert!(!out.title.is_empty());
        }
    }
}
