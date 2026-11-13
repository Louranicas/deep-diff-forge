//! The patch a judge is given for a working tree.
//!
//! A judge that reads a patch needs the same bytes whoever makes it. A plain `git diff` is not
//! that: `diff.noprefix` drops the `a/` `b/` prefixes, `diff.relative` leaves out what lies
//! outside the current directory, an attribute can turn text into a binary patch, and rename
//! detection changes which files a change names. forge-warden's patch maker answers this for two
//! commits by diffing them in a new, empty repository that only borrows the object store, so no
//! configuration of the user, the system or the repository is read.
//!
//! [`judge_patch`] does the same for a working tree. It writes the tree a commit of everything
//! (`git add --all`) would record, without touching the repository: the index it fills and the
//! objects it writes are in a scratch directory the caller owns, and the repository's own object
//! store is only read. Then it diffs the base commit against that tree the way the patch maker
//! does. For a state that is committed, the bytes equal the patch maker's (a test holds this).
//!
//! What the tree holds is what `git add --all` holds: tracked changes, deletions and untracked
//! files, without ignored files. A content filter the repository's own configuration defines is
//! switched off first, as for every acquisition, so making the patch runs no program the
//! repository chose.

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
}

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

/// The environment of the diff itself: no system or user configuration, no system attributes,
/// and no configuration passed down by a parent Git (`git -c …` and `GIT_CONFIG_COUNT`).
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

/// What is read from the repository before anything is made: where it is, how to read it
/// without running its filters, and what it is compared with.
struct Source {
    root: std::path::PathBuf,
    /// The configuration that switches the repository's own filters off.
    filters: Vec<(OsString, OsString)>,
    /// The base commit, resolved.
    base: String,
    /// The repository's object directory.
    objects: std::path::PathBuf,
    /// `sha1` or `sha256`.
    object_format: String,
}

fn source(
    repo: &Path,
    base: &str,
    options: &GitAcquireOptions,
    deadline: std::time::Instant,
) -> Result<Source, GitAcquireError> {
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
    let filters = repository_filters_off(options, &root, deadline)?;
    let inside = Runner {
        options,
        cwd: root.clone(),
        deadline,
        config_env: filters.clone(),
    };
    let base = inside
        .commit_id(base)?
        .ok_or_else(|| refused(format!("`{base}` does not name a commit")))?;
    let mut objects = inside.run(
        "git rev-parse",
        &[
            OsStr::new("rev-parse"),
            OsStr::new("--path-format=absolute"),
            OsStr::new("--git-path"),
            OsStr::new("objects"),
        ],
        &[0],
        64 * 1024,
    )?;
    if objects.last() == Some(&b'\n') {
        objects.pop();
    }
    let object_format = line(inside.run(
        "git rev-parse",
        &[OsStr::new("rev-parse"), OsStr::new("--show-object-format")],
        &[0],
        4096,
    )?)?;
    Ok(Source {
        root,
        filters,
        base,
        objects: path_from_bytes(objects),
        object_format,
    })
}

/// The tree of the working state, written into `own_objects` beside the repository, not into
/// it: the index that is filled is in `scratch` too.
fn working_tree(
    from: &Source,
    scratch: &Path,
    own_objects: &Path,
    options: &GitAcquireOptions,
    deadline: std::time::Instant,
) -> Result<String, GitAcquireError> {
    std::fs::create_dir_all(own_objects)
        .map_err(|e| refused(format!("cannot prepare the scratch directory: {e}")))?;
    let mut staging_env = from.filters.clone();
    staging_env.extend([
        pair("GIT_INDEX_FILE", scratch.join("index")),
        pair("GIT_OBJECT_DIRECTORY", own_objects.to_path_buf()),
        pair("GIT_ALTERNATE_OBJECT_DIRECTORIES", from.objects.clone()),
    ]);
    let staging = Runner {
        options,
        cwd: from.root.clone(),
        deadline,
        config_env: staging_env,
    };
    staging.run(
        "git read-tree",
        &[OsStr::new("read-tree"), OsStr::new(&from.base)],
        &[0],
        4096,
    )?;
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
    line(staging.run("git write-tree", &[OsStr::new("write-tree")], &[0], 4096)?)
}

/// The diff of the base commit against `tree`, made in a new, empty repository in `scratch`
/// that only borrows the two object stores: no configuration of the user, the system or the
/// reviewed repository is read for it.
fn diff_outside(
    from: &Source,
    tree: &str,
    scratch: &Path,
    own_objects: &Path,
    options: &GitAcquireOptions,
    deadline: std::time::Instant,
) -> Result<Vec<u8>, GitAcquireError> {
    let empty_repository = scratch.join("diff.git");
    let outside = Runner {
        options,
        cwd: scratch.to_path_buf(),
        deadline,
        config_env: ISOLATED.iter().map(|(k, v)| pair(k, *v)).collect(),
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
            empty_repository.as_os_str(),
        ],
        &[0],
        4096,
    )?;
    let alternates = format!(
        "{}\n{}\n",
        alternate_line(&from.objects)?,
        alternate_line(own_objects)?
    );
    std::fs::write(empty_repository.join("objects/info/alternates"), alternates)
        .map_err(|e| refused(format!("cannot write the alternates file: {e}")))?;
    let mut git_dir = OsString::from("--git-dir=");
    git_dir.push(empty_repository.as_os_str());
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
    args.push(OsStr::new(&from.base));
    args.push(OsStr::new(tree));
    outside.run("git diff", &args, &[0], options.byte_budget)
}

/// The working tree of the repository containing `repo`, as one patch against `base`.
///
/// `scratch` is an existing, empty directory the caller owns and removes afterwards; the index,
/// the objects and the bare repository this needs are made inside it. Nothing in the repository
/// is written: not its index, not its object store, not its configuration.
///
/// # Errors
/// Any [`GitAcquireError`]: `base` names no commit, Git failed or ran out of time or budget, or
/// a path cannot be used. Nothing partial is returned.
pub fn judge_patch(
    repo: &Path,
    base: &str,
    scratch: &Path,
    options: &GitAcquireOptions,
) -> Result<JudgePatch, GitAcquireError> {
    let deadline = deadline_after(options.time_limit);
    let from = source(repo, base, options, deadline)?;
    let own_objects = scratch.join("objects");
    let tree = working_tree(&from, scratch, &own_objects, options, deadline)?;
    let bytes = diff_outside(&from, &tree, scratch, &own_objects, options, deadline)?;
    Ok(JudgePatch {
        bytes,
        base: from.base,
        tree,
    })
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
