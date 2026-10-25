// DDF integration: R02, R03, R07. Native structured Git acquisition: run Git with a fixed argv (no shell), bind the selection to resolved object IDs, capture untracked files without touching the index, and keep the exact patch bytes authoritative.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-patch--git_acquire
//! Structured Git acquisition port.
//!
//! Every Git invocation is a fixed argument vector (never a shell string) with
//! the hardening prefix
//! `git --no-optional-locks -c core.quotePath=false -c core.hooksPath=/dev/null
//! -c diff.external= -c core.fsmonitor=false`. The diff itself is
//! `diff --no-ext-diff --no-color --raw -z --patch --binary --full-index
//! --no-abbrev --no-textconv --src-prefix=a/ --dst-prefix=b/ --no-relative
//! --submodule=short <selection> --`; the extra flags pin what user or repository
//! configuration could otherwise change (abbreviated raw IDs, `diff.noprefix`,
//! `diff.mnemonicPrefix`, `diff.relative`, textconv output that would not apply,
//! `diff.submodule=log` blocks without a `diff --git` header).
//!
//! # Untracked files
//!
//! With [`GitAcquireOptions::include_untracked`], untracked non-ignored files
//! are listed with `git ls-files --others --exclude-standard -z` and each is
//! diffed with `git diff --no-index … -- /dev/null <path>` (same flags). That
//! command reads the file directly and never writes the index or object
//! database, so acquisition does not mutate the repository (no `git add -N`).
//! Untracked entries Git lists as directories (nested repositories, shown with
//! a trailing `/`) cannot be diffed as a file and are reported in
//! [`GitAcquisition::omitted_untracked`] instead of being silently dropped.
//!
//! # Environment
//!
//! Repository-redirecting variables (`GIT_DIR`, `GIT_WORK_TREE`,
//! `GIT_INDEX_FILE`, …) and diff-shaping ones (`GIT_EXTERNAL_DIFF`,
//! `GIT_DIFF_OPTS`) are removed so the named repository is the one read.
//! Repository-configured clean filters still run when Git reads working-tree
//! files; that is Git's own trust model for the repository and is not altered.

use crate::git_raw::{GitAcquireError, GitChangeKind, GitChangeOrigin, GitPath, StructuredDiff};
use std::ffi::{OsStr, OsString};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What to compare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitSelection {
    /// Working tree (staged and unstaged changes) against `HEAD`; against the
    /// empty tree when `HEAD` is unborn.
    WorkingTree,
    /// Index (staged changes only) against `HEAD`.
    Staged,
    /// Commit `base` against commit `target` (`base..target`).
    Range {
        /// Revision on the old side.
        base: String,
        /// Revision on the new side.
        target: String,
    },
}

impl GitSelection {
    /// Parse a two-dot `A..B` range. Three-dot (merge-base) ranges, empty
    /// sides and option-like revisions are refused.
    ///
    /// # Errors
    /// [`GitAcquireError::InvalidSelection`].
    pub fn parse_range(spec: &str) -> Result<Self, GitAcquireError> {
        let invalid = |detail: &str| GitAcquireError::InvalidSelection {
            detail: detail.to_owned(),
        };
        if spec.contains("...") {
            return Err(invalid(
                "three-dot (merge-base) ranges are not supported; use A..B",
            ));
        }
        let (base, target) = spec
            .split_once("..")
            .ok_or_else(|| invalid("expected a two-dot range A..B"))?;
        if base.is_empty() || target.is_empty() {
            return Err(invalid("both sides of A..B must be named"));
        }
        if target.contains("..") {
            return Err(invalid("expected exactly one `..`"));
        }
        for side in [base, target] {
            if side.starts_with('-') || side.chars().any(char::is_control) {
                return Err(invalid(
                    "a revision may not start with `-` or contain control characters",
                ));
            }
        }
        Ok(Self::Range {
            base: base.to_owned(),
            target: target.to_owned(),
        })
    }

    /// Stable snake-case label.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::WorkingTree => "working_tree",
            Self::Staged => "staged",
            Self::Range { .. } => "range",
        }
    }
}

/// Acquisition options.
#[derive(Debug, Clone)]
pub struct GitAcquireOptions {
    /// Also capture untracked, non-ignored files as additions
    /// ([`GitSelection::WorkingTree`] only).
    pub include_untracked: bool,
    /// Maximum total patch/metadata bytes accepted from Git.
    pub byte_budget: usize,
    /// Git executable to run.
    pub git_program: OsString,
    /// Extra environment for every Git invocation (e.g. config isolation).
    pub env: Vec<(OsString, OsString)>,
}

impl Default for GitAcquireOptions {
    fn default() -> Self {
        Self {
            include_untracked: false,
            byte_budget: crate::DEFAULT_BYTE_BUDGET,
            git_program: OsString::from("git"),
            env: Vec::new(),
        }
    }
}

/// A structured acquisition: the authoritative patch, its paired records and
/// the resolved identity of what was compared.
#[derive(Debug, Clone)]
pub struct GitAcquisition {
    root: PathBuf,
    selection: GitSelection,
    base: String,
    target: Option<String>,
    diff: StructuredDiff,
    omitted_untracked: Vec<GitPath>,
}

impl GitAcquisition {
    /// Repository top-level directory Git reported.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// The selection that was acquired.
    #[must_use]
    pub fn selection(&self) -> &GitSelection {
        &self.selection
    }
    /// Resolved old-side object ID: a commit, or the empty tree for an unborn
    /// `HEAD`.
    #[must_use]
    pub fn base(&self) -> &str {
        &self.base
    }
    /// Resolved new-side commit for a range; `None` for the working tree/index.
    #[must_use]
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }
    /// Paired records and authoritative patch bytes.
    #[must_use]
    pub fn diff(&self) -> &StructuredDiff {
        &self.diff
    }
    /// Untracked entries that could not be captured as a file (nested
    /// repositories); reported, never silently dropped.
    #[must_use]
    pub fn omitted_untracked(&self) -> &[GitPath] {
        &self.omitted_untracked
    }
}

const ENV_REMOVED: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
    "GIT_EXTERNAL_DIFF",
    "GIT_DIFF_OPTS",
];

const GLOBAL_ARGS: &[&str] = &[
    "--no-optional-locks",
    "-c",
    "core.quotePath=false",
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "diff.external=",
    "-c",
    "core.fsmonitor=false",
];

const DIFF_ARGS: &[&str] = &[
    "diff",
    "--no-ext-diff",
    "--no-color",
    "--raw",
    "-z",
    "--patch",
    "--binary",
    "--full-index",
    "--no-abbrev",
    "--no-textconv",
    "--src-prefix=a/",
    "--dst-prefix=b/",
    "--no-relative",
    "--submodule=short",
];

/// Upper bound on retained Git standard-error text.
const STDERR_CAP: u64 = 8 * 1024;

struct Runner<'a> {
    options: &'a GitAcquireOptions,
    cwd: PathBuf,
}

impl Runner<'_> {
    /// Run Git; accept only the listed exit codes; never buffer past `budget`.
    fn run(
        &self,
        command: &'static str,
        args: &[&OsStr],
        ok_codes: &[i32],
        budget: usize,
    ) -> Result<Vec<u8>, GitAcquireError> {
        let mut cmd = Command::new(&self.options.git_program);
        cmd.current_dir(&self.cwd)
            .args(GLOBAL_ARGS)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in ENV_REMOVED {
            cmd.env_remove(name);
        }
        cmd.envs(self.options.env.iter().map(|(k, v)| (k, v)));
        let spawn_err = |e: std::io::Error| GitAcquireError::Spawn {
            command,
            detail: e.to_string(),
        };
        let mut child = cmd.spawn().map_err(spawn_err)?;
        let stderr = child.stderr.take();
        // Drain stderr concurrently so a chatty Git cannot deadlock on a full pipe.
        let stderr_reader = std::thread::spawn(move || {
            let mut kept = Vec::new();
            if let Some(mut pipe) = stderr {
                let _ = pipe.by_ref().take(STDERR_CAP).read_to_end(&mut kept);
                let _ = std::io::copy(&mut pipe, &mut std::io::sink());
            }
            kept
        });
        let mut out = Vec::new();
        let read = child
            .stdout
            .take()
            .map(|pipe| pipe.take(budget as u64 + 1).read_to_end(&mut out));
        if out.len() > budget {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stderr_reader.join();
            return Err(GitAcquireError::BudgetExceeded {
                limit_bytes: budget,
            });
        }
        let status = child.wait().map_err(spawn_err)?;
        let stderr = stderr_reader.join().unwrap_or_default();
        if let Some(Err(e)) = read {
            return Err(spawn_err(e));
        }
        match status.code() {
            Some(code) if ok_codes.contains(&code) => Ok(out),
            code => Err(GitAcquireError::CommandFailed {
                command,
                code,
                stderr: String::from_utf8_lossy(&stderr).trim().to_owned(),
            }),
        }
    }

    /// Resolve `rev` to a full commit ID.
    fn commit_id(&self, rev: &str) -> Result<Option<String>, GitAcquireError> {
        let spec = OsString::from(format!("{rev}^{{commit}}"));
        let out = self.run(
            "git rev-parse",
            &[
                OsStr::new("rev-parse"),
                OsStr::new("--verify"),
                OsStr::new("--quiet"),
                OsStr::new("--end-of-options"),
                &spec,
            ],
            &[0, 1],
            4096,
        )?;
        let id = String::from_utf8_lossy(&out).trim().to_owned();
        Ok((!id.is_empty()).then_some(id))
    }

    fn empty_tree(&self) -> Result<String, GitAcquireError> {
        let out = self.run(
            "git hash-object",
            &[
                OsStr::new("hash-object"),
                OsStr::new("-t"),
                OsStr::new("tree"),
                OsStr::new("--stdin"),
            ],
            &[0],
            4096,
        )?;
        Ok(String::from_utf8_lossy(&out).trim().to_owned())
    }
}

fn os_arg(path: &GitPath) -> Result<OsString, GitAcquireError> {
    path.to_os_string()
        .ok_or_else(|| GitAcquireError::InvalidSelection {
            detail: "untracked path is not representable on this platform".to_owned(),
        })
}

#[cfg(unix)]
fn path_from_bytes(bytes: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt as _;
    PathBuf::from(OsString::from_vec(bytes))
}

#[cfg(not(unix))]
fn path_from_bytes(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
}

/// Acquire `selection` from the repository containing `repo`.
///
/// Read-only: no index refresh (`--no-optional-locks`), no `git add -N`, no
/// hooks, no external diff drivers.
///
/// # Errors
/// Any [`GitAcquireError`]; nothing partial is returned.
pub fn acquire(
    repo: &Path,
    selection: &GitSelection,
    options: &GitAcquireOptions,
) -> Result<GitAcquisition, GitAcquireError> {
    if options.include_untracked && *selection != GitSelection::WorkingTree {
        return Err(GitAcquireError::InvalidSelection {
            detail: "untracked files can only be included in a working-tree selection".to_owned(),
        });
    }
    let probe = Runner {
        options,
        cwd: repo.to_path_buf(),
    };
    let mut top = probe.run(
        "git rev-parse",
        &[OsStr::new("rev-parse"), OsStr::new("--show-toplevel")],
        &[0],
        64 * 1024,
    )?;
    if top.last() == Some(&b'\n') {
        top.pop();
    }
    let root = path_from_bytes(top);
    let runner = Runner {
        options,
        cwd: root.clone(),
    };
    let head = || -> Result<String, GitAcquireError> {
        match runner.commit_id("HEAD")? {
            Some(id) => Ok(id),
            None => runner.empty_tree(),
        }
    };
    let resolve = |rev: &str| -> Result<String, GitAcquireError> {
        runner
            .commit_id(rev)?
            .ok_or_else(|| GitAcquireError::InvalidSelection {
                detail: format!("`{rev}` does not name a commit"),
            })
    };
    let (base, target, mut selection_args): (String, Option<String>, Vec<OsString>) =
        match selection {
            GitSelection::WorkingTree => {
                let base = head()?;
                (base.clone(), None, vec![OsString::from(base)])
            }
            GitSelection::Staged => {
                let base = head()?;
                (
                    base.clone(),
                    None,
                    vec![OsString::from("--cached"), OsString::from(base)],
                )
            }
            GitSelection::Range { base, target } => {
                let (base, target) = (resolve(base)?, resolve(target)?);
                (
                    base.clone(),
                    Some(target.clone()),
                    vec![OsString::from(base), OsString::from(target)],
                )
            }
        };
    selection_args.push(OsString::from("--"));
    let mut args: Vec<&OsStr> = DIFF_ARGS.iter().map(OsStr::new).collect();
    args.extend(selection_args.iter().map(OsString::as_os_str));
    let tracked = runner.run("git diff", &args, &[0], options.byte_budget)?;
    let mut diff = StructuredDiff::parse(&tracked)?;
    let omitted_untracked = if options.include_untracked {
        capture_untracked(&runner, &mut diff)?
    } else {
        Vec::new()
    };
    Ok(GitAcquisition {
        root,
        selection: selection.clone(),
        base,
        target,
        diff,
        omitted_untracked,
    })
}

/// Append each untracked, non-ignored file as an addition produced by
/// `git diff --no-index -- /dev/null <path>`; return the entries that are
/// directories (nested repositories) and could not be captured.
fn capture_untracked(
    runner: &Runner<'_>,
    diff: &mut StructuredDiff,
) -> Result<Vec<GitPath>, GitAcquireError> {
    let mut omitted_untracked = Vec::new();
    let listing = runner.run(
        "git ls-files",
        &[
            OsStr::new("ls-files"),
            OsStr::new("--others"),
            OsStr::new("--exclude-standard"),
            OsStr::new("-z"),
        ],
        &[0],
        runner.options.byte_budget,
    )?;
    for raw in listing.split(|&b| b == 0).filter(|p| !p.is_empty()) {
        let path = GitPath::from_bytes(raw.to_vec());
        if raw.ends_with(b"/") {
            omitted_untracked.push(path);
            continue;
        }
        let file = os_arg(&path)?;
        let remaining = runner
            .options
            .byte_budget
            .saturating_sub(diff.patch_bytes().len());
        let mut args: Vec<&OsStr> = DIFF_ARGS.iter().map(OsStr::new).collect();
        args.extend([
            OsStr::new("--no-index"),
            OsStr::new("--"),
            OsStr::new("/dev/null"),
            &file,
        ]);
        // `--no-index` exits 1 when the inputs differ, which they always do here.
        let out = runner.run("git diff --no-index", &args, &[0, 1], remaining)?;
        let added = diff.append_stream(&out, GitChangeOrigin::Untracked)?;
        let ok = matches!(added, [change]
            if change.kind == GitChangeKind::Added && change.new_path.as_ref() == Some(&path));
        if !ok {
            return Err(GitAcquireError::IdentityMismatch {
                record: diff.changes().len().saturating_sub(1),
                detail: format!(
                    "`--no-index` output for untracked `{}` is not a single addition of that path",
                    path.display_lossy()
                ),
            });
        }
    }
    Ok(omitted_untracked)
}
