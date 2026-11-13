//! `judge_patch` against real repositories: the patch of a working tree is, byte for byte, the
//! patch forge-warden's patch maker (`scripts/make_patch.sh`) gives once that tree is committed;
//! making it changes nothing in the repository; and no configuration, attribute or filter of the
//! repository can change the bytes or run a program.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use deep_diff_forge_patch::{GitAcquireError, GitAcquireOptions, file_at_commit, judge_patch};
use deep_diff_forge_testkit::Scratch;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt as _;
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
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgSign=false",
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

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap().trim().to_owned()
}

fn options() -> GitAcquireOptions {
    GitAcquireOptions {
        env: ISOLATE
            .iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
            .collect(),
        ..GitAcquireOptions::default()
    }
}

/// A repository with one commit, then a working tree that changes a file, deletes one, changes a
/// binary, adds an untracked file with a space in its name and an executable in a new directory,
/// and adds an ignored file.
fn changed_repository(scratch: &Scratch) -> PathBuf {
    let root = scratch.join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("src.rs"), "fn a() {}\n").unwrap();
    std::fs::write(root.join("keep.txt"), "x\n").unwrap();
    std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
    std::fs::write(root.join("data.bin"), b"bin\0ary\n").unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    std::fs::write(root.join("src.rs"), "fn a() { b(); }\n").unwrap();
    std::fs::write(root.join("new name.txt"), "new file\n").unwrap();
    std::fs::remove_file(root.join("keep.txt")).unwrap();
    std::fs::write(root.join("data.bin"), b"bin\0ary2\n").unwrap();
    std::fs::write(root.join("x.log"), "ignored\n").unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/run.sh"), "echo hi\n").unwrap();
    std::fs::set_permissions(
        root.join("sub/run.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    root
}

/// What forge-warden's `scripts/make_patch.sh <base> <head>` writes: the diff of two commits in a
/// new, empty bare repository that only borrows the object store, with its options.
fn patch_maker(root: &Path, scratch: &Scratch, base: &str, head: &str) -> Vec<u8> {
    let empty_repository = scratch.join("maker.git");
    git(
        scratch.path(),
        &[
            "init",
            "--quiet",
            "--bare",
            empty_repository.to_str().unwrap(),
        ],
    );
    let objects = text(git(
        root,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "objects",
        ],
    ));
    std::fs::write(
        empty_repository.join("objects/info/alternates"),
        format!("{objects}\n"),
    )
    .unwrap();
    let empty = text(git(root, &["hash-object", "-t", "tree", "/dev/null"]));
    // The patch maker resolves both names in the repository first: the empty one has no refs.
    let base = text(git(root, &["rev-parse", "--verify", base]));
    let head = text(git(root, &["rev-parse", "--verify", head]));
    let out = Command::new("git")
        .current_dir(scratch.path())
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .arg(format!("--git-dir={}", empty_repository.display()))
        .arg(format!("--attr-source={empty}"))
        .args([
            "-c",
            "core.attributesFile=/dev/null",
            "-c",
            "core.quotePath=true",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--no-relative",
            "--binary",
            "--no-renames",
            "--src-prefix=a/",
            "--dst-prefix=b/",
        ])
        .arg(format!("{base}...{head}"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// What must not change in a repository while a patch is made of it.
fn untouched(root: &Path) -> (Vec<u8>, usize, Vec<u8>, String) {
    let objects = walk(&root.join(".git/objects"));
    (
        std::fs::read(root.join(".git/index")).unwrap(),
        objects,
        // `--no-optional-locks`: a plain `git status` may itself refresh the index.
        git(
            root,
            &[
                "--no-optional-locks",
                "status",
                "--porcelain=v1",
                "-z",
                "--ignored",
            ],
        ),
        text(git(root, &["config", "--local", "--list"])),
    )
}

fn walk(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            if path.is_dir() { walk(&path) } else { 1 }
        })
        .sum()
}

fn fresh(scratch: &Scratch, name: &str) -> PathBuf {
    let dir = scratch.join(name);
    std::fs::create_dir(&dir).unwrap();
    dir
}

#[test]
fn a_working_tree_gives_the_bytes_the_patch_maker_gives_for_the_commit() {
    let scratch = Scratch::new("judge-patch-equal");
    let root = changed_repository(&scratch);
    let before = untouched(&root);
    let made = judge_patch(&root, "HEAD", &fresh(&scratch, "work"), &options()).unwrap();
    assert_eq!(
        untouched(&root),
        before,
        "making the patch changed the repository"
    );
    assert_eq!(made.base, text(git(&root, &["rev-parse", "HEAD"])));

    let shown = String::from_utf8_lossy(&made.bytes);
    for path in [
        "data.bin",
        "keep.txt",
        "new name.txt",
        "src.rs",
        "sub/run.sh",
    ] {
        assert!(
            shown.contains(&format!("diff --git a/{path} b/{path}\n")),
            "{path} missing:\n{shown}"
        );
    }
    assert!(!shown.contains("x.log"), "an ignored file is in the patch");
    assert!(shown.contains("new file mode 100755\n"), "{shown}");
    assert!(shown.contains("deleted file mode 100644\n"), "{shown}");
    assert!(shown.contains("GIT binary patch\n"), "{shown}");

    // Now commit exactly that state and ask the patch maker.
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "change"]);
    assert_eq!(made.tree, text(git(&root, &["rev-parse", "HEAD^{tree}"])));
    let expected = patch_maker(&root, &scratch, "HEAD~1", "HEAD");
    assert_ne!(expected, b"", "the patch maker made no patch");
    assert_eq!(
        made.bytes,
        expected,
        "ours:\n{}\npatch maker:\n{}",
        shown,
        String::from_utf8_lossy(&expected)
    );
}

#[test]
fn a_clean_tree_gives_an_empty_patch_and_a_bad_base_is_refused() {
    let scratch = Scratch::new("judge-patch-clean");
    let root = changed_repository(&scratch);
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "change"]);
    let clean = judge_patch(&root, "HEAD", &fresh(&scratch, "a"), &options()).unwrap();
    assert_eq!(clean.bytes, b"");
    // an older base gives the whole change
    let since_base = judge_patch(&root, "HEAD~1", &fresh(&scratch, "b"), &options()).unwrap();
    assert_eq!(
        since_base.bytes,
        patch_maker(&root, &scratch, "HEAD~1", "HEAD")
    );
    for (n, bad) in ["no-such-rev", "HEAD^{tree}", "--output=/tmp/x"]
        .into_iter()
        .enumerate()
    {
        let work = fresh(&scratch, &format!("bad{n}"));
        assert!(
            matches!(
                judge_patch(&root, bad, &work, &options()),
                Err(GitAcquireError::InvalidSelection { .. })
            ),
            "{bad}"
        );
    }
}

/// Whoever controls the repository must not control the bytes a judge reads, or run a program
/// while they are made.
#[test]
fn repository_configuration_attributes_and_filters_cannot_change_the_patch() {
    let scratch = Scratch::new("judge-patch-hostile");
    let root = changed_repository(&scratch);
    let plain = judge_patch(&root, "HEAD", &fresh(&scratch, "plain"), &options()).unwrap();

    let marker = scratch.join("filter-ran");
    for (key, value) in [
        ("diff.noprefix", "true"),
        ("diff.mnemonicPrefix", "true"),
        ("diff.renames", "copies"),
        ("diff.context", "0"),
        ("diff.relative", "true"),
        ("core.quotePath", "false"),
        ("color.diff", "always"),
        ("diff.external", "false"),
        (
            "filter.evil.clean",
            &format!("sh -c 'touch {}; cat'", marker.display()),
        ),
    ] {
        git(&root, &["config", "--local", key, value]);
    }
    // an attribute that would hide the source change as binary, and name the filter
    std::fs::write(
        root.join(".git/info/attributes"),
        "*.rs -diff filter=evil\n*.txt binary\n",
    )
    .unwrap();
    let hostile = judge_patch(&root, "HEAD", &fresh(&scratch, "hostile"), &options()).unwrap();
    assert!(!marker.exists(), "a repository-defined filter ran");
    assert_eq!(
        String::from_utf8_lossy(&hostile.bytes),
        String::from_utf8_lossy(&plain.bytes)
    );
    // and from a subdirectory, with the same result
    let sub = judge_patch(
        &root.join("sub"),
        "HEAD",
        &fresh(&scratch, "sub"),
        &options(),
    )
    .unwrap();
    assert_eq!(sub.bytes, plain.bytes);
}

#[test]
fn a_patch_over_the_byte_budget_is_refused_not_cut() {
    let scratch = Scratch::new("judge-patch-budget");
    let root = changed_repository(&scratch);
    let small = GitAcquireOptions {
        byte_budget: 64,
        ..options()
    };
    assert!(matches!(
        judge_patch(&root, "HEAD", &fresh(&scratch, "work"), &small),
        Err(GitAcquireError::BudgetExceeded { limit_bytes: 64 })
    ));
}

/// A policy is read from the commit, never from the working tree, and only when it is a
/// regular file there.
#[test]
fn a_file_is_read_from_the_commit_not_from_the_working_tree() {
    let scratch = Scratch::new("judge-patch-file-at");
    let root = changed_repository(&scratch);
    // committed content, although the working tree has changed it
    assert_eq!(
        file_at_commit(&root, "HEAD", "src.rs", &options()).unwrap(),
        Some(b"fn a() {}\n".to_vec())
    );
    // deleted in the working tree, still in the commit
    assert_eq!(
        file_at_commit(&root, "HEAD", "keep.txt", &options()).unwrap(),
        Some(b"x\n".to_vec())
    );
    // only in the working tree, a directory, a name with pattern characters, a missing name
    for absent in [
        "new name.txt",
        "sub",
        "*.rs",
        "[a-z]rc.rs",
        "no/such/file",
        "",
    ] {
        assert_eq!(
            file_at_commit(&root, "HEAD", absent, &options()).unwrap(),
            None,
            "{absent:?}"
        );
    }
    // from a subdirectory the path is still relative to the top level
    std::fs::create_dir_all(root.join("sub")).unwrap();
    assert_eq!(
        file_at_commit(&root.join("sub"), "HEAD", "src.rs", &options()).unwrap(),
        Some(b"fn a() {}\n".to_vec())
    );
    assert!(matches!(
        file_at_commit(&root, "no-such-rev", "src.rs", &options()),
        Err(GitAcquireError::InvalidSelection { .. })
    ));
}
