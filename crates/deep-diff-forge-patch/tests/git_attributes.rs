//! S3.20, patch truth: a change Git first gives as binary because the repository told it to
//! (`*.rs -diff` here, in `.git/info/attributes`, which no diff shows) or because of a NUL byte is
//! read again with `--text`, and the patch that is reviewed is still one `git apply` accepts and
//! that writes the same bytes. Every shape of change is read so: a modification, an addition, a
//! deletion, a rename, an untracked file, and a path that reads as a pathspec pattern; a real
//! binary stays binary, and a text reading over the byte budget leaves the binary one standing.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes and temp directories"
)]

use deep_diff_forge_patch::{
    GitAcquireOptions, GitAcquisition, GitChange, GitPath, GitSelection, acquire_git,
};
use deep_diff_forge_testkit::Scratch;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

const ISOLATE: [(&str, &str); 2] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

fn git(root: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git")
        .current_dir(root)
        .envs(ISOLATE)
        .args([
            "-c",
            "user.name=ddf",
            "-c",
            "user.email=ddf@example.invalid",
        ])
        .args([
            "-c",
            "commit.gpgSign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn options(include_untracked: bool, byte_budget: usize) -> GitAcquireOptions {
    GitAcquireOptions {
        include_untracked,
        byte_budget,
        env: ISOLATE
            .iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
            .collect(),
        ..GitAcquireOptions::default()
    }
}

const SPAWN: &[u8] = b"    let _ = std::process::Command::new(\"sh\").status();\n";

fn source(nul: bool, spawn: bool) -> Vec<u8> {
    let mut text = b"fn main() {\n".to_vec();
    if nul {
        text.extend_from_slice(b"    // marker \0 here\n");
    }
    text.extend_from_slice(b"    let x = 1;\n");
    if spawn {
        text.extend_from_slice(SPAWN);
    }
    text.extend_from_slice(b"    println!(\"{x}\");\n}\n");
    text
}

/// A repository whose `.git/info/attributes` says `*.rs -diff`, with `files` committed.
fn repo(name: &str, files: &[(&str, Vec<u8>)]) -> (PathBuf, Scratch) {
    let scratch = Scratch::new(&format!("attr-{name}"));
    let root = scratch.join("repo");
    std::fs::create_dir_all(root.join(".git/info")).unwrap();
    git(&root, &["init", "-q"]);
    std::fs::write(root.join(".git/info/attributes"), b"*.rs -diff\n").unwrap();
    for (path, bytes) in files {
        std::fs::write(root.join(path), bytes).unwrap();
    }
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    (root, scratch)
}

fn change<'a>(acq: &'a GitAcquisition, path: &str) -> &'a GitChange {
    acq.diff()
        .changes()
        .iter()
        .find(|c| c.review_path().map(GitPath::as_bytes) == Some(path.as_bytes()))
        .unwrap_or_else(|| panic!("no change for {path:?}"))
}

/// The change of `path` is text (hunks, no binary payload) and adds the spawn line.
fn read_as_text(acq: &GitAcquisition, path: &str) {
    let c = change(acq, path);
    let block = acq.diff().change_patch(c);
    let text = String::from_utf8_lossy(block);
    assert!(!c.binary, "{path} is still a binary change:\n{text}");
    assert!(!text.contains("GIT binary patch"), "{text}");
    let spawn = format!("+{}", String::from_utf8_lossy(SPAWN));
    assert!(text.contains(&spawn), "{text}");
}

/// `git apply` of the acquired patch, in a plain directory (no repository, so no attribute
/// applies) holding `base`, accepts it and writes exactly `want` (`None`: the file is gone).
fn applies(
    scratch: &Scratch,
    patch: &[u8],
    base: &[(&str, Vec<u8>)],
    want: &[(&str, Option<Vec<u8>>)],
) {
    let dir = scratch.join("plain");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (path, bytes) in base {
        std::fs::write(dir.join(path), bytes).unwrap();
    }
    std::fs::write(scratch.join("reviewed.patch"), patch).unwrap();
    let out = Command::new("git")
        .current_dir(&dir)
        .envs(ISOLATE)
        .env("GIT_CEILING_DIRECTORIES", scratch.path())
        .args(["apply", "--whitespace=nowarn", "../reviewed.patch"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git apply refused the reviewed patch: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    for (path, bytes) in want {
        match bytes {
            Some(bytes) => assert_eq!(&std::fs::read(dir.join(path)).unwrap(), bytes, "{path}"),
            None => assert!(!dir.join(path).exists(), "{path} still exists"),
        }
    }
}

/// In each selection, with the attribute and with a NUL byte, the reviewed patch is text and
/// `git apply` turns the base into the target, byte for byte.
#[test]
fn the_hidden_change_is_read_as_text_and_still_applies_to_the_same_bytes() {
    for nul in [false, true] {
        for selection in ["range", "working_tree", "staged"] {
            let base = [("m.rs", source(nul, false))];
            let (root, scratch) = repo(&format!("{selection}-{nul}"), &base);
            let target = source(nul, true);
            std::fs::write(root.join("m.rs"), &target).unwrap();
            let selection = match selection {
                "range" => {
                    git(&root, &["commit", "-q", "-a", "-m", "change"]);
                    GitSelection::parse_range("HEAD~1..HEAD").unwrap()
                }
                "staged" => {
                    git(&root, &["add", "m.rs"]);
                    GitSelection::Staged
                }
                _ => GitSelection::WorkingTree,
            };
            let acq = acquire_git(&root, &selection, &options(false, 1 << 20)).unwrap();
            read_as_text(&acq, "m.rs");
            assert_eq!(acq.diff().changes().len(), 1);
            let patch = acq.diff().patch_bytes();
            assert_eq!(acq.diff().change_patch(&acq.diff().changes()[0]), patch);
            applies(&scratch, patch, &base, &[("m.rs", Some(target))]);
        }
    }
}

/// One working-tree change holding every shape: an addition, a deletion, a rename with an edit,
/// an untracked file, a file named `*.rs` beside `x.rs` (a pattern if read as a pathspec), and a
/// real image between them. Each source file is read as text, the image stays binary, the
/// sections keep Git's order, and the whole patch applies to the same bytes.
#[test]
fn every_shape_of_change_is_read_again_and_the_patch_applies_whole() {
    let gone: Vec<u8> = (1..=12)
        .flat_map(|n| format!("const Z{n}: u8 = {n};\n").into_bytes())
        .collect();
    let moved: Vec<u8> = (1..=30)
        .flat_map(|n| format!("    let a{n} = {n};\n").into_bytes())
        .collect();
    let image = |seed: u8| -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend((0..=255u8).cycle().take(600).map(|b| b.wrapping_mul(seed)));
        bytes
    };
    let base = [
        ("del.rs", gone.clone()),
        ("old.rs", moved.clone()),
        ("img.png", image(3)),
        ("*.rs", source(false, false)),
        ("x.rs", source(false, false)),
    ];
    let (root, scratch) = repo("shapes", &base);
    let added = [b"pub fn other() {\n", SPAWN, b"}\n"].concat();
    let mut moved_new = moved.clone();
    moved_new.extend_from_slice(SPAWN);
    git(&root, &["mv", "old.rs", "moved.rs"]);
    std::fs::write(root.join("moved.rs"), &moved_new).unwrap();
    std::fs::write(root.join("add.rs"), &added).unwrap();
    git(&root, &["add", "add.rs"]);
    std::fs::remove_file(root.join("del.rs")).unwrap();
    std::fs::write(root.join("new.rs"), source(true, true)).unwrap();
    std::fs::write(root.join("img.png"), image(7)).unwrap();
    std::fs::write(root.join("*.rs"), source(false, true)).unwrap();
    let acq = acquire_git(&root, &GitSelection::WorkingTree, &options(true, 1 << 20)).unwrap();
    for path in ["add.rs", "moved.rs", "new.rs", "*.rs"] {
        read_as_text(&acq, path);
    }
    let deleted = change(&acq, "del.rs");
    assert!(!deleted.binary);
    assert!(
        String::from_utf8_lossy(acq.diff().change_patch(deleted)).contains("-const Z1: u8 = 1;")
    );
    assert!(change(&acq, "img.png").binary);
    let moved_change = change(&acq, "moved.rs");
    assert_eq!(
        moved_change.old_path,
        Some(GitPath::from_bytes(b"old.rs".to_vec()))
    );
    // the sections tile the patch in order, as Git's blocks do
    let mut end = 0;
    for c in acq.diff().changes() {
        assert_eq!(c.patch_range.start, end);
        end = c.patch_range.end;
    }
    assert_eq!(end, acq.diff().patch_bytes().len());
    // and the parser reads one file per block
    let captured = acq.diff().capture().unwrap();
    assert_eq!(captured.files().len(), acq.diff().changes().len());
    applies(
        &scratch,
        acq.diff().patch_bytes(),
        &base,
        &[
            ("add.rs", Some(added)),
            ("del.rs", None),
            ("old.rs", None),
            ("moved.rs", Some(moved_new)),
            ("new.rs", Some(source(true, true))),
            ("img.png", Some(image(7))),
            ("*.rs", Some(source(false, true))),
            ("x.rs", Some(source(false, false))),
        ],
    );
}

/// The text reading is kept only within what is left of the byte budget: a large new file whose
/// binary block fits and whose text does not stays binary, and the acquisition still succeeds.
/// With room, the same change is text.
#[test]
fn a_text_reading_over_the_byte_budget_leaves_the_binary_reading_standing() {
    let (root, _scratch) = repo("budget", &[("keep.txt", b"k\n".to_vec())]);
    let mut big = b"fn main() {\n".to_vec();
    for n in 0..5000 {
        big.extend_from_slice(format!("    let a{} = 1;\n", n % 7).as_bytes());
    }
    big.extend_from_slice(SPAWN);
    big.extend_from_slice(b"}\n");
    std::fs::write(root.join("big.rs"), &big).unwrap();
    git(&root, &["add", "big.rs"]);
    let tight = acquire_git(&root, &GitSelection::Staged, &options(false, 20_000)).unwrap();
    assert!(change(&tight, "big.rs").binary);
    assert!(tight.diff().patch_bytes().len() < 20_000);
    let roomy = acquire_git(&root, &GitSelection::Staged, &options(false, 1 << 20)).unwrap();
    read_as_text(&roomy, "big.rs");
    assert!(roomy.diff().patch_bytes().len() > 20_000);
}
