// DDF integration: R03, R07, R46. Shared by the conformance table's tests and the generated differential: real `git apply` in a scratch repository with a seeded object store, and the tree it writes against the tree a review describes.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-patch--tests--support
//! A scratch repository whose object store holds every blob a patch may name, Git run with no
//! user or system configuration, no hooks and no external diff, and the comparison of the tree a
//! real `git apply` writes with the tree DDF's review describes. `reviewed_tree` and `agree`
//! follow `git_differential.rs`'s `real_apply` (kept the same there); here `patched` also checks
//! each removed and context line against the file, so a review whose hunks do not fit the file
//! it names describes no tree.

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

/// A scratch repository holding a tree, with `blobs` in its object store. A patch's `@OLD@` is
/// the id of the first file's blob, `@NEW@` the id of the first stored blob and `@ZERO@` the null
/// id; `ids` names every stored blob, so a generator can write any of them.
pub struct Repo {
    scratch: deep_diff_forge_testkit::Scratch,
    pub ids: Vec<String>,
}

impl Repo {
    pub fn new(tag: &str, blobs: &[&[u8]]) -> Self {
        let scratch = deep_diff_forge_testkit::Scratch::new(&format!("git-conf-{tag}"));
        fs::create_dir(scratch.join("home")).unwrap();
        let repo = Self {
            scratch,
            ids: Vec::new(),
        };
        fs::create_dir(repo.root()).unwrap();
        assert!(repo.git(&["init", "-q"]).status.success(), "git init");
        let ids = blobs.iter().map(|blob| repo.store(blob)).collect();
        Self { ids, ..repo }
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
                "-c",
                "diff.external=",
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

    /// A real `git apply` on `before`: the tree Git leaves, or `None` when it refuses (and so
    /// writes nothing).
    pub fn apply(&self, before: &Tree, patch: &[u8]) -> Option<Tree> {
        self.reset(before);
        let name = self.write_patch(patch);
        let applied = self.git(&["apply", &name]).status.success();
        let mut after = Tree::new();
        read_tree(&self.root(), &self.root(), &mut after).unwrap();
        applied.then_some(after)
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

/// `old` with the file's hunks applied at the lines they name, or `None` when a removed or
/// context line is not the line of `old` it names (Git would not apply the hunk there).
fn patched(old: &[u8], file: &ReviewFile) -> Option<Vec<u8>> {
    let old: Vec<&[u8]> = old.split_inclusive(|&b| b == b'\n').collect();
    let (mut at, mut out) = (0, Vec::new());
    for hunk in &file.patch_twin.hunks {
        let start = (hunk.old_start.unwrap_or(0) as usize).saturating_sub(1);
        while at < start && at < old.len() {
            out.extend_from_slice(old[at]);
            at += 1;
        }
        for line in &hunk.lines {
            let mut text = line.text.as_bytes().to_vec();
            if !line.no_newline {
                text.push(b'\n');
            }
            match line.kind {
                PatchLineKind::Context | PatchLineKind::Removed => {
                    if old.get(at).copied() != Some(text.as_slice()) {
                        return None;
                    }
                    if line.kind == PatchLineKind::Context {
                        out.extend_from_slice(&text);
                    }
                    at += 1;
                }
                PatchLineKind::Added => out.extend_from_slice(&text),
            }
        }
    }
    for line in old.iter().skip(at) {
        out.extend_from_slice(line);
    }
    Some(out)
}

/// The tree the review describes: its sections carried out one after the other on `before`,
/// each as its status, paths, mode headers and hunks say, or `None` when a hunk does not fit
/// the file it names. A rename or copy takes its source from `before`, which is what its hunks
/// are written against.
pub fn reviewed_tree(before: &Tree, files: &[ReviewFile]) -> Option<Tree> {
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
            _ if file.status == FileStatus::BinaryChanged => None,
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
/// same content wherever the review shows content.
pub fn agree(git: &Tree, reviewed: &Tree) -> bool {
    git.len() == reviewed.len()
        && git.iter().zip(reviewed).all(|((gp, g), (rp, r))| {
            gp == rp
                && g.executable == r.executable
                && g.symlink == r.symlink
                && (r.content.is_none() || g.content == r.content)
        })
}
