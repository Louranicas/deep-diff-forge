#![no_main]
// DDF 0.9.0, task S2.5: the patch the hooks build from a working tree for the judge and the
// evidence (`deep_diff_forge_patch::judge_patch`, which `agent hook post-tool` and `agent hook
// stop` call), from edits the fuzzer chooses: files written, deleted or made executable, with
// any bytes in their names and contents. Real Git runs (one base repository per process, put
// back to its base commit before every input).
// Design/status: docs/TESTING_GOLD_STANDARD.md (fuzz targets).
//
// The property: no panic, and the patch is one the parser reads, as the hooks read it
// (`SealedReview::new` with no guards), or refuses with an error; when it reads it, the paths
// it reads are exactly the paths the edits changed, each once.
use deep_diff_forge_core::process::{Limits, run_bounded};
use deep_diff_forge_patch::{GitAcquireOptions, InputGuards, SealedReview, judge_patch};
use libfuzzer_sys::fuzz_target;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The files of the base commit: plain names, names Git quotes in a patch header, and a name
/// past ASCII.
const BASE: [(&[u8], &[u8]); 7] = [
    (b"a.txt", b"one\ntwo\n"),
    (b"src/lib.rs", b"fn a() {}\n"),
    (b"sp ace.md", b"x\n"),
    (b"quo\"te", b"q\n"),
    (b"t\tab", b"t\n"),
    (b"\xc3\xbcn\xc3\xafcode", b"u\n"),
    (b"dir/sub/deep.py", b"print(1)\n"),
];

/// A file of the working tree: its bytes and whether it is executable.
type Tree = BTreeMap<Vec<u8>, (Vec<u8>, bool)>;

fn base_tree() -> Tree {
    BASE.iter()
        .map(|(path, bytes)| (path.to_vec(), (bytes.to_vec(), false)))
        .collect()
}

/// The environment of every Git this target runs or has run: no configuration or ignore file
/// of the system or the user, so the patch depends on the edits alone.
fn quiet_env(root: &Path) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    [
        ("GIT_CONFIG_NOSYSTEM", OsStr::new("1")),
        ("GIT_CONFIG_GLOBAL", OsStr::new("/dev/null")),
        ("XDG_CONFIG_HOME", root.join("xdg").as_os_str()),
    ]
    .into_iter()
    .map(|(name, value)| (name.into(), value.to_owned()))
    .collect()
}

/// Run Git in `dir` under a deadline (the target's door to a process: it makes the base
/// repository).
#[expect(
    clippy::disallowed_methods,
    reason = "the fuzz target's door: Git makes its base repository, under run_bounded"
)]
fn git(dir: &Path, root: &Path, args: &[&str]) {
    let mut command = Command::new("git");
    command
        .current_dir(dir)
        .envs(quiet_env(root))
        .args([
            "-c",
            "user.name=fuzz",
            "-c",
            "user.email=fuzz@invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args);
    let limits = Limits {
        time: std::time::Duration::from_secs(60),
        stdout_bytes: 1 << 20,
        stderr_bytes: 1 << 16,
    };
    let done = run_bounded(&mut command, &[], limits).expect("git runs");
    assert!(done.status.success(), "git {args:?} in {}", dir.display());
}

/// The directory of this process's files: under the system's temporary directory (`TMPDIR`),
/// named by the process, so two fuzzing processes do not share a repository.
#[expect(
    clippy::disallowed_methods,
    reason = "the fuzz target's door: its repository lives as long as the process"
)]
fn process_dir() -> PathBuf {
    std::env::temp_dir().join(format!("ddf-fuzz-judge-patch-{}", std::process::id()))
}

fn path_of(dir: &Path, path: &[u8]) -> PathBuf {
    dir.join(OsStr::from_bytes(path))
}

/// Make the working tree `tree` under `dir`, whose only other entry is `.git`.
fn write_tree(dir: &Path, tree: &Tree) {
    for entry in std::fs::read_dir(dir).expect("the repository can be read") {
        let entry = entry.expect("an entry can be read");
        if entry.file_name() == ".git" {
            continue;
        }
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            std::fs::remove_dir_all(entry.path()).expect("a directory can be removed");
        } else {
            std::fs::remove_file(entry.path()).expect("a file can be removed");
        }
    }
    for (path, (bytes, executable)) in tree {
        let at = path_of(dir, path);
        std::fs::create_dir_all(at.parent().expect("a file has a directory"))
            .expect("a directory can be made");
        std::fs::write(&at, bytes).expect("a file can be written");
        let mode = if *executable { 0o755 } else { 0o644 };
        std::fs::set_permissions(&at, std::fs::Permissions::from_mode(mode))
            .expect("a mode can be set");
    }
}

/// The directory of this process, the repository in it with the base commit, and the
/// directory its scratch directories go in.
fn repository() -> &'static (PathBuf, PathBuf, PathBuf) {
    static PLACE: OnceLock<(PathBuf, PathBuf, PathBuf)> = OnceLock::new();
    PLACE.get_or_init(|| {
        let root = process_dir();
        if root.exists() {
            std::fs::remove_dir_all(&root).expect("an old directory can be removed");
        }
        let (repo, scratch) = (root.join("repo"), root.join("scratch"));
        std::fs::create_dir_all(&repo).expect("the repository directory can be made");
        std::fs::create_dir_all(&scratch).expect("the scratch directory can be made");
        git(&repo, &root, &["init", "-q", "--template="]);
        write_tree(&repo, &base_tree());
        git(&repo, &root, &["add", "--all"]);
        git(&repo, &root, &["commit", "-q", "-m", "base"]);
        (root, repo, scratch)
    })
}

/// Whether `path` can name a file of the working tree: relative, components that are not
/// empty, `.`, `..` or Git's own (`.git`, and the `.gitignore` and `.gitattributes` files,
/// which change what is staged or how), and no NUL.
fn usable(path: &[u8]) -> bool {
    !path.is_empty()
        && path.len() <= 200
        && !path.contains(&0)
        && path.split(|b| *b == b'/').all(|part| {
            !part.is_empty()
                && part.len() <= 64
                && part != b"."
                && part != b".."
                && !part.to_ascii_lowercase().starts_with(b".git")
        })
}

/// Apply the edits `data` spells to `tree`: records of an operation byte, a path (a byte below
/// 0x80 names a base file; another gives the length of the name that follows) and, for a
/// write, a length byte and the bytes. An edit that cannot be made (a path under a file, a
/// file where a directory is) is passed over.
fn edit(tree: &mut Tree, data: &[u8]) {
    let mut rest = data;
    let mut take = |n: usize| -> Option<&[u8]> {
        let (head, tail) = rest.split_at_checked(n)?;
        rest = tail;
        Some(head)
    };
    for _ in 0..8 {
        let Some(&[operation, name]) = take(2) else {
            return;
        };
        let path = if name < 0x80 {
            BASE[usize::from(name) % BASE.len()].0.to_vec()
        } else {
            let Some(path) = take(usize::from(name & 0x3f) + 1) else {
                return;
            };
            path.to_vec()
        };
        let under_a_file =
            (1..path.len()).any(|end| path[end] == b'/' && tree.contains_key(&path[..end]));
        let is_a_directory = tree.keys().any(|key| {
            key.len() > path.len() && key.starts_with(&path) && key[path.len()] == b'/'
        });
        match operation % 4 {
            0 | 1 => {
                let Some(&[len]) = take(1) else {
                    return;
                };
                let Some(bytes) = take(usize::from(len)) else {
                    return;
                };
                if usable(&path) && !under_a_file && !is_a_directory {
                    let executable = tree.get(&path).is_some_and(|(_, x)| *x);
                    tree.insert(path, (bytes.to_vec(), executable));
                }
            }
            2 => {
                tree.remove(&path);
            }
            _ => {
                if let Some((_, executable)) = tree.get_mut(&path) {
                    *executable = !*executable;
                }
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() > 4096 {
        return;
    }
    let (root, repo, scratches) = repository();
    let mut tree = base_tree();
    edit(&mut tree, data);
    let base = base_tree();
    let changed: BTreeSet<Vec<u8>> = base
        .keys()
        .chain(tree.keys())
        .filter(|path| base.get(*path) != tree.get(*path))
        .cloned()
        .collect();
    write_tree(repo, &tree);
    let scratch = scratches.join("s");
    std::fs::create_dir(&scratch).expect("a scratch directory can be made");
    let options = GitAcquireOptions {
        time_limit: std::time::Duration::from_secs(60),
        env: quiet_env(root),
        ..GitAcquireOptions::default()
    };
    let made = judge_patch(repo, "HEAD", &scratch, &options);
    std::fs::remove_dir_all(&scratch).expect("the scratch directory can be removed");
    // A patch that could not be made is refused by the builder with its reason.
    let Ok(patch) = made else {
        return;
    };
    let guards = InputGuards {
        files: false,
        hunks: false,
    };
    let Ok(review) = SealedReview::new(&patch.bytes, guards, "fuzz") else {
        // Refused by the parser, with its error.
        return;
    };
    let read: Vec<&str> = review.files().iter().map(|file| file.path.as_str()).collect();
    let once: BTreeSet<&str> = read.iter().copied().collect();
    assert_eq!(once.len(), read.len(), "a path is read twice: {read:?}");
    let named: BTreeSet<String> = changed
        .iter()
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect();
    let read: BTreeSet<String> = once.into_iter().map(str::to_owned).collect();
    assert_eq!(
        read,
        named,
        "the parser reads other paths than the edits changed:\n{}",
        String::from_utf8_lossy(&patch.bytes)
    );
    assert!(
        changed.iter().all(|path| std::str::from_utf8(path).is_ok()),
        "a name that is not UTF-8 was read: {named:?}"
    );
});
