//! Guard files (F6.b): the files whose change decides how an agent's work is judged or checked,
//! a closed table matched on whole path components. A path is a guard file by its name where
//! the table names a file that is one wherever it is, else by its whole path from the top of
//! the repository: `src/warden.toml.rs` and `docs/.github/workflows/x.yml` are none.

/// What a guard file decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardFile {
    /// The policy file a stop judges with (`--policy`, `warden.toml` by default).
    Policy,
    /// A `.gitattributes` file: how Git shows, compares and merges the files it names.
    GitAttributes,
    /// A CI workflow: what runs on the change after it leaves the machine.
    CiWorkflow,
    /// Lint or tool configuration: what the project's checks report.
    LintConfig,
    /// The agent client's hook settings: whether the hooks run.
    HookSettings,
}

impl GuardFile {
    /// The name of the kind, as a message gives it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Policy => "policy",
            Self::GitAttributes => "git_attributes",
            Self::CiWorkflow => "ci_workflow",
            Self::LintConfig => "lint_config",
            Self::HookSettings => "hook_settings",
        }
    }
}

/// Files that are guard files in any directory, by their name: Git reads a `.gitattributes`
/// in every directory, and these tools look for their configuration upwards from a file.
const ANY_DIRECTORY: &[(&str, GuardFile)] = &[
    (".gitattributes", GuardFile::GitAttributes),
    ("clippy.toml", GuardFile::LintConfig),
    (".clippy.toml", GuardFile::LintConfig),
    ("rustfmt.toml", GuardFile::LintConfig),
    (".rustfmt.toml", GuardFile::LintConfig),
    ("deny.toml", GuardFile::LintConfig),
    ("ruff.toml", GuardFile::LintConfig),
    (".ruff.toml", GuardFile::LintConfig),
    (".flake8", GuardFile::LintConfig),
    (".pylintrc", GuardFile::LintConfig),
    (".eslintrc", GuardFile::LintConfig),
    (".eslintrc.json", GuardFile::LintConfig),
    (".eslintrc.js", GuardFile::LintConfig),
    (".eslintrc.cjs", GuardFile::LintConfig),
    (".eslintrc.yml", GuardFile::LintConfig),
    (".eslintrc.yaml", GuardFile::LintConfig),
    ("eslint.config.js", GuardFile::LintConfig),
    ("eslint.config.mjs", GuardFile::LintConfig),
    ("eslint.config.cjs", GuardFile::LintConfig),
    (".golangci.yml", GuardFile::LintConfig),
    (".golangci.yaml", GuardFile::LintConfig),
];

/// Files that are guard files at the top of the repository, by their whole path.
const AT_TOP: &[(&[&str], GuardFile)] = &[
    (&[".gitlab-ci.yml"], GuardFile::CiWorkflow),
    (&[".circleci", "config.yml"], GuardFile::CiWorkflow),
    (&[".pre-commit-config.yaml"], GuardFile::LintConfig),
    (&[".cargo", "config.toml"], GuardFile::LintConfig),
    (&[".cargo", "config"], GuardFile::LintConfig),
    (&[".claude", "settings.json"], GuardFile::HookSettings),
    (&[".claude", "settings.local.json"], GuardFile::HookSettings),
];

/// Directories at the top of the repository every file under which is a guard file.
const UNDER_TOP: &[(&[&str], GuardFile)] = &[
    (&[".github", "workflows"], GuardFile::CiWorkflow),
    (&[".forgejo", "workflows"], GuardFile::CiWorkflow),
    (&[".gitea", "workflows"], GuardFile::CiWorkflow),
];

/// The components of a path from the top of the repository: no empty or `.` component.
fn components(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect()
}

/// What the file at `path` (from the top of the repository) guards, `None` for any other file.
/// `policy` is the policy file's path as a stop is given it (`--policy`).
#[must_use]
pub fn guard_file(path: &str, policy: &str) -> Option<GuardFile> {
    let parts = components(path);
    if !parts.is_empty() && parts == components(policy) {
        return Some(GuardFile::Policy);
    }
    let name = parts.last()?;
    let named = ANY_DIRECTORY.iter().find(|(file, _)| file == name);
    let at_top = AT_TOP.iter().find(|(whole, _)| parts == *whole);
    let under = UNDER_TOP
        .iter()
        .find(|(dir, _)| parts.len() > dir.len() && parts.starts_with(dir));
    named
        .map(|(_, kind)| *kind)
        .or_else(|| at_top.map(|(_, kind)| *kind))
        .or_else(|| under.map(|(_, kind)| *kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_files_are_matched_on_whole_components() {
        let policy = "warden.toml";
        for (path, kind) in [
            ("warden.toml", GuardFile::Policy),
            ("./warden.toml", GuardFile::Policy),
            (".gitattributes", GuardFile::GitAttributes),
            ("src/.gitattributes", GuardFile::GitAttributes),
            (".github/workflows/x.yml", GuardFile::CiWorkflow),
            (".gitlab-ci.yml", GuardFile::CiWorkflow),
            ("crates/a/clippy.toml", GuardFile::LintConfig),
            (".claude/settings.json", GuardFile::HookSettings),
        ] {
            assert_eq!(guard_file(path, policy), Some(kind), "{path}");
        }
        for path in [
            "src/warden.toml.rs",
            "src/warden.toml",
            "a.gitattributes",
            ".gitattributes.bak",
            "docs/.github/workflows/x.yml",
            ".github/workflows",
            ".github/workflows.yml",
            "sub/.gitlab-ci.yml",
            "x/.claude/settings.json",
            "",
        ] {
            assert_eq!(guard_file(path, policy), None, "{path}");
        }
        assert_eq!(
            guard_file("ci/policy.toml", "ci/policy.toml"),
            Some(GuardFile::Policy)
        );
        assert_eq!(guard_file("warden.toml", "ci/policy.toml"), None);
    }
}
