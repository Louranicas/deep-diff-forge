//! Release publication vocabulary.
//!
//! Each publication target carries an independent state (the gap analysis's
//! "every release target has an independent state" policy), so a partial
//! release — GitHub done, crates.io credential-blocked — is reported honestly
//! rather than as a single pass/fail.

/// State of one publication target, independent of the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetState {
    /// No remote/credentials configured for this target.
    NotConfigured,
    /// Configured but blocked (e.g. missing credentials).
    Blocked,
    /// Intentionally skipped this release.
    Skipped,
    /// Successfully published.
    Published,
    /// Attempted and failed.
    Failed,
}

impl TargetState {
    /// Stable kebab-case label for machine output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotConfigured => "not-configured",
            Self::Blocked => "blocked",
            Self::Skipped => "skipped",
            Self::Published => "published",
            Self::Failed => "failed",
        }
    }

    /// Whether this state represents a completed publication.
    #[must_use]
    pub fn is_published(self) -> bool {
        matches!(self, Self::Published)
    }
}

/// One named publication target and its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseTarget {
    /// Target name (e.g. `"crates.io"`).
    pub name: String,
    /// Target state.
    pub state: TargetState,
}

impl ReleaseTarget {
    /// Construct a target.
    #[must_use]
    pub fn new(name: impl Into<String>, state: TargetState) -> Self {
        Self {
            name: name.into(),
            state,
        }
    }
}

/// A release plan: a version and the per-target publication posture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleasePlan {
    /// Semantic version being released.
    pub version: String,
    /// Publication targets in declaration order.
    pub targets: Vec<ReleaseTarget>,
}

impl ReleasePlan {
    /// Start a plan for `version` with no targets.
    #[must_use]
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            targets: Vec::new(),
        }
    }

    /// Append a target (builder style).
    #[must_use]
    pub fn with_target(mut self, name: impl Into<String>, state: TargetState) -> Self {
        self.targets.push(ReleaseTarget::new(name, state));
        self
    }

    /// Look up a target's state by name.
    #[must_use]
    pub fn target_state(&self, name: &str) -> Option<TargetState> {
        self.targets
            .iter()
            .find(|t| t.name == name)
            .map(|t| t.state)
    }

    /// Whether every target is published.
    #[must_use]
    pub fn fully_published(&self) -> bool {
        !self.targets.is_empty() && self.targets.iter().all(|t| t.state.is_published())
    }

    /// Names of targets that are not yet published.
    #[must_use]
    pub fn pending(&self) -> Vec<&str> {
        self.targets
            .iter()
            .filter(|t| !t.state.is_published())
            .map(|t| t.name.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_state_labels_are_kebab_case() {
        assert_eq!(TargetState::NotConfigured.as_str(), "not-configured");
        assert_eq!(TargetState::Blocked.as_str(), "blocked");
        assert_eq!(TargetState::Skipped.as_str(), "skipped");
        assert_eq!(TargetState::Published.as_str(), "published");
        assert_eq!(TargetState::Failed.as_str(), "failed");
    }

    #[test]
    fn only_published_is_published() {
        assert!(TargetState::Published.is_published());
        assert!(!TargetState::Blocked.is_published());
        assert!(!TargetState::NotConfigured.is_published());
        assert!(!TargetState::Failed.is_published());
        assert!(!TargetState::Skipped.is_published());
    }

    #[test]
    fn target_constructs_from_str() {
        let t = ReleaseTarget::new("crates.io", TargetState::Blocked);
        assert_eq!(t.name, "crates.io");
        assert_eq!(t.state, TargetState::Blocked);
    }

    #[test]
    fn plan_starts_empty() {
        let p = ReleasePlan::new("0.1.0");
        assert_eq!(p.version, "0.1.0");
        assert!(p.targets.is_empty());
    }

    #[test]
    fn with_target_appends_in_order() {
        let p = ReleasePlan::new("0.1.0")
            .with_target("github", TargetState::Published)
            .with_target("crates.io", TargetState::Blocked);
        assert_eq!(p.targets.len(), 2);
        assert_eq!(p.targets[0].name, "github");
        assert_eq!(p.targets[1].name, "crates.io");
    }

    #[test]
    fn target_state_lookup() {
        let p = ReleasePlan::new("0.1.0").with_target("gitlab", TargetState::Published);
        assert_eq!(p.target_state("gitlab"), Some(TargetState::Published));
        assert_eq!(p.target_state("missing"), None);
    }

    #[test]
    fn fully_published_requires_all() {
        let all = ReleasePlan::new("1")
            .with_target("a", TargetState::Published)
            .with_target("b", TargetState::Published);
        assert!(all.fully_published());

        let partial = ReleasePlan::new("1")
            .with_target("a", TargetState::Published)
            .with_target("b", TargetState::Blocked);
        assert!(!partial.fully_published());
    }

    #[test]
    fn empty_plan_is_not_fully_published() {
        assert!(!ReleasePlan::new("1").fully_published());
    }

    #[test]
    fn pending_lists_unpublished() {
        let p = ReleasePlan::new("1")
            .with_target("github", TargetState::Published)
            .with_target("crates.io", TargetState::Blocked)
            .with_target("gitlab", TargetState::Published);
        assert_eq!(p.pending(), vec!["crates.io"]);
    }

    #[test]
    fn pending_empty_when_all_published() {
        let p = ReleasePlan::new("1").with_target("a", TargetState::Published);
        assert!(p.pending().is_empty());
    }

    #[test]
    fn plan_equality_is_structural() {
        let a = ReleasePlan::new("0.1.0").with_target("x", TargetState::Published);
        let b = ReleasePlan::new("0.1.0").with_target("x", TargetState::Published);
        assert_eq!(a, b);
    }

    #[test]
    fn distinct_versions_differ() {
        assert_ne!(ReleasePlan::new("0.1.0"), ReleasePlan::new("0.2.0"));
    }
}
