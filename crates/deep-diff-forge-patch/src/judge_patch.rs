//! The patch a judge is given for a working tree.
//!
//! A judge that reads a patch needs the same bytes whoever makes it. A plain `git diff` is not
//! that: `diff.noprefix` drops the `a/` `b/` prefixes, `diff.relative` leaves out what lies
//! outside the current directory, an attribute can turn text into a binary patch, and rename
//! detection changes which files a change names. forge-warden's patch maker answers this for two
//! commits by diffing them in a new, empty repository that only borrows the object store, so no
//! configuration of the user, the system or the repository is read.
//!
//! [`judge_patch`] does the same for a working tree. It writes the tree of everything in the
//! working tree (`git add --all`), without touching the repository: the Git directory that is
//! used is a new, empty one in a scratch directory the caller owns, with the working tree as its
//! work tree, and the repository's own object store is only read. Then it diffs the base commit
//! against that tree the way the patch maker does. For a state that is committed with no
//! conversion, the bytes equal the patch maker's (a test holds this).
//!
//! What the tree holds is what `git add --all` holds: tracked changes, deletions and untracked
//! files, without ignored files (`.gitignore` files, the repository's `info/exclude`, which is
//! copied, and the user's ignore file), and then every file that the repository's own index
//! lists and the working tree has. A file under an ignore rule that was added with `git add -f`
//! is in the next commit; `git add --all` alone left it out of the patch (independent review of
//! the hooks, HR-M-2).
//!
//! **A nested repository is left out.** A directory in the working tree that is a Git
//! repository of its own and that the base and the index do not hold (a scratch `git init`, a
//! checkout made inside the project) is not part of what the project would commit as files.
//! `git add --all` records one that has a commit as a link to that commit and fails on one
//! that has none, which made every patch of such a tree fail (round 11 of the review of the
//! hooks). They are kept out of the staged tree and named in [`JudgePatch::nested`].
//!
//! **Staging reads no configuration.** The scratch Git directory has none, and the staging
//! commands run with the system's and the user's configuration off as well, with no attributes
//! file of the user or the system: a filter the user's configuration defines used to run on
//! every staged file and the judge was given its output (HR-L-5). One value is carried over by
//! hand: the user's `core.excludesFile`, so that what the user ignores everywhere stays out of
//! the patch.
//!
//! **Nothing under the repository's Git directory but its object store and `info/exclude`
//! shapes the bytes.** Staging in the repository's own Git directory read its
//! `info/attributes`, so a line there (`*.txt text eol=lf`) changed what the judge was given
//! although no file of the change said so (review V4, VD4-M-2). In the scratch Git directory
//! there is no such file and no configuration of the repository: no filter it defines exists,
//! so making the patch runs no program the repository chose. What does convert content is in
//! the change itself, where a judge can see it: a `.gitattributes` file in the working tree.
//! The cost is stated: where the repository's own configuration would make a commit differ
//! (`core.autocrlf`, `core.fileMode=false`, a filter), this tree is the working tree's bytes
//! and modes, not that commit's.

use crate::git_acquire::{Runner, deadline_after, path_from_bytes, repository_filters_off};
use crate::{GitAcquireError, GitAcquireOptions};
use std::ffi::{OsStr, OsString};
use std::path::Path;

/// A working tree as one patch against a base commit, in the form a judge reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgePatch {
    /// The patch: `git diff --binary --no-renames` with `a/` and `b/` prefixes, made with no
    /// configuration and no attributes. Empty when the tree equals the base.
    pub bytes: Vec<u8>,
    /// The base commit, resolved.
    pub base: String,
    /// The tree of the working state (it exists only in the scratch directory).
    pub tree: String,
    /// The nested repositories that were left out of the tree, as paths from the top level.
    pub nested: Vec<String>,
}

/// Why [`judge_patch`] made no patch. The three are kept apart because a caller answers each
/// differently: no base is nothing to judge, a working tree this cannot read truly is a change
/// that was not judged, and a failure is a failure. One error for all of them once made a
/// failure look like nothing to judge.
#[derive(Debug)]
pub enum JudgePatchError {
    /// `base` names no commit: a repository with no commit yet, or a name that is not one.
    NoBase,
    /// The index marks paths skip-worktree (a sparse checkout, or `git update-index
    /// --skip-worktree`). Their files are absent on purpose, and a tree staged from the working
    /// tree would show every one of them as deleted (review V4 round 2, VD4R2-M-1). Refused
    /// until the scratch index can carry those marks.
    SkipWorktree,
    /// The index has unmerged paths (a merge, a rebase or a cherry-pick stopped on a conflict):
    /// it has no tree, so there is nothing a commit could record. Git's own word for it,
    /// "error building trees", does not say what a person has to do (sixth independent
    /// review of the hooks, HR6-L-5).
    Unmerged,
    /// Git failed or ran out of time or budget, or a file this needs could not be prepared.
    Failed(GitAcquireError),
}

impl std::fmt::Display for JudgePatchError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoBase => out.write_str("the base names no commit"),
            Self::SkipWorktree => out.write_str(
                "the index marks paths skip-worktree (a sparse checkout): the working tree is not the whole change, and no patch is made from it",
            ),
            Self::Unmerged => out.write_str(
                "the index has unmerged paths (a merge, a rebase or a cherry-pick stopped on a conflict), so it holds nothing a commit could record: resolve them and `git add` the files",
            ),
            Self::Failed(error) => write!(out, "{error}"),
        }
    }
}

impl std::error::Error for JudgePatchError {}

impl From<GitAcquireError> for JudgePatchError {
    fn from(error: GitAcquireError) -> Self {
        Self::Failed(error)
    }
}

/// The most bytes of `git ls-files -t` that are read to look for skip-worktree marks.
const INDEX_LISTING_CAP: usize = 64 * 1024 * 1024;

/// The diff options of forge-warden's patch maker (`scripts/make_patch.sh`), in its order.
const DIFF_ARGS: &[&str] = &[
    "diff",
    "--no-ext-diff",
    "--no-textconv",
    "--no-color",
    "--no-relative",
    "--binary",
    "--no-renames",
    "--src-prefix=a/",
    "--dst-prefix=b/",
];

/// The environment of what runs outside the reviewed repository (making the scratch repository,
/// the diff itself): no system or user configuration, no system attributes, and no
/// configuration passed down by a parent Git (`git -c …` and `GIT_CONFIG_COUNT`).
const ISOLATED: &[(&str, &str)] = &[
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_ATTR_NOSYSTEM", "1"),
    ("GIT_CONFIG_COUNT", "0"),
    ("GIT_CONFIG_PARAMETERS", ""),
];

fn refused(detail: impl Into<String>) -> GitAcquireError {
    GitAcquireError::InvalidSelection {
        detail: detail.into(),
    }
}

/// Git's one-line answer as text, without its newline.
fn line(mut bytes: Vec<u8>) -> Result<String, GitAcquireError> {
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    String::from_utf8(bytes).map_err(|_| refused("Git's answer is not UTF-8"))
}

/// A path as one line of an `objects/info/alternates` file. The file is read line by line, a
/// line that starts with `#` is a comment and one that starts with `"` is quoted, so a path that
/// would be read as something else is refused instead of written.
fn alternate_line(path: &Path) -> Result<&str, GitAcquireError> {
    let text = path
        .to_str()
        .ok_or_else(|| refused("an object directory's path is not UTF-8"))?;
    if text.contains('\n') || text.starts_with('#') || text.starts_with('"') {
        return Err(refused(
            "an object directory's path cannot be written to an alternates file",
        ));
    }
    Ok(text)
}

fn pair(name: &str, value: impl Into<OsString>) -> (OsString, OsString) {
    (OsString::from(name), value.into())
}

/// What a scratch repository borrows from the reviewed one.
struct Store {
    /// The repository's object directory.
    objects: std::path::PathBuf,
    /// The repository's `info/exclude`: paths it ignores without a `.gitignore` saying so.
    exclude: std::path::PathBuf,
    /// `sha1` or `sha256`.
    object_format: String,
}

/// The [`Store`] of the repository `inside` runs in.
fn store(inside: &Runner) -> Result<Store, GitAcquireError> {
    Ok(Store {
        objects: git_path(inside, "objects")?,
        exclude: git_path(inside, "info/exclude")?,
        object_format: line(inside.run(
            "git rev-parse",
            &[OsStr::new("rev-parse"), OsStr::new("--show-object-format")],
            &[0],
            4096,
        )?)?,
    })
}

/// What is read from the repository before anything is made: where it is, how to read it
/// without running its filters, and what it is compared with.
struct Source {
    root: std::path::PathBuf,
    /// The base commit, resolved.
    base: String,
    store: Store,
    /// The paths the repository's own index lists, as Git wrote them.
    indexed: Vec<Vec<u8>>,
    /// The user's own ignore file (`core.excludesFile` in the user's configuration), when it is
    /// set to an absolute path.
    user_excludes: Option<std::path::PathBuf>,
}

/// Git's answer to `args`, a path on one line, without its newline.
fn path_answer(
    runner: &Runner,
    command: &'static str,
    args: &[&OsStr],
    ok_codes: &[i32],
) -> Result<std::path::PathBuf, GitAcquireError> {
    let mut bytes = runner.run(command, args, ok_codes, 64 * 1024)?;
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    Ok(path_from_bytes(bytes))
}

/// Where Git keeps `name` for the repository `inside` runs in, as an absolute path.
fn git_path(inside: &Runner, name: &str) -> Result<std::path::PathBuf, GitAcquireError> {
    path_answer(
        inside,
        "git rev-parse",
        &[
            OsStr::new("rev-parse"),
            OsStr::new("--path-format=absolute"),
            OsStr::new("--git-path"),
            OsStr::new(name),
        ],
        &[0],
    )
}

/// The paths the index of the repository `inside` runs in lists, or
/// [`JudgePatchError::SkipWorktree`] when it marks any of them skip-worktree.
fn indexed_paths(inside: &Runner) -> Result<Vec<Vec<u8>>, JudgePatchError> {
    // `-t` tags each path; `S` is skip-worktree. `--sparse` keeps a sparse index as it is.
    let listing = inside.run(
        "git ls-files",
        &[
            OsStr::new("ls-files"),
            OsStr::new("-t"),
            OsStr::new("-z"),
            OsStr::new("--sparse"),
        ],
        &[0],
        INDEX_LISTING_CAP,
    )?;
    if listing
        .split(|byte| *byte == 0)
        .any(|entry| entry.starts_with(b"S "))
    {
        return Err(JudgePatchError::SkipWorktree);
    }
    // Each entry is a tag, a space and the path.
    Ok(listing
        .split(|byte| *byte == 0)
        .filter_map(|entry| entry.get(2..))
        .filter(|path| !path.is_empty())
        .map(<[u8]>::to_vec)
        .collect())
}

fn source(
    repo: &Path,
    base: &str,
    options: &GitAcquireOptions,
    deadline: std::time::Instant,
) -> Result<Source, JudgePatchError> {
    let probe = Runner {
        options,
        cwd: repo.to_path_buf(),
        deadline,
        config_env: Vec::new(),
    };
    let root = path_answer(
        &probe,
        "git rev-parse",
        &[OsStr::new("rev-parse"), OsStr::new("--show-toplevel")],
        &[0],
    )?;
    let filters = repository_filters_off(options, &root, deadline)?;
    let inside = Runner {
        options,
        cwd: root.clone(),
        deadline,
        // The repository's own filters stay off for everything that is read from it.
        config_env: filters,
    };
    let base = match inside.commit_id(base) {
        Ok(Some(id)) => id,
        Ok(None) | Err(GitAcquireError::InvalidSelection { .. }) => {
            return Err(JudgePatchError::NoBase);
        }
        Err(other) => return Err(other.into()),
    };
    let indexed = indexed_paths(&inside)?;
    // `--global` reads only the user's files; exit 1 says the key is not set.
    let user_excludes = match path_answer(
        &probe,
        "git config",
        &[
            OsStr::new("config"),
            OsStr::new("--global"),
            OsStr::new("--path"),
            OsStr::new("--get"),
            OsStr::new("core.excludesFile"),
        ],
        &[0, 1],
    ) {
        Ok(path) => Some(path).filter(|path| path.is_absolute()),
        // Git has no user configuration to ask (no home directory is set): nothing of the
        // user's is carried over, and that is no reason to make no patch.
        Err(GitAcquireError::CommandFailed { .. }) => None,
        Err(other) => return Err(other.into()),
    };
    Ok(Source {
        root,
        base,
        store: store(&inside)?,
        indexed,
        user_excludes,
    })
}

/// The most bytes of the repository's `info/exclude` that are copied.
const EXCLUDE_CAP: u64 = 1024 * 1024;

/// The environment of a command that runs outside the reviewed repository.
fn isolated() -> Vec<(OsString, OsString)> {
    ISOLATED.iter().map(|(k, v)| pair(k, *v)).collect()
}

/// A new, empty bare repository in `scratch` that borrows the reviewed repository's object
/// store and ignores what its `info/exclude` ignores. Everything this module writes goes into
/// it. It has no configuration but what `git init` writes, no attributes and no hooks.
fn empty_repository(
    from: &Store,
    scratch: &Path,
    options: &GitAcquireOptions,
    deadline: std::time::Instant,
) -> Result<std::path::PathBuf, GitAcquireError> {
    use std::io::Read as _;
    let repository = scratch.join("judge.git");
    let outside = Runner {
        options,
        cwd: scratch.to_path_buf(),
        deadline,
        config_env: isolated(),
    };
    let object_format = OsString::from(format!("--object-format={}", from.object_format));
    outside.run(
        "git init",
        &[
            OsStr::new("init"),
            OsStr::new("--quiet"),
            OsStr::new("--template="),
            object_format.as_os_str(),
            OsStr::new("--bare"),
            repository.as_os_str(),
        ],
        &[0],
        4096,
    )?;
    std::fs::write(
        repository.join("objects/info/alternates"),
        format!("{}\n", alternate_line(&from.objects)?),
    )
    .map_err(|e| refused(format!("cannot write the alternates file: {e}")))?;
    // What the repository ignores without a `.gitignore` saying so stays ignored. A missing
    // file is no rule; one that cannot be read, or is over the cap, is refused rather than
    // dropped, since dropping it would put files in the patch that a commit leaves out.
    match std::fs::File::open(&from.exclude) {
        Ok(file) => {
            let mut rules = Vec::new();
            file.take(EXCLUDE_CAP + 1)
                .read_to_end(&mut rules)
                .map_err(|e| refused(format!("cannot read the repository's info/exclude: {e}")))?;
            if rules.len() as u64 > EXCLUDE_CAP {
                return Err(refused("the repository's info/exclude is over 1 MiB"));
            }
            std::fs::create_dir_all(repository.join("info"))
                .and_then(|()| std::fs::write(repository.join("info/exclude"), rules))
                .map_err(|e| refused(format!("cannot prepare the scratch directory: {e}")))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(refused(format!(
                "cannot read the repository's info/exclude: {error}"
            )));
        }
    }
    Ok(repository)
}

/// The environment of the commands that stage the working tree: the scratch repository as the
/// Git directory, the reviewed repository's top level as the work tree, and no configuration.
/// The system's and the user's files are off ([`ISOLATED`]), so no filter either defines
/// exists; the user's attributes file is off; and the one value kept is the user's own ignore
/// file, passed by hand.
fn staging_env(from: &Source, repository: &Path) -> Vec<(OsString, OsString)> {
    let mut config = vec![("core.attributesFile", OsString::from("/dev/null"))];
    if let Some(excludes) = &from.user_excludes {
        config.push(("core.excludesFile", excludes.clone().into_os_string()));
    }
    let mut env: Vec<(OsString, OsString)> = ISOLATED
        .iter()
        .filter(|(name, _)| *name != "GIT_CONFIG_COUNT")
        .map(|(k, v)| pair(k, *v))
        .collect();
    for (at, (key, value)) in config.iter().enumerate() {
        env.push(pair(&format!("GIT_CONFIG_KEY_{at}"), *key));
        env.push(pair(&format!("GIT_CONFIG_VALUE_{at}"), value.clone()));
    }
    env.push(pair("GIT_CONFIG_COUNT", config.len().to_string()));
    env.extend([
        pair("GIT_DIR", repository.to_path_buf()),
        pair("GIT_WORK_TREE", from.root.clone()),
    ]);
    env
}

/// The paths the repository's own index lists that the scratch index does not hold although the
/// working tree has a file there: what `git add --all` left out because an ignore rule names
/// it. A path the working tree does not have is not one of them: the judged state is the
/// working tree.
fn left_out(from: &Source, staged: &[u8]) -> Vec<Vec<u8>> {
    let staged: std::collections::BTreeSet<&[u8]> = staged
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .collect();
    from.indexed
        .iter()
        .filter(|path| !staged.contains(path.as_slice()))
        .filter(|path| {
            std::fs::symlink_metadata(from.root.join(path_from_bytes((*path).clone()))).is_ok()
        })
        .cloned()
        .collect()
}

/// The tree of the working state, written into `repository` (the scratch one), with the
/// reviewed repository's top level as its work tree: its index and the objects it writes are in
/// the scratch directory, and nothing of the reviewed repository's Git directory is read but
/// its object store. `scratch` takes the list of paths that are staged by name.
fn working_tree(
    from: &Source,
    scratch: &Path,
    repository: &Path,
    options: &GitAcquireOptions,
    deadline: std::time::Instant,
) -> Result<(String, Vec<String>), GitAcquireError> {
    let staging = Runner {
        options,
        cwd: from.root.clone(),
        deadline,
        config_env: staging_env(from, repository),
    };
    staging.run(
        "git read-tree",
        &[OsStr::new("read-tree"), OsStr::new(&from.base)],
        &[0],
        4096,
    )?;
    let nested = leave_nested_out(&staging, repository, &from.indexed)?;
    staging.run(
        "git add",
        &[
            OsStr::new("add"),
            OsStr::new("--all"),
            OsStr::new("--"),
            OsStr::new("."),
        ],
        &[0],
        64 * 1024,
    )?;
    // What the repository's index holds is in the next commit, whatever an ignore rule says.
    let staged = staging.run(
        "git ls-files",
        &[OsStr::new("ls-files"), OsStr::new("-z")],
        &[0],
        INDEX_LISTING_CAP,
    )?;
    let forced = left_out(from, &staged);
    if !forced.is_empty() {
        let list = scratch.join("indexed-paths");
        let mut bytes = Vec::new();
        for path in &forced {
            bytes.extend_from_slice(path);
            bytes.push(0);
        }
        std::fs::write(&list, bytes)
            .map_err(|e| refused(format!("cannot prepare the scratch directory: {e}")))?;
        let mut from_file = OsString::from("--pathspec-from-file=");
        from_file.push(list.as_os_str());
        staging.run(
            "git add",
            &[
                OsStr::new("--literal-pathspecs"),
                OsStr::new("add"),
                OsStr::new("--force"),
                from_file.as_os_str(),
                OsStr::new("--pathspec-file-nul"),
            ],
            &[0],
            64 * 1024,
        )?;
    }
    let tree = line(staging.run("git write-tree", &[OsStr::new("write-tree")], &[0], 4096)?)?;
    Ok((tree, nested))
}

/// A path as an ignore rule that names exactly it, as a directory, from the top level: the
/// characters a rule reads as a pattern are escaped. `None` for a path a rule cannot name (one
/// that holds a newline).
fn exact_rule(path: &[u8]) -> Option<Vec<u8>> {
    if path.contains(&b'\n') || path.is_empty() {
        return None;
    }
    let mut rule = vec![b'/'];
    for byte in path {
        if matches!(byte, b'\\' | b'*' | b'?' | b'[') {
            rule.push(b'\\');
        }
        rule.push(*byte);
    }
    // A space before the end of a rule is dropped unless it is escaped; the rule ends in `/`.
    rule.extend_from_slice(b"/\n");
    Some(rule)
}

/// Keep the nested repositories of the working tree out of what `staging` adds, and name
/// them. Git lists a directory that is a repository of its own, and that the index does not
/// hold, as one untracked entry ending in `/`; each gets a rule in the scratch repository's
/// `info/exclude`. One the base holds (a submodule) is not untracked and stays, and so does
/// one the repository's own index lists (`indexed`).
fn leave_nested_out(
    staging: &Runner,
    repository: &Path,
    indexed: &[Vec<u8>],
) -> Result<Vec<String>, GitAcquireError> {
    use std::io::Write as _;
    let others = staging.run(
        "git ls-files",
        &[
            OsStr::new("ls-files"),
            OsStr::new("--others"),
            OsStr::new("--exclude-standard"),
            OsStr::new("-z"),
        ],
        &[0],
        INDEX_LISTING_CAP,
    )?;
    let nested: Vec<&[u8]> = others
        .split(|byte| *byte == 0)
        .filter_map(|entry| entry.strip_suffix(b"/"))
        // One the repository's own index holds is the project's: it is staged like any path.
        .filter(|path| !indexed.iter().any(|held| held == path))
        .collect();
    if nested.is_empty() {
        return Ok(Vec::new());
    }
    let mut rules = Vec::new();
    for path in &nested {
        rules.extend(exact_rule(path).ok_or_else(|| {
            refused("a nested repository's path cannot be written as an ignore rule")
        })?);
    }
    std::fs::create_dir_all(repository.join("info"))
        .and_then(|()| {
            std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(repository.join("info/exclude"))
        })
        // a copied `info/exclude` may not end in a newline
        .and_then(|mut file| {
            file.write_all(b"\n")?;
            file.write_all(&rules)
        })
        .map_err(|e| refused(format!("cannot prepare the scratch directory: {e}")))?;
    Ok(nested
        .iter()
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect())
}

/// The diff of the base commit against `tree`, made in the scratch repository from outside any
/// work tree: no configuration of the user, the system or the reviewed repository is read for
/// it.
fn diff_outside(
    base: &str,
    tree: &str,
    scratch: &Path,
    repository: &Path,
    options: &GitAcquireOptions,
    deadline: std::time::Instant,
) -> Result<Vec<u8>, GitAcquireError> {
    let outside = Runner {
        options,
        cwd: scratch.to_path_buf(),
        deadline,
        config_env: isolated(),
    };
    let mut git_dir = OsString::from("--git-dir=");
    git_dir.push(repository.as_os_str());
    // Attributes come from the empty tree, not from any work tree: a `.gitattributes` saying
    // `-diff` or `binary` would otherwise turn added text into a binary patch a judge cannot read.
    let empty_tree = line(outside.run(
        "git hash-object",
        &[
            git_dir.as_os_str(),
            OsStr::new("hash-object"),
            OsStr::new("-t"),
            OsStr::new("tree"),
            OsStr::new("--stdin"),
        ],
        &[0],
        4096,
    )?)?;
    let attr_source = OsString::from(format!("--attr-source={empty_tree}"));
    let mut args: Vec<&OsStr> = vec![
        git_dir.as_os_str(),
        attr_source.as_os_str(),
        OsStr::new("-c"),
        OsStr::new("core.attributesFile=/dev/null"),
        OsStr::new("-c"),
        OsStr::new("core.quotePath=true"),
    ];
    args.extend(DIFF_ARGS.iter().map(OsStr::new));
    // The empty tree stands for "no commit to compare with".
    args.push(OsStr::new(if base.is_empty() {
        empty_tree.as_str()
    } else {
        base
    }));
    args.push(OsStr::new(tree));
    outside.run("git diff", &args, &[0], options.byte_budget)
}

/// The working tree of the repository containing `repo`, as one patch against `base`.
///
/// `scratch` is an existing, empty directory the caller owns and removes afterwards; the bare
/// repository this needs, with its index and the objects it writes, is made inside it. Nothing
/// in the reviewed repository is written: not its index, not its object store, not its
/// configuration.
///
/// # Errors
/// [`JudgePatchError`]: `base` names no commit, the index marks paths skip-worktree, or Git
/// failed, ran out of time or budget, or a path cannot be used. Nothing partial is returned.
pub fn judge_patch(
    repo: &Path,
    base: &str,
    scratch: &Path,
    options: &GitAcquireOptions,
) -> Result<JudgePatch, JudgePatchError> {
    let deadline = deadline_after(options.time_limit);
    let from = source(repo, base, options, deadline)?;
    let repository = empty_repository(&from.store, scratch, options, deadline)?;
    let (tree, nested) = working_tree(&from, scratch, &repository, options, deadline)?;
    let bytes = diff_outside(&from.base, &tree, scratch, &repository, options, deadline)?;
    Ok(JudgePatch {
        bytes,
        base: from.base,
        tree,
        nested,
    })
}

/// The staged change of the working tree `repo` is in, as a patch against the commit `base`:
/// what the index holds, which is what a plain `git commit` would record, whatever the
/// working tree holds now. Empty when the index equals `base`'s tree.
///
/// The entries of the repository's index are read (`git ls-files --stage`) and written into
/// an index of the scratch repository, which borrows the object store; the tree is written
/// there and diffed like a commit's. Nothing in the reviewed repository is written.
///
/// A stop judged the working tree and the commits. What was staged and then taken out of the
/// working tree again without `git add` was in neither, and the next plain `git commit`
/// recorded it (fourth independent review of the hooks, HR4-H-1).
///
/// # Errors
/// As [`judge_patch`]; an index with unmerged paths has no tree, which is
/// [`JudgePatchError::Unmerged`].
pub fn staged_patch(
    repo: &Path,
    base: &str,
    scratch: &Path,
    options: &GitAcquireOptions,
) -> Result<Vec<u8>, JudgePatchError> {
    let deadline = deadline_after(options.time_limit);
    let from = source(repo, base, options, deadline)?;
    let repository = empty_repository(&from.store, scratch, options, deadline)?;
    let inside = Runner {
        options,
        cwd: from.root.clone(),
        deadline,
        config_env: Vec::new(),
    };
    let entries = inside.run(
        "git ls-files",
        &[
            OsStr::new("ls-files"),
            OsStr::new("--stage"),
            OsStr::new("-z"),
        ],
        &[0],
        INDEX_LISTING_CAP,
    )?;
    // An entry of a stage other than 0 is one side of a conflict: `<mode> <id> <stage>\t<path>`.
    let unmerged = entries.split(|byte| *byte == 0).any(|entry| {
        entry
            .iter()
            .position(|byte| *byte == b'\t')
            .and_then(|tab| tab.checked_sub(1))
            .is_some_and(|stage| entry[stage] != b'0')
    });
    if unmerged {
        return Err(JudgePatchError::Unmerged);
    }
    // An entry made with `git add -N` says that a path will be added and holds no content:
    // a commit leaves it out, so it is no staged content. It was read as a staged empty
    // file (fifth independent review of the hooks, HR5-L-1). Git shows such a path, and
    // nothing else, as added when the index is compared with the working tree.
    let intended = inside.run(
        "git diff",
        &[
            OsStr::new("diff"),
            OsStr::new("--no-ext-diff"),
            OsStr::new("--no-textconv"),
            OsStr::new("--no-renames"),
            OsStr::new("--name-only"),
            OsStr::new("--diff-filter=A"),
            OsStr::new("-z"),
        ],
        &[0],
        INDEX_LISTING_CAP,
    )?;
    let intended: std::collections::BTreeSet<&[u8]> = intended
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .collect();
    let entries: Vec<u8> = entries
        .split_inclusive(|byte| *byte == 0)
        .filter(|entry| {
            let path = entry
                .iter()
                .position(|byte| *byte == b'\t')
                .map_or(&entry[..0], |tab| &entry[tab + 1..entry.len() - 1]);
            !intended.contains(path)
        })
        .flatten()
        .copied()
        .collect();
    let list = scratch.join("index-entries");
    std::fs::write(&list, entries)
        .map_err(|e| refused(format!("cannot prepare the scratch directory: {e}")))?;
    let staging = Runner {
        options,
        cwd: from.root.clone(),
        deadline,
        config_env: staging_env(&from, &repository),
    };
    staging.run_reading(
        "git update-index",
        &[
            OsStr::new("update-index"),
            OsStr::new("-z"),
            OsStr::new("--index-info"),
        ],
        &[0],
        4096,
        &list,
    )?;
    let tree = line(staging.run("git write-tree", &[OsStr::new("write-tree")], &[0], 4096)?)?;
    Ok(diff_outside(
        &from.base,
        &tree,
        scratch,
        &repository,
        options,
        deadline,
    )?)
}

/// The patch from the commit `from` to the commit `to`, in the form and with the bytes
/// [`judge_patch`] gives for a working tree whose state is `to`'s tree: diffed in a new, empty
/// repository in `scratch` that only borrows the object store, with no configuration and no
/// attributes. `from` is `None` for a commit with nothing to compare with (no history in
/// common): the patch then adds everything `to` holds.
///
/// # Errors
/// [`JudgePatchError::NoBase`] when `from` or `to` names no commit; otherwise as
/// [`judge_patch`].
pub fn commit_patch(
    repo: &Path,
    from: Option<&str>,
    to: &str,
    scratch: &Path,
    options: &GitAcquireOptions,
) -> Result<Vec<u8>, JudgePatchError> {
    let deadline = deadline_after(options.time_limit);
    let inside = Runner {
        options,
        cwd: repo.to_path_buf(),
        deadline,
        config_env: Vec::new(),
    };
    let resolved = |rev: &str| match inside.commit_id(rev) {
        Ok(Some(id)) => Ok(id),
        Ok(None) | Err(GitAcquireError::InvalidSelection { .. }) => Err(JudgePatchError::NoBase),
        Err(other) => Err(other.into()),
    };
    let from = from.map(resolved).transpose()?.unwrap_or_default();
    let to = resolved(to)?;
    let repository = empty_repository(&store(&inside)?, scratch, options, deadline)?;
    Ok(diff_outside(
        &from,
        &to,
        scratch,
        &repository,
        options,
        deadline,
    )?)
}

/// The most bytes of a listing of refs that are read.
const REF_LISTING_CAP: usize = 4 * 1024 * 1024;

/// The refs of a repository that can name a session's commits, each with the commit it leads
/// to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RefTips {
    /// `(name, commit)` in the order of the names. A tag is followed to the commit it tags; a
    /// ref that leads to no commit is left out.
    pub tips: Vec<(String, String)>,
    /// The repository has more refs than were asked for: `tips` is the first of them.
    pub partial: bool,
}

/// The branches, remote-tracking refs, tags and the stash of the repository containing `repo`
/// (`refs/heads`, `refs/remotes`, `refs/tags`, `refs/stash`), at most `limit` of them.
///
/// # Errors
/// Any [`GitAcquireError`] from running Git, or a listing that cannot be read.
pub fn ref_tips(
    repo: &Path,
    limit: usize,
    options: &GitAcquireOptions,
) -> Result<RefTips, GitAcquireError> {
    let runner = Runner {
        options,
        cwd: repo.to_path_buf(),
        deadline: deadline_after(options.time_limit),
        config_env: Vec::new(),
    };
    let count = OsString::from(format!("--count={}", limit.saturating_add(1)));
    let listing = runner.run(
        "git for-each-ref",
        &[
            OsStr::new("for-each-ref"),
            count.as_os_str(),
            OsStr::new(
                "--format=%(refname)%00%(objecttype)%00%(objectname)%00%(*objecttype)%00%(*objectname)",
            ),
            OsStr::new("refs/heads"),
            OsStr::new("refs/remotes"),
            OsStr::new("refs/tags"),
            OsStr::new("refs/stash"),
        ],
        &[0],
        REF_LISTING_CAP,
    )?;
    let text = String::from_utf8(listing).map_err(|_| refused("a ref's name is not UTF-8"))?;
    let mut tips = Vec::new();
    let mut seen = 0usize;
    for record in text.lines() {
        let fields: Vec<&str> = record.split('\0').collect();
        let [name, kind, id, tagged_kind, tagged] = fields[..] else {
            return Err(refused("git for-each-ref wrote a line that cannot be read"));
        };
        seen += 1;
        if seen > limit {
            break;
        }
        match (kind, tagged_kind) {
            ("commit", _) => tips.push((name.to_owned(), id.to_owned())),
            (_, "commit") => tips.push((name.to_owned(), tagged.to_owned())),
            _ => {}
        }
    }
    Ok(RefTips {
        tips,
        partial: seen > limit,
    })
}

/// Whether `tip` leads to a commit that none of the commits `known` leads to. A commit in
/// `known` that the repository no longer has is passed over.
///
/// # Errors
/// Any [`GitAcquireError`] from running Git; `tip` not naming a commit that can be read.
pub fn has_commits_beyond(
    repo: &Path,
    tip: &str,
    known: &[String],
    options: &GitAcquireOptions,
) -> Result<bool, GitAcquireError> {
    let runner = Runner {
        options,
        cwd: repo.to_path_buf(),
        deadline: deadline_after(options.time_limit),
        config_env: Vec::new(),
    };
    let tip = runner
        .commit_id(tip)?
        .ok_or_else(|| refused(format!("`{tip}` does not name a commit")))?;
    let mut args: Vec<&OsStr> = vec![
        OsStr::new("rev-list"),
        OsStr::new("--max-count=1"),
        OsStr::new("--ignore-missing"),
        OsStr::new(&tip),
        OsStr::new("--not"),
    ];
    // Only full commit ids are passed on: nothing here can be read as an option.
    args.extend(
        known
            .iter()
            .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(OsStr::new),
    );
    Ok(!runner.run("git rev-list", &args, &[0], 4096)?.is_empty())
}

/// One working tree of a repository, as `git worktree list` gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    /// Its top level (for a bare repository's own entry, the repository).
    pub path: std::path::PathBuf,
    /// The commit its `HEAD` names, when it names one.
    pub head: Option<String>,
    /// Git can be asked there: the directory exists and is still a working tree (it holds a
    /// `.git`), or is the bare repository itself. Not so for a working tree that was removed
    /// by hand, with or without another directory put at its path, until
    /// `git worktree prune` forgets it.
    pub usable: bool,
    /// The entry of a bare repository itself: it has refs and reflogs and no working tree.
    pub bare: bool,
}

/// The working trees of the repository containing `repo`, the main one first.
///
/// # Errors
/// Any [`GitAcquireError`] from running Git.
pub fn worktrees(
    repo: &Path,
    options: &GitAcquireOptions,
) -> Result<Vec<Worktree>, GitAcquireError> {
    use std::os::unix::ffi::OsStrExt as _;
    let runner = Runner {
        options,
        cwd: repo.to_path_buf(),
        deadline: deadline_after(options.time_limit),
        config_env: Vec::new(),
    };
    let listing = runner.run(
        "git worktree list",
        &[
            OsStr::new("worktree"),
            OsStr::new("list"),
            OsStr::new("--porcelain"),
            OsStr::new("-z"),
        ],
        &[0],
        NAMED_LISTING_CAP,
    )?;
    let mut found: Vec<Worktree> = Vec::new();
    for record in listing.split(|byte| *byte == 0) {
        if let Some(path) = record.strip_prefix(b"worktree ") {
            let path = std::path::PathBuf::from(OsStr::from_bytes(path));
            let usable = path.is_dir() && path.join(".git").exists();
            found.push(Worktree {
                path,
                head: None,
                usable,
                bare: false,
            });
        } else if let Some(last) = found.last_mut() {
            if record == b"bare" {
                last.bare = true;
                last.usable = last.path.is_dir();
            } else if let Some(id) = record.strip_prefix(b"HEAD ")
                && let Ok(id) = std::str::from_utf8(id)
                && is_object_id(id)
                && id.bytes().any(|b| b != b'0')
            {
                last.head = Some(id.to_owned());
            }
        }
    }
    Ok(found)
}

/// Whether the working tree at `tree` has changes that are not committed: staged, unstaged or
/// untracked. Asked with `git status`, told not to refresh the index, so nothing is written.
///
/// # Errors
/// Any [`GitAcquireError`] from running Git; a status over a mebibyte is one (and is a tree
/// with changes).
pub fn has_uncommitted_changes(
    tree: &Path,
    options: &GitAcquireOptions,
) -> Result<bool, GitAcquireError> {
    let runner = Runner {
        options,
        cwd: tree.to_path_buf(),
        deadline: deadline_after(options.time_limit),
        config_env: Vec::new(),
    };
    let status = runner.run(
        "git status",
        &[
            OsStr::new("--no-optional-locks"),
            OsStr::new("status"),
            OsStr::new("--porcelain=v1"),
            OsStr::new("-z"),
            OsStr::new("--untracked-files=normal"),
        ],
        &[0],
        1024 * 1024,
    )?;
    Ok(!status.is_empty())
}

/// Which refs [`named_commits`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Named {
    /// Every ref of the repository, the remote-tracking ones included: what is known when a
    /// session begins.
    Everything,
    /// The refs a session's own work is under: `refs/heads`, `refs/tags` and `refs/stash`. A
    /// commit that only a remote-tracking ref names was fetched, not made here.
    Own,
}

/// The most bytes of a listing of refs or of a reflog that are read: room for a million ids.
const NAMED_LISTING_CAP: usize = 192 * 1024 * 1024;
/// The most working trees of one repository whose reflogs [`named_commits`] reads; a
/// repository with more is refused.
const WORKTREES_CAP: usize = 256;
/// The most commit ids [`named_commits`] gives; a repository with more is refused.
pub const NAMED_COMMITS_CAP: usize = 1_000_000;

/// A full object id as Git writes one in hex.
fn is_object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The id of every commit that something in the repository containing `repo` names: its refs
/// (all of them, or the session's own kinds: [`Named`]), a tag being followed to the commit
/// it tags; the `HEAD` of every working tree (`git worktree list`); and every entry of the
/// reflog of each working tree's `HEAD` and of the reflog of `refs/stash`. Ids only, each
/// once, sorted.
///
/// No name of a ref is read, so a name that is not UTF-8 cannot matter, and nothing is cut
/// off after some number of refs: a list of the first 4,096 refs by name left out the stash
/// and the tags of a repository with many branches (third independent review of the hooks,
/// HR3-H-2 and HR3-L-4). The reflogs name what no ref names any more: a commit made on a
/// detached `HEAD`, and the older entries of the stash.
///
/// # Errors
/// Any [`GitAcquireError`] from running Git; a listing that cannot be read; more than
/// [`NAMED_COMMITS_CAP`] commits.
pub fn named_commits(
    repo: &Path,
    which: Named,
    options: &GitAcquireOptions,
) -> Result<Vec<String>, GitAcquireError> {
    let runner = Runner {
        options,
        cwd: repo.to_path_buf(),
        deadline: deadline_after(options.time_limit),
        config_env: Vec::new(),
    };
    let mut found = std::collections::BTreeSet::new();
    let mut refs: Vec<&OsStr> = vec![
        OsStr::new("for-each-ref"),
        OsStr::new("--format=%(objecttype) %(objectname) %(*objecttype) %(*objectname)"),
    ];
    if which == Named::Own {
        refs.extend([
            OsStr::new("refs/heads"),
            OsStr::new("refs/tags"),
            OsStr::new("refs/stash"),
        ]);
    }
    let listing = runner.run("git for-each-ref", &refs, &[0], NAMED_LISTING_CAP)?;
    for record in String::from_utf8_lossy(&listing).lines() {
        // `commit ID`, or for a tag `tag ID commit ID` with the commit it leads to.
        let fields: Vec<&str> = record.split(' ').collect();
        let (["commit", commit, ..] | [_, _, "commit", commit]) = fields[..] else {
            continue;
        };
        if !is_object_id(commit) {
            return Err(refused("git for-each-ref wrote a line that cannot be read"));
        }
        found.insert(commit.to_owned());
    }
    // Every working tree of the repository: its `HEAD`, and what the reflog of that `HEAD`
    // names. Each working tree has a `HEAD` and a reflog of its own, so each is asked where it
    // stands. One whose directory is gone, or is no working tree any more, is passed over:
    // [`worktrees`] says which, for the caller to name.
    let places = worktrees(repo, options)?;
    if places.len() > WORKTREES_CAP {
        return Err(refused(
            "the repository has more working trees than a hook reads the history of (`git worktree prune` removes the ones that are gone)",
        ));
    }
    // The reflog is read with `rev-list`, which prints ids and nothing else: `git log` adds
    // what the user's configuration asks for (`log.showSignature`), and a line that was no id
    // made every session of such a user one that could not be recorded (fourth independent
    // review of the hooks, HR4-L-1).
    let reflog = |from: &Runner<'_>, walked: &str| -> Result<Vec<String>, GitAcquireError> {
        // Nothing to walk from where the name leads to no commit.
        if from.commit_id(walked)?.is_none() {
            return Ok(Vec::new());
        }
        let entries = from.run(
            "git rev-list",
            &[
                OsStr::new("rev-list"),
                OsStr::new("--walk-reflogs"),
                OsStr::new("--end-of-options"),
                OsStr::new(walked),
            ],
            &[0],
            NAMED_LISTING_CAP,
        )?;
        let text = String::from_utf8_lossy(&entries);
        if !text.lines().all(is_object_id) {
            return Err(refused("git rev-list wrote a line that cannot be read"));
        }
        Ok(text.lines().map(str::to_owned).collect())
    };
    found.extend(reflog(&runner, "refs/stash")?);
    for place in places {
        found.extend(place.head);
        if !place.usable {
            continue;
        }
        let there = Runner {
            options,
            cwd: place.path,
            deadline: runner.deadline,
            config_env: Vec::new(),
        };
        found.extend(reflog(&there, "HEAD")?);
    }
    if found.len() > NAMED_COMMITS_CAP {
        return Err(refused(
            "the repository names more than 1,000,000 commits by its refs and reflogs",
        ));
    }
    Ok(found.into_iter().collect())
}

/// One commit with its first parent (`None` for a commit that has no parent).
pub type CommitAndParent = (String, Option<String>);

/// The commits that `sources` lead to and none of `known` leads to, parents before children
/// (the oldest first), each with its first parent. `list` is a file this writes and Git reads
/// (in a scratch directory the caller owns): a list of ids is not put on a command line. An
/// id in `known` that the repository no longer has is passed over.
///
/// # Errors
/// Any [`GitAcquireError`] from running Git; an id that is not one; a list that cannot be
/// written.
pub fn commits_beyond(
    repo: &Path,
    sources: &[String],
    known: &[String],
    list: &Path,
    options: &GitAcquireOptions,
) -> Result<Vec<CommitAndParent>, GitAcquireError> {
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    if !sources.iter().chain(known).all(|id| is_object_id(id)) {
        return Err(refused("a commit id in the list is not one"));
    }
    let mut text = String::with_capacity((sources.len() + known.len()) * 66);
    for id in known {
        text.push('^');
        text.push_str(id);
        text.push('\n');
    }
    for id in sources {
        text.push_str(id);
        text.push('\n');
    }
    std::fs::write(list, text)
        .map_err(|e| refused(format!("cannot prepare the scratch directory: {e}")))?;
    let runner = Runner {
        options,
        cwd: repo.to_path_buf(),
        deadline: deadline_after(options.time_limit),
        config_env: Vec::new(),
    };
    let listing = runner.run_reading(
        "git rev-list",
        &[
            OsStr::new("rev-list"),
            OsStr::new("--topo-order"),
            OsStr::new("--reverse"),
            OsStr::new("--parents"),
            OsStr::new("--ignore-missing"),
            OsStr::new("--stdin"),
        ],
        &[0],
        NAMED_LISTING_CAP,
        list,
    )?;
    let mut commits = Vec::new();
    for record in String::from_utf8_lossy(&listing).lines() {
        let mut ids = record.split(' ');
        let (Some(commit), parent) = (ids.next(), ids.next()) else {
            continue;
        };
        if !is_object_id(commit) || parent.is_some_and(|id| !is_object_id(id)) {
            return Err(refused("git rev-list wrote a line that cannot be read"));
        }
        commits.push((commit.to_owned(), parent.map(str::to_owned)));
    }
    Ok(commits)
}

/// The best common ancestor of the commits `one` and `other`, or `None` when they share no
/// history.
///
/// # Errors
/// Any [`GitAcquireError`] from running Git; either not naming a commit that can be read.
pub fn merge_base(
    repo: &Path,
    one: &str,
    other: &str,
    options: &GitAcquireOptions,
) -> Result<Option<String>, GitAcquireError> {
    let runner = Runner {
        options,
        cwd: repo.to_path_buf(),
        deadline: deadline_after(options.time_limit),
        config_env: Vec::new(),
    };
    let named = |rev: &str| {
        runner
            .commit_id(rev)?
            .ok_or_else(|| refused(format!("`{rev}` does not name a commit")))
    };
    let (one, other) = (named(one)?, named(other)?);
    let found = line(runner.run(
        "git merge-base",
        &[
            OsStr::new("merge-base"),
            OsStr::new(&one),
            OsStr::new(&other),
        ],
        &[0, 1],
        4096,
    )?)?;
    Ok(Some(found).filter(|id| !id.is_empty()))
}

/// The submodules of the repository containing `repo` whose own working tree has changes that
/// are not committed there (modified or untracked files), as paths from the top level. The
/// patch of the outer repository shows a submodule only as the commit it is at.
///
/// # Errors
/// Any [`GitAcquireError`] from running Git.
pub fn dirty_submodules(
    repo: &Path,
    options: &GitAcquireOptions,
) -> Result<Vec<String>, GitAcquireError> {
    let runner = Runner {
        options,
        cwd: repo.to_path_buf(),
        deadline: deadline_after(options.time_limit),
        config_env: Vec::new(),
    };
    let status = runner.run(
        "git status",
        &[
            OsStr::new("status"),
            OsStr::new("--porcelain=v2"),
            OsStr::new("-z"),
            OsStr::new("--ignore-submodules=none"),
            OsStr::new("--untracked-files=no"),
        ],
        &[0],
        INDEX_LISTING_CAP,
    )?;
    // An ordinary entry is `1 XY SUB mH mI mW hH hI PATH`; `SUB` is `S` with a letter for a
    // changed commit, modified content (`M`) and untracked files (`U`).
    Ok(status
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            let text = std::str::from_utf8(entry).ok()?;
            let mut fields = text.splitn(9, ' ');
            let (kind, _, sub) = (fields.next()?, fields.next()?, fields.next()?);
            let path = fields.nth(5)?;
            let dirty = sub.starts_with('S')
                && (sub.as_bytes().get(2) == Some(&b'M') || sub.as_bytes().get(3) == Some(&b'U'));
            (kind == "1" && dirty).then(|| path.to_owned())
        })
        .collect())
}

/// What `HEAD` names in a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadState {
    /// A commit, by its full id.
    Commit(String),
    /// No commit: the branch `HEAD` points at does not exist yet.
    Unborn {
        /// Some other branch or tag of the repository has a commit (an orphan branch), as
        /// opposed to a repository with no commit at all.
        repository_has_commits: bool,
    },
}

/// What `HEAD` names in the repository containing `repo`.
///
/// # Errors
/// Any [`GitAcquireError`] from running Git.
pub fn head_state(repo: &Path, options: &GitAcquireOptions) -> Result<HeadState, GitAcquireError> {
    let runner = Runner {
        options,
        cwd: repo.to_path_buf(),
        deadline: deadline_after(options.time_limit),
        config_env: Vec::new(),
    };
    if let Some(id) = runner.commit_id("HEAD")? {
        return Ok(HeadState::Commit(id));
    }
    let any = runner.run(
        "git rev-list",
        &[
            OsStr::new("rev-list"),
            OsStr::new("--max-count=1"),
            OsStr::new("--all"),
        ],
        &[0],
        4096,
    )?;
    Ok(HeadState::Unborn {
        repository_has_commits: !any.is_empty(),
    })
}

/// Whether the history of `HEAD` contains `commit` (or is it). `false` when `HEAD` names no
/// commit.
///
/// # Errors
/// Any [`GitAcquireError`] from running Git, or `commit` not naming a commit that can be read.
pub fn head_contains(
    repo: &Path,
    commit: &str,
    options: &GitAcquireOptions,
) -> Result<bool, GitAcquireError> {
    let runner = Runner {
        options,
        cwd: repo.to_path_buf(),
        deadline: deadline_after(options.time_limit),
        config_env: Vec::new(),
    };
    let commit = runner
        .commit_id(commit)?
        .ok_or_else(|| refused(format!("`{commit}` does not name a commit")))?;
    let Some(head) = runner.commit_id("HEAD")? else {
        return Ok(false);
    };
    match runner.output_code(
        "git merge-base",
        &[
            OsStr::new("merge-base"),
            OsStr::new("--is-ancestor"),
            OsStr::new(&commit),
            OsStr::new(&head),
        ],
        &[0, 1],
    )? {
        0 => Ok(true),
        _ => Ok(false),
    }
}

/// The bytes of the file at `path` in `commit`, or `None` when that commit has no such file.
///
/// A judge's policy is read this way: from the base commit, never from the working tree, so a
/// change cannot loosen the policy it is judged by. `path` is relative to the repository's top
/// level. A tree, a link or a submodule at that path is `None`: only a regular file is a policy.
///
/// # Errors
/// Any [`GitAcquireError`] from running Git, or `commit` not naming a commit.
pub fn file_at_commit(
    repo: &Path,
    commit: &str,
    path: &str,
    options: &GitAcquireOptions,
) -> Result<Option<Vec<u8>>, GitAcquireError> {
    let deadline = deadline_after(options.time_limit);
    let runner = Runner {
        options,
        cwd: repo.to_path_buf(),
        deadline,
        config_env: Vec::new(),
    };
    let commit = runner
        .commit_id(commit)?
        .ok_or_else(|| refused(format!("`{commit}` does not name a commit")))?;
    if path.is_empty() {
        return Ok(None);
    }
    // `ls-tree -z` names the entry's mode, type and object without reading attributes or
    // running a filter. `:(literal)` keeps a path with `*` or `[` from being read as a pattern.
    let pathspec = OsString::from(format!(":(literal,top){path}"));
    let listing = runner.run(
        "git ls-tree",
        &[
            OsStr::new("ls-tree"),
            OsStr::new("-z"),
            OsStr::new("--full-tree"),
            OsStr::new(&commit),
            OsStr::new("--"),
            pathspec.as_os_str(),
        ],
        &[0],
        64 * 1024,
    )?;
    // Each entry is `<mode> SP <type> SP <object> TAB <path>`. Only the entry whose path is
    // exactly the one asked for counts: an empty path or a directory lists other entries.
    let mut wanted = None;
    for entry in listing.split(|&b| b == 0).filter(|e| !e.is_empty()) {
        let Some(tab) = entry.iter().position(|&b| b == b'\t') else {
            return Err(refused("git ls-tree wrote an entry that cannot be read"));
        };
        if &entry[tab + 1..] == path.as_bytes() {
            wanted = Some(&entry[..tab]);
        }
    }
    let Some(head) = wanted else {
        return Ok(None);
    };
    let fields: Vec<&[u8]> = head.split(|&b| b == b' ').collect();
    let [mode, kind, object] = fields[..] else {
        return Err(refused("git ls-tree wrote an entry that cannot be read"));
    };
    if kind != b"blob" || !(mode == b"100644" || mode == b"100755") {
        return Ok(None);
    }
    let object = String::from_utf8_lossy(object).into_owned();
    let bytes = runner.run(
        "git cat-file",
        &[
            OsStr::new("cat-file"),
            OsStr::new("blob"),
            OsStr::new(&object),
        ],
        &[0],
        options.byte_budget,
    )?;
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_alternates_line_is_a_plain_utf8_path() {
        assert_eq!(
            alternate_line(Path::new("/r/.git/objects")).unwrap(),
            "/r/.git/objects"
        );
        for bad in ["/a\nb", "#objects", "\"quoted\""] {
            assert!(alternate_line(Path::new(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_one_line_answer_loses_only_its_newline() {
        assert_eq!(line(b"abc\n".to_vec()).unwrap(), "abc");
        assert_eq!(line(b"abc".to_vec()).unwrap(), "abc");
        assert_eq!(line(b"a\nb\n".to_vec()).unwrap(), "a\nb");
        assert!(line(vec![0xff, b'\n']).is_err());
    }
}
