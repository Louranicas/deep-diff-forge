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
//! # Direct file pairs
//!
//! [`diff_files`] compares two regular files with `git diff --no-index` and returns
//! Git's patch bytes unchanged. Git runs from `/` on the canonical paths, so no
//! repository around the files is discovered and none of its configuration or
//! attributes apply: a reviewed checkout's own clean filter used to run when Git was
//! started inside it. `core.attributesFile=/dev/null`, `GIT_ATTR_NOSYSTEM=1` and
//! `core.autocrlf=false` also turn off the user's and the system's attributes and
//! end-of-line conversion, so Git diffs the bytes on disk. Symbolic links,
//! directories and special files are refused before Git runs, and so are inputs
//! whose sizes together exceed the byte budget.
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
    /// The most time the whole acquisition may take, every Git call included. A Git process
    /// still running at the deadline is killed and the acquisition fails with
    /// [`GitAcquireError::TimedOut`]: a stalled filesystem or a hung hook cannot hang the caller.
    pub time_limit: std::time::Duration,
}

/// The default [`GitAcquireOptions::time_limit`].
pub const DEFAULT_GIT_TIME_LIMIT: std::time::Duration = std::time::Duration::from_secs(120);

impl Default for GitAcquireOptions {
    fn default() -> Self {
        Self {
            include_untracked: false,
            byte_budget: crate::DEFAULT_BYTE_BUDGET,
            git_program: OsString::from("git"),
            env: Vec::new(),
            time_limit: DEFAULT_GIT_TIME_LIMIT,
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

/// Acquisition's own option on top of [`deep_diff_forge_core::git::ARGS`]: paths unquoted, so the
/// raw records keep their exact bytes.
const GLOBAL_ARGS: &[&str] = &["-c", "core.quotePath=false"];

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
    // diff.ignoreSubmodules or a .gitmodules `ignore = all` would drop a submodule's change from
    // the capture with exit 0, so a gate saw 0 files (bug hunt 2026-10-28, H1-02); the option
    // overrides both.
    "--ignore-submodules=none",
];

/// Upper bound on retained Git standard-error text.
const STDERR_CAP: u64 = 8 * 1024;

struct Runner<'a> {
    options: &'a GitAcquireOptions,
    cwd: PathBuf,
    /// When the whole acquisition must be done (see [`GitAcquireOptions::time_limit`]).
    deadline: std::time::Instant,
    /// Configuration passed to every call through `GIT_CONFIG_COUNT`/`KEY`/`VALUE`, which Git
    /// reads at command-line precedence (see [`neutral_filters`]), or [`diff_files`]'s
    /// `GIT_ATTR_NOSYSTEM`.
    config_env: Vec<(OsString, OsString)>,
}

/// The configuration that turns off every content filter the repository's own configuration
/// defines (`filter.<name>.clean`, `smudge`, `process`), as `GIT_CONFIG_*` variables. A filter
/// is a program Git runs on file content: one set in `.git/config` and named in `.gitattributes`
/// ran during working-tree acquisition, so reviewing a repository someone handed you ran their
/// code (bug hunt 2026-10-28, H2-01). Filters from the user's global or system configuration,
/// such as Git LFS, are the user's own and stay on.
fn neutral_filters(names: &[String]) -> Vec<(OsString, OsString)> {
    let mut env = Vec::new();
    let mut count = 0usize;
    for name in names {
        for (key, value) in [
            ("clean", ""),
            ("smudge", ""),
            ("process", ""),
            ("required", "false"),
        ] {
            env.push((
                OsString::from(format!("GIT_CONFIG_KEY_{count}")),
                OsString::from(format!("filter.{name}.{key}")),
            ));
            env.push((
                OsString::from(format!("GIT_CONFIG_VALUE_{count}")),
                OsString::from(value),
            ));
            count += 1;
        }
    }
    env.push((
        OsString::from("GIT_CONFIG_COUNT"),
        OsString::from(count.to_string()),
    ));
    env
}

/// [`neutral_filters`] for every filter the repository's local configuration (with its includes)
/// defines.
fn local_filters_off(
    options: &GitAcquireOptions,
    root: &Path,
    deadline: std::time::Instant,
) -> Result<Vec<(OsString, OsString)>, GitAcquireError> {
    let listing = Runner {
        options,
        cwd: root.to_path_buf(),
        deadline,
        config_env: Vec::new(),
    }
    .run(
        "git config",
        &[
            OsStr::new("config"),
            OsStr::new("--local"),
            OsStr::new("--includes"),
            OsStr::new("--null"),
            OsStr::new("--name-only"),
            OsStr::new("--get-regexp"),
            OsStr::new("^filter\\."),
        ],
        &[0, 1],
        1024 * 1024,
    )?;
    Ok(neutral_filters(&filter_names(&listing)))
}

/// The filter names in `git config --null --name-only` output (`filter.<name>.<key>` records).
fn filter_names(listing: &[u8]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for record in listing.split(|&b| b == 0 || b == b'\n') {
        let key = String::from_utf8_lossy(record);
        if let Some(name) = key
            .strip_prefix("filter.")
            .and_then(|rest| rest.rsplit_once('.'))
            .map(|(name, _)| name.to_owned())
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    names
}

/// The next poll interval while waiting for Git: doubling from 1 ms, at most 20 ms, so a quick
/// command costs little latency and a slow one little CPU.
fn next_pause(pause: std::time::Duration) -> std::time::Duration {
    (pause * 2).min(std::time::Duration::from_millis(20))
}

/// The acquisition's deadline: now plus its time limit.
#[expect(
    clippy::disallowed_methods,
    reason = "the git-acquisition clock door: one deadline bounds every Git call of an acquisition"
)]
fn deadline_after(limit: std::time::Duration) -> std::time::Instant {
    std::time::Instant::now() + limit
}

/// What an accepted Git run left behind.
struct GitOutput {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
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
        self.output(command, args, ok_codes, budget)
            .map(|done| done.stdout)
    }

    /// [`Self::run`], keeping the exit code and the first [`STDERR_CAP`] bytes of stderr for a
    /// caller that must judge them too.
    #[expect(
        clippy::disallowed_methods,
        reason = "the git-acquisition door: patch acquisition spawns git only here, hardened by core::git"
    )]
    fn output(
        &self,
        command: &'static str,
        args: &[&OsStr],
        ok_codes: &[i32],
        budget: usize,
    ) -> Result<GitOutput, GitAcquireError> {
        let mut cmd = Command::new(&self.options.git_program);
        cmd.current_dir(&self.cwd)
            .args(deep_diff_forge_core::git::ARGS)
            .args(GLOBAL_ARGS)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in deep_diff_forge_core::git::ENV_REMOVED {
            cmd.env_remove(name);
        }
        cmd.envs(deep_diff_forge_core::git::ENV_SET.iter().copied());
        cmd.envs(self.options.env.iter().map(|(k, v)| (k, v)));
        cmd.envs(self.config_env.iter().map(|(k, v)| (k, v)));
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
        // Read stdout on its own thread, so this one can enforce the deadline: before, a stalled
        // Git (a hung filesystem or hook) blocked the caller forever on `child.wait()`.
        let over_budget = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stdout = child.stdout.take();
        let flag = std::sync::Arc::clone(&over_budget);
        let stdout_reader = std::thread::spawn(move || {
            let mut out = Vec::new();
            // `byte_budget` is a public field: at usize::MAX a plain `+ 1` wrapped to a zero-byte
            // read, so a change was captured as none (bug hunt 2026-10-28, H1-10).
            let limit = (budget as u64).saturating_add(1);
            let read = stdout.map(|pipe| pipe.take(limit).read_to_end(&mut out));
            if out.len() > budget {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            (out, read)
        });
        let mut pause = std::time::Duration::from_millis(1);
        let stopped = |child: &mut std::process::Child| {
            let _ = child.kill();
            let _ = child.wait();
        };
        let status = loop {
            if over_budget.load(std::sync::atomic::Ordering::SeqCst) {
                stopped(&mut child);
                drop((stdout_reader, stderr_reader));
                return Err(GitAcquireError::BudgetExceeded {
                    limit_bytes: budget,
                });
            }
            if let Some(status) = child.try_wait().map_err(spawn_err)? {
                break status;
            }
            if std::time::Instant::now() >= self.deadline {
                stopped(&mut child);
                // The readers are detached, not joined: a child Git started (a filter, say)
                // can outlive the kill and hold the pipes open, and joining waited for it, so
                // the deadline did not bound the caller (bug hunt 2026-10-28, H1-03).
                drop((stdout_reader, stderr_reader));
                return Err(GitAcquireError::TimedOut {
                    command,
                    limit_ms: self.options.time_limit.as_millis(),
                });
            }
            std::thread::sleep(pause);
            pause = next_pause(pause);
        };
        let (out, read) = stdout_reader.join().unwrap_or_default();
        let stderr = stderr_reader.join().unwrap_or_default();
        if out.len() > budget {
            return Err(GitAcquireError::BudgetExceeded {
                limit_bytes: budget,
            });
        }
        if let Some(Err(e)) = read {
            return Err(spawn_err(e));
        }
        match status.code() {
            Some(code) if ok_codes.contains(&code) => Ok(GitOutput {
                code,
                stdout: out,
                stderr,
            }),
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
use utf8_path_from_bytes as path_from_bytes;

/// Off Unix a path is text: valid UTF-8 (the usual case) is reused without a copy, and anything
/// else is decoded lossily. Unix test builds compile it too, so its tests run on Unix CI.
#[cfg(any(not(unix), test))]
fn utf8_path_from_bytes(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(
        String::from_utf8(bytes)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
    )
}

/// Where a repository keeps its data: its top level, its Git directory and its common directory
/// (they differ for a linked worktree, or a `.git` file pointing elsewhere), and whether its object
/// store borrows from another (`objects/info/alternates`). A caller that may read only some
/// directories checks all three, not only the top level: a `.git` file inside an allowed root
/// can point at a repository outside it (bug hunt 2026-10-28, H2-02).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryDirs {
    /// The working tree's top level.
    pub top: PathBuf,
    /// The Git directory (`git rev-parse --absolute-git-dir`).
    pub git_dir: PathBuf,
    /// The common directory shared by linked worktrees (`git rev-parse --git-common-dir`).
    pub common_dir: PathBuf,
    /// The object store lists alternates.
    pub has_alternates: bool,
}

/// The version line Git reports (`git version 2.51.0`), run the way an acquisition runs Git: the
/// same program, environment and time limit, from `cwd` (no repository is needed). Health checks
/// use it to tell whether Git inputs can work at all.
///
/// # Errors
/// Any [`GitAcquireError`] from running Git: not found, timed out, or a failing exit.
pub fn git_version(cwd: &Path, options: &GitAcquireOptions) -> Result<String, GitAcquireError> {
    let runner = Runner {
        options,
        cwd: cwd.to_path_buf(),
        deadline: deadline_after(options.time_limit),
        config_env: Vec::new(),
    };
    let out = runner.run("git version", &[OsStr::new("version")], &[0], 4096)?;
    Ok(String::from_utf8_lossy(&out).trim().to_owned())
}

/// Resolve [`RepositoryDirs`] for the repository containing `repo`, every path canonical.
///
/// # Errors
/// Any [`GitAcquireError`] from running Git or resolving its answer.
pub fn repository_dirs(
    repo: &Path,
    options: &GitAcquireOptions,
) -> Result<RepositoryDirs, GitAcquireError> {
    let runner = Runner {
        options,
        cwd: repo.to_path_buf(),
        deadline: deadline_after(options.time_limit),
        config_env: Vec::new(),
    };
    let out = runner.run(
        "git rev-parse",
        &[
            OsStr::new("rev-parse"),
            OsStr::new("--show-toplevel"),
            OsStr::new("--absolute-git-dir"),
            OsStr::new("--git-common-dir"),
        ],
        &[0],
        64 * 1024,
    )?;
    let text = String::from_utf8(out).map_err(|_| GitAcquireError::InvalidSelection {
        detail: "the repository's directories are not UTF-8".to_owned(),
    })?;
    let lines: Vec<&str> = text.lines().collect();
    let [top, git_dir, common] = lines[..] else {
        return Err(GitAcquireError::InvalidSelection {
            detail: "git rev-parse did not name the repository's directories".to_owned(),
        });
    };
    let canonical = |path: PathBuf| {
        std::fs::canonicalize(&path).map_err(|e| GitAcquireError::InvalidSelection {
            detail: format!("cannot resolve {}: {e}", path.display()),
        })
    };
    let top = canonical(PathBuf::from(top))?;
    // A relative common directory is relative to the directory Git ran in.
    let common_dir = canonical(repo.join(common))?;
    Ok(RepositoryDirs {
        has_alternates: common_dir.join("objects/info/alternates").exists(),
        top,
        git_dir: canonical(PathBuf::from(git_dir))?,
        common_dir,
    })
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
    let deadline = deadline_after(options.time_limit);
    let probe = Runner {
        options,
        cwd: repo.to_path_buf(),
        deadline,
        config_env: Vec::new(),
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
        deadline,
        config_env: local_filters_off(options, &root, deadline)?,
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

/// Global options for [`diff_files`], ahead of the subcommand: no attributes file, so no filter,
/// end-of-line conversion or diff driver applies, and no CRLF conversion.
const FILE_PAIR_GLOBAL_ARGS: &[&str] = &[
    "-c",
    "core.attributesFile=/dev/null",
    "-c",
    "core.autocrlf=false",
];

/// The `git diff --no-index` options for [`diff_files`]: [`DIFF_ARGS`] without the raw records,
/// which name only the old path when the two names differ and so cannot be paired with the block.
const FILE_PAIR_DIFF_ARGS: &[&str] = &[
    "diff",
    "--no-index",
    "--no-ext-diff",
    "--no-color",
    "--patch",
    "--binary",
    "--full-index",
    "--no-textconv",
    "--src-prefix=a/",
    "--dst-prefix=b/",
    "--",
];

/// `path`'s canonical form and size, or why it may not be compared: it must exist, and both it
/// and its canonical form must be a regular file, not a symbolic link, directory or special file.
/// Only the last component is refused as a link: a linked parent directory resolves, and Git's
/// headers then name the resolved path.
fn regular_file(side: &'static str, path: &Path) -> Result<(PathBuf, u64), GitAcquireError> {
    let unreadable = |e: std::io::Error| GitAcquireError::FileUnreadable {
        side,
        detail: format!("{}: {e}", path.display()),
    };
    let refuse_kind = |meta: &std::fs::Metadata| {
        let kind = meta.file_type();
        let what = if kind.is_symlink() {
            "a symbolic link"
        } else if kind.is_dir() {
            "a directory"
        } else if kind.is_file() {
            return Ok(());
        } else {
            "a special file (FIFO, socket or device)"
        };
        Err(GitAcquireError::FileRefused {
            side,
            detail: format!("{} is {what}, not a regular file", path.display()),
        })
    };
    refuse_kind(&std::fs::symlink_metadata(path).map_err(unreadable)?)?;
    let canonical = std::fs::canonicalize(path).map_err(unreadable)?;
    // Checked again on the resolved path, which is what Git reads. Only a swap between the two
    // looks can make this differ from the first check, so no test reaches it; it narrows the
    // window and does not close it (see `diff_files`).
    let meta = std::fs::symlink_metadata(&canonical).map_err(unreadable)?;
    refuse_kind(&meta)?;
    Ok((canonical, meta.len()))
}

/// Compare two regular files with `git diff --no-index`: the exact patch bytes Git emits, empty
/// when the files are identical. Headers name the canonical paths without their leading `/`
/// (`a/home/me/old.rs`), because Git runs from `/` (see the module's "Direct file pairs").
///
/// The files are checked, then read by Git: a concurrent writer can change them in between, so
/// this is not a sandbox against one. A side Git cannot open by then is an error, never an
/// empty patch. [`GitAcquireOptions::byte_budget`] bounds the two sizes together and Git's
/// output, and [`GitAcquireOptions::time_limit`] bounds the run.
///
/// # Errors
/// [`GitAcquireError::FileUnreadable`] for a side that cannot be inspected,
/// [`GitAcquireError::FileRefused`] for one that is not a regular file or for sizes over the
/// budget, [`GitAcquireError::InvalidSelection`] when untracked files are requested, and any
/// error from running Git.
pub fn diff_files(
    old: &Path,
    new: &Path,
    options: &GitAcquireOptions,
) -> Result<Vec<u8>, GitAcquireError> {
    if options.include_untracked {
        return Err(GitAcquireError::InvalidSelection {
            detail: "untracked files can only be included in a working-tree selection".to_owned(),
        });
    }
    let (old, old_len) = regular_file("old", old)?;
    let (new, new_len) = regular_file("new", new)?;
    let budget = options.byte_budget;
    if old_len.saturating_add(new_len) > budget as u64 {
        return Err(GitAcquireError::FileRefused {
            side: "old and new",
            detail: format!("{old_len} + {new_len} bytes exceed the {budget}-byte input budget"),
        });
    }
    let runner = Runner {
        options,
        cwd: PathBuf::from("/"),
        deadline: deadline_after(options.time_limit),
        config_env: vec![(OsString::from("GIT_ATTR_NOSYSTEM"), OsString::from("1"))],
    };
    let from_root = |path: &Path| {
        path.strip_prefix("/")
            .unwrap_or(path)
            .as_os_str()
            .to_owned()
    };
    let (old, new) = (from_root(&old), from_root(&new));
    let mut args: Vec<&OsStr> = FILE_PAIR_GLOBAL_ARGS.iter().map(OsStr::new).collect();
    args.extend(FILE_PAIR_DIFF_ARGS.iter().map(OsStr::new));
    args.extend([old.as_os_str(), new.as_os_str()]);
    // `--no-index` exits 0, silently, for identical files and 1, with a patch, when they differ.
    // It also exits 1, with nothing on stdout, for a path it cannot open ("Could not access"):
    // one removed after the checks above. Anything but those two shapes is a failure, so a
    // vanished file is never reviewed as unchanged.
    let command = "git diff --no-index";
    let done = runner.output(command, &args, &[0, 1], budget)?;
    if !done.stderr.is_empty() || (done.code == 1) == done.stdout.is_empty() {
        return Err(GitAcquireError::CommandFailed {
            command,
            code: Some(done.code),
            stderr: String::from_utf8_lossy(&done.stderr).trim().to_owned(),
        });
    }
    Ok(done.stdout)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_unix_paths_keep_utf8_and_decode_other_bytes_lossily() {
        assert_eq!(
            utf8_path_from_bytes(b"src/caf\xc3\xa9.rs".to_vec()),
            PathBuf::from("src/caf\u{e9}.rs")
        );
        assert_eq!(
            utf8_path_from_bytes(b"src/a\xffb.rs".to_vec()),
            PathBuf::from("src/a\u{fffd}b.rs")
        );
    }
}

#[cfg(test)]
#[cfg(unix)]
mod runner_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    #[expect(
        clippy::disallowed_methods,
        reason = "door: a stub Git that hit ETXTBSY is retried after a short pause"
    )]
    fn pause_for_busy_stub() {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    /// A stand-in Git that ignores its arguments and prints exactly `bytes` bytes.
    fn printing(name: &str, bytes: usize) -> (PathBuf, GitAcquireOptions) {
        let dir = std::env::temp_dir().join(format!("ddf-runner-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let stub = dir.join("git");
        std::fs::write(
            &stub,
            format!("#!/bin/sh\nhead -c {bytes} /dev/zero | tr '\\0' x\n"),
        )
        .unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        let options = GitAcquireOptions {
            git_program: stub.into_os_string(),
            time_limit: std::time::Duration::from_secs(30),
            ..GitAcquireOptions::default()
        };
        (dir, options)
    }

    /// Output of exactly the budget is accepted; one byte more is refused.
    #[test]
    fn output_at_the_budget_is_kept_and_one_byte_over_is_refused() {
        let (dir, options) = printing("budget", 100);
        let runner = Runner {
            options: &options,
            cwd: dir.clone(),
            deadline: deadline_after(options.time_limit),
            config_env: Vec::new(),
        };
        // A sibling test that forks while the stub is still open for writing can make the
        // first exec fail with ETXTBSY; retry that harness race (see tests/git_acquire.rs).
        let mut kept = runner.run("stub", &[], &[0], 100);
        for _ in 0..100 {
            match &kept {
                Err(GitAcquireError::Spawn { detail, .. }) if detail.contains("os error 26") => {
                    pause_for_busy_stub();
                    kept = runner.run("stub", &[], &[0], 100);
                }
                _ => break,
            }
        }
        assert_eq!(kept.unwrap().len(), 100);
        assert_eq!(
            runner.run("stub", &[], &[0], usize::MAX).unwrap().len(),
            100,
            "the largest budget reads everything"
        );
        assert!(matches!(
            runner.run("stub", &[], &[0], 99),
            Err(GitAcquireError::BudgetExceeded { limit_bytes: 99 })
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn polling_backs_off_from_one_millisecond_to_twenty() {
        let ms = std::time::Duration::from_millis;
        assert_eq!(next_pause(ms(1)), ms(2));
        assert_eq!(next_pause(ms(8)), ms(16));
        assert_eq!(next_pause(ms(16)), ms(20));
        assert_eq!(next_pause(ms(20)), ms(20));
    }
}
