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
//!
//! # Configuration trust
//!
//! A content filter (`filter.<name>.clean`, `smudge`, `process`) defined in the
//! repository's own configuration (`local` or `worktree` scope, with their
//! includes) is turned off, byte for byte by name: the repository chooses which
//! files a filter gets, and must not also choose the program. A filter from a
//! protected scope (`global`, `system`, `command`: `-c` and the caller's
//! `GIT_CONFIG_COUNT`/`KEY`/`VALUE`, which DDF's own entries are numbered after),
//! such as Git LFS, is the user's own and runs, so the capture is what the
//! repository would store. No scope can turn on fsmonitor, hooks, an external
//! diff, textconv or a transport: those are overridden on the command line,
//! which takes precedence over every file.

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
    /// reads at command-line precedence (see [`neutral_filters`]).
    config_env: Vec<(OsString, OsString)>,
}

/// The configuration that turns off every content filter the repository's own configuration
/// defines (`filter.<name>.clean`, `smudge`, `process`), as `GIT_CONFIG_*` variables. A filter
/// is a program Git runs on file content: one set in `.git/config` and named in `.gitattributes`
/// ran during working-tree acquisition, so reviewing a repository someone handed you ran their
/// code (bug hunt 2026-10-28, H2-01). The entries are numbered from `first`, after the caller's
/// own (see [`caller_config_count`]), so they add to the caller's configuration, not replace it.
fn neutral_filters(
    names: &[Vec<u8>],
    first: usize,
) -> Result<Vec<(OsString, OsString)>, GitAcquireError> {
    let mut env = Vec::new();
    let mut count = first;
    for name in names {
        for (key, value) in [
            ("clean", ""),
            ("smudge", ""),
            ("process", ""),
            ("required", "false"),
        ] {
            env.push((
                OsString::from(format!("GIT_CONFIG_KEY_{count}")),
                filter_key(name, key)?,
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
    Ok(env)
}

/// `filter.<name>.<key>` with the name's bytes as Git listed them. A subsection name is raw bytes
/// matched exactly: a lossy decode turned `\xff` into U+FFFD, so the override named another
/// filter and the repository's own still ran (review of 2026-10-31).
#[cfg(unix)]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the same signature as the off-Unix form, which can refuse"
)]
fn filter_key(name: &[u8], key: &str) -> Result<OsString, GitAcquireError> {
    use std::os::unix::ffi::OsStringExt as _;
    Ok(OsString::from_vec(
        [b"filter.", name, b".", key.as_bytes()].concat(),
    ))
}

#[cfg(not(unix))]
use utf8_filter_key as filter_key;

/// Off Unix an environment value is text, so a filter name that is not UTF-8 cannot be named
/// exactly, and acquisition is refused rather than leaving that filter on. Unix test builds
/// compile it too, so its tests run on Unix CI.
#[cfg(any(not(unix), test))]
fn utf8_filter_key(name: &[u8], key: &str) -> Result<OsString, GitAcquireError> {
    let name = std::str::from_utf8(name).map_err(|_| GitAcquireError::InvalidSelection {
        detail: "a repository filter name is not representable on this platform".to_owned(),
    })?;
    Ok(OsString::from(format!("filter.{name}.{key}")))
}

/// How many `GIT_CONFIG_KEY_<n>` entries the caller already passes: the `GIT_CONFIG_COUNT` in
/// [`GitAcquireOptions::env`], else the inherited one, else none. DDF's entries are numbered
/// after them; before, they started at 0 and replaced the caller's. A count Git reads but this
/// does not (Git's `strtoul` takes leading white space) is refused rather than guessed.
#[expect(
    clippy::disallowed_methods,
    reason = "the git-config-count door: DDF's GIT_CONFIG_* entries follow the caller's own"
)]
fn caller_config_count(options: &GitAcquireOptions) -> Result<usize, GitAcquireError> {
    let count = options
        .env
        .iter()
        .rev()
        .find(|(name, _)| name == "GIT_CONFIG_COUNT")
        .map(|(_, value)| value.clone())
        .or_else(|| std::env::var_os("GIT_CONFIG_COUNT"));
    match count.as_ref().map(|count| count.to_str()) {
        None | Some(Some("")) => Ok(0),
        Some(text) => text.and_then(|text| text.parse().ok()).ok_or_else(|| {
            GitAcquireError::InvalidSelection {
                detail: format!("GIT_CONFIG_COUNT is not a plain count: {count:?}"),
            }
        }),
    }
}

/// The scopes Git calls protected (git-config(1), "Protected configuration"): the user's global
/// file, the system file, and the command line or `GIT_CONFIG_*` environment (kept: see
/// [`caller_config_count`]). Whoever can write them can already run code as the user without
/// Git, so a filter they define is the user's own program, and its clean output (a Git LFS
/// pointer, say) is what the repository stores.
const PROTECTED_SCOPES: &[&[u8]] = &[b"system", b"global", b"command"];

/// [`neutral_filters`] for every filter defined in any scope that is not protected: the
/// repository's `local` configuration, its `worktree` configuration (`config.worktree`, read
/// once `extensions.worktreeConfig` is set), whatever either includes, and any scope a later Git
/// adds. Before, only `git config --local` was listed, which does not read `config.worktree`, so
/// a filter there still ran.
fn repository_filters_off(
    options: &GitAcquireOptions,
    root: &Path,
    deadline: std::time::Instant,
) -> Result<Vec<(OsString, OsString)>, GitAcquireError> {
    let first = caller_config_count(options)?;
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
            OsStr::new("--includes"),
            OsStr::new("--show-scope"),
            OsStr::new("--null"),
            OsStr::new("--name-only"),
            OsStr::new("--get-regexp"),
            OsStr::new("^filter\\."),
        ],
        &[0, 1],
        1024 * 1024,
    )?;
    neutral_filters(&unprotected_filter_names(&listing), first)
}

/// The filter names, as raw bytes, in `git config --show-scope --null --name-only` output
/// (`<scope>\0<key>\0` pairs, keys `filter.<name>.<key>`) that any unprotected scope defines.
fn unprotected_filter_names(listing: &[u8]) -> Vec<Vec<u8>> {
    let mut names: Vec<Vec<u8>> = Vec::new();
    let mut records = listing.split(|&b| b == 0);
    while let (Some(scope), Some(record)) = (records.next(), records.next()) {
        if PROTECTED_SCOPES.contains(&scope) {
            continue;
        }
        if let Some(rest) = record.strip_prefix(b"filter.")
            && let Some(dot) = rest.iter().rposition(|&b| b == b'.')
            && !names.iter().any(|name| name[..] == rest[..dot])
        {
            names.push(rest[..dot].to_vec());
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

impl Runner<'_> {
    /// Run Git; accept only the listed exit codes; never buffer past `budget`.
    #[expect(
        clippy::disallowed_methods,
        reason = "the git-acquisition door: patch acquisition spawns git only here, hardened by core::git"
    )]
    fn run(
        &self,
        command: &'static str,
        args: &[&OsStr],
        ok_codes: &[i32],
        budget: usize,
    ) -> Result<Vec<u8>, GitAcquireError> {
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
        config_env: repository_filters_off(options, &root, deadline)?,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_filters_from_unprotected_scopes_are_turned_off() {
        let listing = b"system\0filter.sys.clean\0global\0filter.lfs.process\0\
            command\0filter.cmd.smudge\0local\0filter.a.b.clean\0worktree\0filter.wt.smudge\0\
            local\0filter.a.b.process\0future\0filter.new.clean\0global\0filter.wt.clean\0\
            worktree\0filter.\xff.clean\0local\0core.bare\0";
        assert_eq!(
            unprotected_filter_names(listing),
            [&b"a.b"[..], b"wt", b"new", b"\xff"]
        );
        assert_eq!(unprotected_filter_names(b""), Vec::<Vec<u8>>::new());
    }

    #[cfg(unix)]
    #[test]
    fn filter_names_reach_git_byte_for_byte_or_not_at_all() {
        use std::os::unix::ffi::OsStrExt as _;
        let env = neutral_filters(&[b"\xff".to_vec()], 2).unwrap();
        assert_eq!(env.len(), 9);
        assert_eq!(env[0].0, "GIT_CONFIG_KEY_2");
        assert_eq!(env[0].1.as_bytes(), b"filter.\xff.clean");
        assert_eq!(env[7], ("GIT_CONFIG_VALUE_5".into(), "false".into()));
        assert_eq!(env[8], ("GIT_CONFIG_COUNT".into(), "6".into()));
        assert_eq!(
            utf8_filter_key(b"lfs", "clean"),
            Ok(OsString::from("filter.lfs.clean"))
        );
        assert!(matches!(
            utf8_filter_key(b"\xff", "clean"),
            Err(GitAcquireError::InvalidSelection { .. })
        ));
    }

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
