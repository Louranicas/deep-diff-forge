// DDF integration: R03, R07, R46. Shared by the conformance table's tests and the generated differential: real `git apply` in a scratch repository with a seeded object store, and the tree it writes against the tree a review describes.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module index: docs/ARCHITECTURE.md#crates-as-built-030
//! A scratch repository whose object store holds every blob a patch may name, Git run with no
//! user or system configuration, no hooks and no external diff, and the comparison of the tree a
//! real `git apply` writes with the tree DDF's review describes. `reviewed_tree` and `agree`
//! follow `git_differential.rs`'s `real_apply`, whose fixed cases are made on their base tree;
//! here `patched` places each hunk by Git's anchors, as `CONFORMANCE.md` promises, and checks
//! each removed and context line against the file, so a review whose hunks fit nowhere Git
//! would place them describes no tree.

use deep_diff_forge_core::{FileStatus, PatchLineKind, ReviewFile};
use deep_diff_forge_patch::old_path;
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};
use std::{fs, io};

/// One file of a tree. `content` is `None` where the review shows a binary change: it says the
/// file is rewritten and not with what, so any content agrees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub executable: bool,
    pub symlink: bool,
    pub content: Option<Vec<u8>>,
}

pub type Tree = BTreeMap<String, Entry>;

pub fn regular(content: &[u8]) -> Entry {
    Entry {
        executable: false,
        symlink: false,
        content: Some(content.to_vec()),
    }
}

/// A scratch repository holding a tree, with `blobs` in its object store. `ids` names every
/// stored blob in order, so a generator can write any of them, and `contents` maps each id to
/// its bytes, so the oracle can say what a binary section that names one writes.
pub struct Repo {
    scratch: deep_diff_forge_testkit::Scratch,
    pub ids: Vec<String>,
    pub contents: BTreeMap<String, Vec<u8>>,
}

impl Repo {
    /// A repository whose object ids are `format`'s (`sha1` or `sha256`).
    pub fn with_format(tag: &str, blobs: &[&[u8]], format: &str) -> Self {
        let scratch = deep_diff_forge_testkit::Scratch::new(&format!("git-conf-{tag}"));
        fs::create_dir(scratch.join("home")).unwrap();
        let repo = Self {
            scratch,
            ids: Vec::new(),
            contents: BTreeMap::new(),
        };
        fs::create_dir(repo.root()).unwrap();
        let format = format!("--object-format={format}");
        assert!(
            repo.git(&["init", "-q", &format]).status.success(),
            "git init {format}"
        );
        let ids: Vec<String> = blobs.iter().map(|blob| repo.store(blob)).collect();
        let contents = ids
            .iter()
            .cloned()
            .zip(blobs.iter().map(|b| b.to_vec()))
            .collect();
        Self {
            ids,
            contents,
            ..repo
        }
    }

    pub fn root(&self) -> std::path::PathBuf {
        self.scratch.join("work")
    }

    /// Git with no user or system configuration, a private home, no hooks, no external diff
    /// or text conversion, file transport only, and no variable of the caller's that names a
    /// repository, index or object store.
    pub fn git(&self, args: &[&str]) -> Output {
        let mut command = Command::new("git");
        command
            .current_dir(self.root())
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/local/bin")
            .env("HOME", self.scratch.join("home"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_ALLOW_PROTOCOL", "file")
            .env("GIT_TERMINAL_PROMPT", "0")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "core.autocrlf=false",
                "-c",
                "core.quotepath=false",
            ])
            .args(args);
        command.output().unwrap()
    }

    /// Stores `content` as a blob and returns its id.
    pub fn store(&self, content: &[u8]) -> String {
        let path = self.scratch.join("blob.tmp");
        fs::write(&path, content).unwrap();
        let out = self.git(&["hash-object", "-w", path.to_str().unwrap()]);
        assert!(out.status.success(), "git hash-object");
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    /// Replaces the work tree with `tree` (the object store is kept).
    pub fn reset(&self, tree: &Tree) {
        for entry in fs::read_dir(self.root()).unwrap() {
            let path = entry.unwrap().path();
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                fs::remove_dir_all(&path).unwrap();
            } else {
                fs::remove_file(&path).unwrap();
            }
        }
        for (name, entry) in tree {
            let path = self.root().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let content = entry.content.as_deref().unwrap_or_default();
            if entry.symlink {
                std::os::unix::fs::symlink(std::str::from_utf8(content).unwrap(), &path).unwrap();
            } else {
                fs::write(&path, content).unwrap();
                let mode = if entry.executable { 0o755 } else { 0o644 };
                fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            }
        }
    }

    fn write_patch(&self, patch: &[u8]) -> String {
        fs::write(self.root().join(".git/probe.patch"), patch).unwrap();
        ".git/probe.patch".to_owned()
    }

    /// `git apply --numstat --summary`: Git's reading of the patch, or `None` when Git refuses
    /// to read it.
    pub fn reading(&self, patch: &[u8]) -> Option<String> {
        let name = self.write_patch(patch);
        let out = self.git(&["apply", "--numstat", "--summary", &name]);
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// `git apply --check`.
    pub fn checks(&self, patch: &[u8]) -> bool {
        let name = self.write_patch(patch);
        self.git(&["apply", "--check", &name]).status.success()
    }

    /// A real `git apply` on `before`: whether Git applied the patch, what it said on stderr,
    /// and the tree it left (which must be `before` when it refused: Git refuses a patch whole).
    pub fn apply_outcome(&self, before: &Tree, patch: &[u8]) -> (bool, String, Tree) {
        self.reset(before);
        let name = self.write_patch(patch);
        let out = self.git(&["apply", &name]);
        let mut after = Tree::new();
        read_tree(&self.root(), &self.root(), &mut after).unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        (out.status.success(), stderr, after)
    }
}

/// The work tree under `dir`, without `.git`. An empty directory (what Git makes for a
/// submodule) is an entry whose name ends in `/`.
fn read_tree(root: &Path, dir: &Path, into: &mut Tree) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let meta = fs::symlink_metadata(&path)?;
        let name = path
            .strip_prefix(root)
            .map_err(io::Error::other)?
            .to_string_lossy()
            .into_owned();
        if name == ".git" {
            continue;
        }
        if meta.is_dir() {
            if fs::read_dir(&path)?.next().is_none() {
                into.insert(format!("{name}/"), regular(b""));
            }
            read_tree(root, &path, into)?;
        } else if meta.file_type().is_symlink() {
            let target = fs::read_link(&path)?;
            into.insert(
                name,
                Entry {
                    executable: false,
                    symlink: true,
                    content: Some(target.as_os_str().as_encoded_bytes().to_vec()),
                },
            );
        } else {
            into.insert(
                name,
                Entry {
                    executable: meta.permissions().mode() & 0o100 != 0,
                    symlink: false,
                    content: Some(fs::read(&path)?),
                },
            );
        }
    }
    Ok(())
}

/// `old` with the file's hunks applied where plain `git apply` places them, or `None` when Git
/// finds no place for one. Git's rule (`apply_one_fragment`, `find_pos`, `match_fragment`):
/// a hunk at old line 0 or 1 must match at the start of the file, one with no trailing context
/// at its end (both: the whole file); any other is tried at its header's new line, then one
/// line after, one before, two after and so on. Its context and removed lines must equal the
/// file's there, and no line an earlier hunk wrote may match again. Without `-C` Git tries
/// nothing else.
fn patched(old: &[u8], file: &ReviewFile) -> Option<Vec<u8>> {
    let mut image: Vec<(Vec<u8>, bool)> = old
        .split_inclusive(|&b| b == b'\n')
        .map(|l| (l.to_vec(), false))
        .collect();
    for hunk in &file.patch_twin.hunks {
        let text = |kind: PatchLineKind| -> Vec<Vec<u8>> {
            hunk.lines
                .iter()
                .filter(|l| l.kind != kind)
                .map(|l| {
                    let mut t = l.text.as_bytes().to_vec();
                    if !l.no_newline {
                        t.push(b'\n');
                    }
                    t
                })
                .collect()
        };
        let (pre, post) = (text(PatchLineKind::Added), text(PatchLineKind::Removed));
        let start = hunk.old_start.unwrap_or(0) <= 1;
        let end = hunk
            .lines
            .last()
            .is_none_or(|l| l.kind != PatchLineKind::Context);
        // What the review says of the hunk must be the anchor this oracle places it by, and
        // nothing for a hunk placed by search.
        let said = match (start, end) {
            (true, true) => Some("Git places this hunk on the whole file"),
            (true, false) => Some("Git places this hunk at the start of the file"),
            (false, true) => Some("Git places this hunk at the end of the file"),
            (false, false) => None,
        };
        assert_eq!(
            hunk.git_anchor(),
            said,
            "git_anchor() disagrees with the oracle"
        );
        let line = if start {
            0
        } else if end {
            image.len().checked_sub(pre.len()).unwrap_or(image.len())
        } else {
            (hunk.new_start.unwrap_or(0) as usize)
                .saturating_sub(1)
                .min(image.len())
        };
        let fits = |at: usize| {
            at + pre.len() <= image.len()
                && (!end || at + pre.len() == image.len())
                && (!start || at == 0)
                && pre
                    .iter()
                    .zip(&image[at..])
                    .all(|(p, (l, done))| !done && p == l)
        };
        let after = (1..=image.len()).flat_map(|d| [Some(line + d), line.checked_sub(d)]);
        let at = std::iter::once(Some(line))
            .chain(after)
            .flatten()
            .filter(|&at| at <= image.len())
            .find(|&at| fits(at))?;
        let written = post.into_iter().map(|l| (l, true));
        image.splice(at..at + pre.len(), written);
    }
    Some(image.into_iter().flat_map(|(l, _)| l).collect())
}

/// What a binary section writes, as its `index` line names it: nothing for the null id, else the
/// blob whose full id it names when the store holds it. `None` when the review does not say:
/// an abbreviated id (Git refuses to apply it), or a blob outside `blobs` (Git builds it from
/// the `GIT binary patch` data, which this oracle does not decode; Git checks the result against
/// the id itself). Those are the binary results this oracle does not prove.
fn binary_result(file: &ReviewFile, blobs: &BTreeMap<String, Vec<u8>>) -> Option<Vec<u8>> {
    let ids = file
        .patch_twin
        .metadata
        .iter()
        .find_map(|m| m.strip_prefix("index "))?;
    let new = ids.split(' ').next()?.split_once("..")?.1;
    if !new.is_empty() && new.bytes().all(|b| b == b'0') {
        return Some(Vec::new());
    }
    blobs.get(new).cloned()
}

/// The tree the review describes: its sections carried out one after the other on `before`,
/// each as its status, paths, mode headers and hunks say, or `None` when a hunk does not fit
/// the file it names. A rename or copy takes its source from `before`, which is what its hunks
/// are written against. A binary section writes the blob its `index` line names (see
/// [`binary_result`]).
pub fn reviewed_tree(
    before: &Tree,
    files: &[ReviewFile],
    blobs: &BTreeMap<String, Vec<u8>>,
) -> Option<Tree> {
    let mut after = before.clone();
    for file in files {
        let meta = &file.patch_twin.metadata;
        let says = |prefix: &str| meta.iter().any(|m| m.starts_with(prefix));
        let mode = ["new file mode ", "new mode "]
            .iter()
            .find_map(|p| meta.iter().find_map(|m| m.strip_prefix(p)));
        if file.status == FileStatus::Deleted || says("deleted file mode ") {
            after.remove(&file.path);
            continue;
        }
        let moved_from = old_path(file);
        let mut entry = match &moved_from {
            Some(from) => before.get(from).cloned(),
            None => after.get(&file.path).cloned(),
        }
        .unwrap_or_else(|| regular(b""));
        if says("rename from ") {
            after.remove(moved_from.as_deref()?);
        }
        if let Some(mode) = mode {
            entry.executable = mode == "100755";
            entry.symlink = mode == "120000";
        }
        if mode == Some("160000") {
            // A submodule: Git makes an empty directory and writes nothing of the section.
            after.insert(format!("{}/", file.path), regular(b""));
            continue;
        }
        entry.content = match entry.content.take() {
            _ if file.patch_twin.is_binary() => binary_result(file, blobs),
            // An earlier section's binary change left content the review does not know; Git
            // applies a later section to it, and that is not checked here.
            None => None,
            Some(old) => Some(patched(&old, file)?),
        };
        after.insert(file.path.clone(), entry);
    }
    Some(after)
}

/// Whether Git's tree is the reviewed one: the same paths with the same type and mode, and the
/// same content wherever the review shows content. An empty directory Git leaves behind after it
/// moves a file out of it is no file, and is not compared unless the review makes one (a
/// submodule's).
pub fn agree(git: &Tree, reviewed: &Tree) -> bool {
    let git: Tree = git
        .iter()
        .filter(|(path, _)| !path.ends_with('/') || reviewed.contains_key(*path))
        .map(|(path, entry)| (path.clone(), entry.clone()))
        .collect();
    git.len() == reviewed.len()
        && git.iter().zip(reviewed).all(|((gp, g), (rp, r))| {
            gp == rp
                && g.executable == r.executable
                && g.symlink == r.symlink
                && (r.content.is_none() || g.content == r.content)
        })
}
