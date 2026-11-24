//! The reason classes of the hooks: one code for each cause a stop, an advice hook or
//! `agent check` can give an outcome, as the project's ledger records it (the design's
//! F7-W3a and F7-W3b). A code says why, never whether the stop holds: whether a stop holds is
//! decided where it is decided without the class (`hook::blocks_now`,
//! `hook::decide_stop_trees`), and the column "holds" below says what those decide.
//!
//! The rule of the codes (stated in `docs/reference/agent-hooks.md`, "Reason classes", which
//! holds this table word for word): a code's meaning never changes; codes are added without a
//! version change; a consumer treats an unknown code as not a PASS; removing one is a new
//! ledger version. The three codes written before this table (`work.unreadable`,
//! `work.skip_worktree`, `hold.version_gap`, and `fail.stands`, `fail.new`,
//! `fail.unsaved_continuing`, `hold.rule`, `pass`) keep their spelling: ledgers on disk hold
//! them.

/// Why an outcome is what it is. Every variant has its row in [`ReasonClass::row`], which is
/// matched exhaustively: a new cause cannot be built without its code, its printed phrase,
/// whether it holds and its meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum ReasonClass {
    Pass,
    FailNew,
    FailStands,
    FailUnsavedContinuing,
    HoldRule,
    HoldClaimsUnverified,
    HoldNotes,
    HoldVersionGap,
    WorkUnreadable,
    WorkSkipWorktree,
    ChainJudgeMissing,
    ChainJudgeInside,
    ChainPolicyMissing,
    ChainInputUnwritable,
    ChainNoRanker,
    ChainRankersUnlisted,
    ChainRankerUnsealed,
    ChainJudgeRefused,
    ChainJudgeEnvironment,
    ChainJudgeExit,
    ChainNotAVerdict,
    ChainPatchTooLarge,
    TimeJudgeStopped,
    TimeNoTimeLeft,
    TimeNotAsked,
    TimeGit,
    StateDamaged,
    StateOlder,
    StateNewer,
    StateNoDirectory,
    StateInsideTree,
    StateLockBusy,
    StateListsDamaged,
    StateNoBaseline,
    StateNotApart,
    PlaceGitFailed,
    PlaceUnpaired,
    PlaceDirectoryGone,
    PlaceClientDirDiffers,
    PlaceReplaced,
    PlaceMoved,
    PlaceOrphan,
    PlaceBeforeFirstCommit,
    PlaceNoCommitYet,
    PlaceHeadUnreadable,
    PlaceBaseGone,
    PlaceUnmerged,
    PlaceShallowParent,
    PlaceScratch,
    InputRefused,
}

impl ReasonClass {
    /// Every class, in the order of the table.
    pub(crate) const ALL: [Self; 50] = [
        Self::Pass,
        Self::FailNew,
        Self::FailStands,
        Self::FailUnsavedContinuing,
        Self::HoldRule,
        Self::HoldClaimsUnverified,
        Self::HoldNotes,
        Self::HoldVersionGap,
        Self::WorkUnreadable,
        Self::WorkSkipWorktree,
        Self::ChainJudgeMissing,
        Self::ChainJudgeInside,
        Self::ChainPolicyMissing,
        Self::ChainInputUnwritable,
        Self::ChainNoRanker,
        Self::ChainRankersUnlisted,
        Self::ChainRankerUnsealed,
        Self::ChainJudgeRefused,
        Self::ChainJudgeEnvironment,
        Self::ChainJudgeExit,
        Self::ChainNotAVerdict,
        Self::ChainPatchTooLarge,
        Self::TimeJudgeStopped,
        Self::TimeNoTimeLeft,
        Self::TimeNotAsked,
        Self::TimeGit,
        Self::StateDamaged,
        Self::StateOlder,
        Self::StateNewer,
        Self::StateNoDirectory,
        Self::StateInsideTree,
        Self::StateLockBusy,
        Self::StateListsDamaged,
        Self::StateNoBaseline,
        Self::StateNotApart,
        Self::PlaceGitFailed,
        Self::PlaceUnpaired,
        Self::PlaceDirectoryGone,
        Self::PlaceClientDirDiffers,
        Self::PlaceReplaced,
        Self::PlaceMoved,
        Self::PlaceOrphan,
        Self::PlaceBeforeFirstCommit,
        Self::PlaceNoCommitYet,
        Self::PlaceHeadUnreadable,
        Self::PlaceBaseGone,
        Self::PlaceUnmerged,
        Self::PlaceShallowParent,
        Self::PlaceScratch,
        Self::InputRefused,
    ];

    /// Whether the cause lies in the working tree (`work.*`): such a unit is said as
    /// UNJUDGED, and recorded so, when it does not hold the stop; any other unit that holds
    /// once is said as HOLD. This chooses words and the ledger's outcome, never whether a stop
    /// holds.
    pub(crate) fn is_work(self) -> bool {
        self.as_str().starts_with("work.")
    }

    /// The class whose code is exactly `code`, if this build has one.
    pub(crate) fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|class| class.as_str() == code)
    }

    /// What the class means, in one sentence ([`ReasonClass::row`]): what later stops say of a
    /// commit from before, whose own words are not kept.
    pub(crate) fn meaning(self) -> &'static str {
        self.row().3
    }

    /// The code as the ledger writes it.
    pub(crate) fn as_str(self) -> &'static str {
        self.row().0
    }

    /// The class's row: its code, what the stop prints for it (one phrase), whether it holds
    /// a stop, and what it means. `docs/reference/agent-hooks.md` holds the first three, and
    /// `tests/contract.rs` reads them from between the two marks below.
    #[expect(
        clippy::too_many_lines,
        reason = "the table of every class, one row each, which the document holds"
    )]
    pub(crate) fn row(self) -> (&'static str, &'static str, &'static str, &'static str) {
        // TABLE BEGIN
        match self {
            Self::Pass => (
                "pass",
                "nothing, or a note",
                "never",
                "the judge passed the unit",
            ),
            Self::FailNew => (
                "fail.new",
                "block",
                "once for each failing line",
                "the judge failed the unit with a line that has not blocked before",
            ),
            Self::FailStands => (
                "fail.stands",
                "FAIL STANDS",
                "never",
                "the judge failed the unit and every failing line has blocked before",
            ),
            Self::FailUnsavedContinuing => (
                "fail.unsaved_continuing",
                "FAIL STANDS (the hook cannot keep its own record)",
                "never",
                "a failure at a stop that follows a block while the state cannot be saved",
            ),
            Self::HoldRule => (
                "hold.rule",
                "HOLD",
                "never",
                "the judge holds a rule of the policy for a person",
            ),
            Self::HoldClaimsUnverified => (
                "hold.claims_unverified",
                "HOLD",
                "never",
                "the judge holds for claims it cannot verify, with no rule held",
            ),
            Self::HoldNotes => (
                "hold.notes",
                "HOLD",
                "never",
                "the judge holds with no rule held, for its notes",
            ),
            Self::HoldVersionGap => (
                "hold.version_gap",
                "block once, then HOLD",
                "once for each cause, at most three stops",
                "the judge holds only for a label it does not know",
            ),
            Self::WorkUnreadable => (
                "work.unreadable",
                "block once, then UNJUDGED",
                "once for each cause, at most three stops",
                "a file of the working tree this account cannot read",
            ),
            Self::WorkSkipWorktree => (
                "work.skip_worktree",
                "block once, then UNJUDGED",
                "once for each cause, at most three stops",
                "paths of the working tree marked skip-worktree",
            ),
            Self::ChainJudgeMissing => (
                "chain.judge_missing",
                "UNJUDGED",
                "never",
                "no judge was found",
            ),
            Self::ChainJudgeInside => (
                "chain.judge_inside",
                "UNJUDGED",
                "never",
                "the judge is inside the repository it would judge",
            ),
            Self::ChainPolicyMissing => (
                "chain.policy_missing",
                "UNJUDGED",
                "never",
                "the base commit has no policy file",
            ),
            Self::ChainInputUnwritable => (
                "chain.input_unwritable",
                "UNJUDGED",
                "never",
                "the judge's input could not be written",
            ),
            Self::ChainNoRanker => (
                "chain.no_ranker",
                "UNJUDGED",
                "never",
                "this binary's own path could not be found, so no ranker was offered",
            ),
            Self::ChainRankersUnlisted => (
                "chain.rankers_unlisted",
                "UNJUDGED",
                "never",
                "the rankers could not be listed and hashed",
            ),
            Self::ChainRankerUnsealed => (
                "chain.ranker_unsealed",
                "UNJUDGED",
                "never",
                "a ranker could not be hashed, so no verdict could be sealed to it",
            ),
            Self::ChainJudgeRefused => (
                "chain.judge_refused",
                "UNJUDGED",
                "never",
                "the judge refused the policy, the ranking or the ranker (exit 4)",
            ),
            Self::ChainJudgeEnvironment => (
                "chain.judge_environment",
                "UNJUDGED",
                "never",
                "the judge could not read its inputs or its environment (exit 3)",
            ),
            Self::ChainJudgeExit => (
                "chain.judge_exit",
                "UNJUDGED",
                "never",
                "the judge exited with a code that is not a verdict",
            ),
            Self::ChainNotAVerdict => (
                "chain.not_a_verdict",
                "UNJUDGED",
                "never",
                "a pass or a hold whose document is not a verdict on these inputs",
            ),
            Self::ChainPatchTooLarge => (
                "chain.patch_too_large",
                "UNJUDGED",
                "never",
                "a commit's patch is over the size the patch maker makes",
            ),
            Self::TimeJudgeStopped => (
                "time.judge_stopped",
                "UNJUDGED",
                "never",
                "the judge was stopped before it gave a verdict",
            ),
            Self::TimeNoTimeLeft => (
                "time.no_time_left",
                "UNJUDGED",
                "never",
                "the stop had no time left to run the judge",
            ),
            Self::TimeNotAsked => (
                "time.not_asked",
                "UNJUDGED",
                "never",
                "commits not asked about at this stop for lack of time",
            ),
            Self::TimeGit => (
                "time.git",
                "UNJUDGED",
                "never",
                "Git did not finish in the time a hook has",
            ),
            Self::StateDamaged => (
                "state.damaged",
                "the tree judged without the state",
                "never",
                "the session's state file cannot be read as a state",
            ),
            Self::StateOlder => (
                "state.older",
                "the tree judged without the state",
                "never",
                "the session's state is of a schema older than the last release's",
            ),
            Self::StateNewer => (
                "state.newer",
                "the tree judged without the state",
                "never",
                "the session's state is of a newer schema than this build reads",
            ),
            Self::StateNoDirectory => (
                "state.no_directory",
                "the tree judged without the state",
                "never",
                "no state directory can be used",
            ),
            Self::StateInsideTree => (
                "state.inside_tree",
                "UNJUDGED",
                "never",
                "the state directory lies inside the project's working tree",
            ),
            Self::StateLockBusy => (
                "state.lock_busy",
                "the tree judged without the state",
                "never",
                "another hook of the session holds its lock",
            ),
            Self::StateListsDamaged => (
                "state.lists_damaged",
                "the tree judged without the state",
                "never",
                "a list beside the state is not the one the state names",
            ),
            Self::StateNoBaseline => (
                "state.no_baseline",
                "UNJUDGED",
                "never",
                "the commits there when the session began could not be listed",
            ),
            Self::StateNotApart => (
                "state.not_apart",
                "UNJUDGED",
                "never",
                "which of a staged failure is the staged change's own cannot be told",
            ),
            Self::PlaceGitFailed => (
                "place.git_failed",
                "UNJUDGED",
                "never",
                "Git failed or answered in a form that cannot be read",
            ),
            Self::PlaceUnpaired => (
                "place.unpaired",
                "UNJUDGED",
                "never",
                "which working tree a directory is in could not be found out",
            ),
            Self::PlaceDirectoryGone => (
                "place.directory_gone",
                "UNJUDGED",
                "never",
                "the session's directory does not exist and no project is recorded",
            ),
            Self::PlaceClientDirDiffers => (
                "place.client_dir_differs",
                "UNJUDGED",
                "never",
                "the client names another project than the session began in",
            ),
            Self::PlaceReplaced => (
                "place.replaced",
                "UNJUDGED",
                "never",
                "another repository stands at the project's path",
            ),
            Self::PlaceMoved => (
                "place.moved",
                "UNJUDGED",
                "never",
                "the project is no longer a Git working tree",
            ),
            Self::PlaceOrphan => (
                "place.orphan",
                "UNJUDGED",
                "never",
                "the session began on a branch with no commit in a repository that has commits",
            ),
            Self::PlaceBeforeFirstCommit => (
                "place.before_first_commit",
                "UNJUDGED",
                "never",
                "the session began before the repository's first commit",
            ),
            Self::PlaceNoCommitYet => (
                "place.no_commit_yet",
                "a note",
                "never",
                "the repository has no commit yet and files that were not judged",
            ),
            Self::PlaceHeadUnreadable => (
                "place.head_unreadable",
                "UNJUDGED",
                "never",
                "the commit HEAD names could not be read",
            ),
            Self::PlaceBaseGone => (
                "place.base_gone",
                "UNJUDGED",
                "never",
                "the commit a change is measured from can no longer be read",
            ),
            Self::PlaceUnmerged => (
                "place.unmerged",
                "UNJUDGED",
                "never",
                "the index has unmerged paths",
            ),
            Self::PlaceShallowParent => (
                "place.shallow_parent",
                "UNJUDGED",
                "never",
                "a shallow clone does not have a commit's parent",
            ),
            Self::PlaceScratch => (
                "place.scratch",
                "UNJUDGED, did not run",
                "never",
                "the hook's scratch directory could not be made",
            ),
            Self::InputRefused => (
                "input.refused",
                "UNJUDGED, did not run",
                "never",
                "the hook's input or command line could not be read",
            ),
        }
        // TABLE END
    }
}

/// The class of a reason a session's file of judged commits keeps in words (an answer `n…:`
/// or `x:` from before), read from the words this build writes there, in front first: the
/// words the hook puts around a reason (a commit asked about in its turn) are taken off, and
/// then each reason's own opening words name its class. Words this build does not write are
/// [`ReasonClass::ChainJudgeExit`]: no verdict, for a reason not known here. It labels a
/// record and decides nothing; the judged file keeps codes of its own from schema v12 on.
pub(crate) fn class_of_words(words: &str) -> ReasonClass {
    const IN_TURN: &str =
        "; it is asked about again in its turn, one such commit at each stop; the last reason: ";
    let words = match words.split_once(IN_TURN) {
        Some((head, reason)) if head.starts_with("no verdict in ") => reason,
        _ => words,
    };
    let git = |rest: &str| {
        if rest.contains("did not finish in the time a hook has") {
            ReasonClass::TimeGit
        } else if rest.contains("Git output exceeds the") {
            ReasonClass::ChainPatchTooLarge
        } else {
            ReasonClass::PlaceGitFailed
        }
    };
    let opening: [(&str, ReasonClass); 14] = [
        (
            "the judge refused the policy, the ranking or the ranker (exit 4)",
            ReasonClass::ChainJudgeRefused,
        ),
        (
            "the judge could not read its inputs or its environment (exit 3)",
            ReasonClass::ChainJudgeEnvironment,
        ),
        (
            "the judge was stopped before it gave a verdict",
            ReasonClass::TimeJudgeStopped,
        ),
        (
            "the stop had no time left to run the judge",
            ReasonClass::TimeNoTimeLeft,
        ),
        (
            "the judge's input could not be written",
            ReasonClass::ChainInputUnwritable,
        ),
        (
            "this binary's own path could not be found",
            ReasonClass::ChainNoRanker,
        ),
        ("forge-warden was not found", ReasonClass::ChainJudgeMissing),
        (
            "the base commit has no policy file",
            ReasonClass::ChainPolicyMissing,
        ),
        (
            "the rankers could not be listed and hashed",
            ReasonClass::ChainRankersUnlisted,
        ),
        ("they were not judged yet", ReasonClass::TimeNotAsked),
        ("this clone is shallow", ReasonClass::PlaceShallowParent),
        (
            "not asked at this stop for lack of time",
            ReasonClass::TimeNotAsked,
        ),
        ("the judge ", ReasonClass::ChainJudgeExit),
        ("the ranker ", ReasonClass::ChainRankerUnsealed),
    ];
    if let Some(rest) = words
        .strip_prefix("the commit could not be read from Git: ")
        .or_else(|| words.strip_prefix("the policy could not be read from Git: "))
    {
        return git(rest);
    }
    if words.starts_with("the judge ") && words.contains(" is inside the repository it would judge")
    {
        return ReasonClass::ChainJudgeInside;
    }
    if words.starts_with("the judge exited ") && words.contains(") but that is not a verdict") {
        return ReasonClass::ChainNotAVerdict;
    }
    opening
        .iter()
        .find(|(start, _)| words.starts_with(start))
        .map_or(ReasonClass::ChainJudgeExit, |(_, class)| *class)
}

/// A reason with its class: what a hook says, and why, as the ledger records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Why {
    pub(crate) class: ReasonClass,
    pub(crate) text: String,
}

impl Why {
    pub(crate) fn new(class: ReasonClass, text: impl Into<String>) -> Self {
        Self {
            class,
            text: text.into(),
        }
    }
}

impl std::fmt::Display for Why {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(&self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every class is in [`ReasonClass::ALL`] once, every code once, each code in one of the
    /// design's families and written as `family.name` in lowercase (or `pass`).
    #[test]
    fn every_class_has_one_row_and_one_code() {
        let codes: std::collections::BTreeSet<&str> = ReasonClass::ALL
            .iter()
            .map(|class| class.as_str())
            .collect();
        assert_eq!(codes.len(), ReasonClass::ALL.len());
        let classes: std::collections::BTreeSet<ReasonClass> =
            ReasonClass::ALL.iter().copied().collect();
        assert_eq!(classes.len(), ReasonClass::ALL.len());
        for class in ReasonClass::ALL {
            let code = class.as_str();
            let family = code.split('.').next().unwrap_or_default();
            assert!(
                code == "pass"
                    || [
                        "fail", "hold", "work", "chain", "time", "state", "place", "input"
                    ]
                    .contains(&family),
                "{code}"
            );
            assert!(
                code.bytes()
                    .all(|b| b.is_ascii_lowercase() || b == b'.' || b == b'_'),
                "{code}"
            );
            let (_, printed, holds, meaning) = class.row();
            assert!(!printed.is_empty() && !holds.is_empty() && !meaning.is_empty());
            // a cause in the work, and only there, can hold (N91: by the evidence F12 has)
            let holds_by_rule = holds.starts_with("once for each cause");
            assert_eq!(
                holds_by_rule,
                family == "work" || code == "hold.version_gap",
                "{code}"
            );
        }
        // the codes already in ledgers on disk keep their spelling
        for (class, code) in [
            (ReasonClass::WorkUnreadable, "work.unreadable"),
            (ReasonClass::WorkSkipWorktree, "work.skip_worktree"),
            (ReasonClass::HoldVersionGap, "hold.version_gap"),
            (ReasonClass::FailStands, "fail.stands"),
            (ReasonClass::FailNew, "fail.new"),
            (
                ReasonClass::FailUnsavedContinuing,
                "fail.unsaved_continuing",
            ),
            (ReasonClass::HoldRule, "hold.rule"),
            (ReasonClass::Pass, "pass"),
        ] {
            assert_eq!(class.as_str(), code);
        }
    }
}
