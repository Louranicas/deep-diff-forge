//! `judge_patch` against real repositories: the patch of a working tree is, byte for byte, the
//! patch forge-warden's patch maker (`scripts/make_patch.sh`) gives once that tree is committed;
//! making it changes nothing in the repository; and no configuration, attribute or filter of the
//! repository can change the bytes or run a program.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use deep_diff_forge_patch::{
    GitAcquireError, GitAcquireOptions, JudgePatchError, file_at_commit, judge_patch,
};
use deep_diff_forge_testkit::Scratch;
use std::ffi::{OsStr, OsString};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

const ISOLATE: [(&str, &str); 2] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

/// Git for a test's own steps. `maintenance.auto=false`: `git commit` starts a detached `git
/// maintenance run --auto`, which holds `objects/maintenance.lock` for a moment after the
/// commit has returned; [`untouched`] counts the files under `objects`, so on a busy machine a
/// count taken right after a commit was one more than the count taken later, and "making the
/// patch changed the repository" failed with nothing changed (22 of 400 runs beside another
/// test suite; none of 400 with this).
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
            "-c",
            "maintenance.auto=false",
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

/// forge-warden's own patch maker, `scripts/make_patch.sh` of its main at `fbd621e0`, recorded
/// byte for byte beside the hook's other forge-warden fixtures, and its SHA-256.
const MAKE_PATCH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../deep-diff-forge-cli/tests/fixtures/forge-warden-main-fbd621e0/make_patch.sh"
);
const MAKE_PATCH_SHA256: &str = "f53c440c2bae2aab26261ec01535a9083c72e67591176ec6d64b3c5093b5d7f3";

/// What forge-warden's `scripts/make_patch.sh <base> <head> <repository>` writes, from the
/// recorded copy of the script itself. This was a hand copy of the maker's options, older than
/// the maker: it passed while the judge's patch had short index lines and no
/// `--submodule=short` or `--ignore-submodules=none` (S3.17a.h).
fn patch_maker(root: &Path, scratch: &Scratch, base: &str, head: &str) -> Vec<u8> {
    let script = std::fs::read(MAKE_PATCH).unwrap();
    assert_eq!(
        deep_diff_forge_core::sha256_hex(&script),
        MAKE_PATCH_SHA256,
        "{MAKE_PATCH} is not the copy recorded from forge-warden's main fbd621e0"
    );
    let temporary = scratch.join(format!(
        "maker-tmp-{}",
        text(git(root, &["rev-parse", head]))
    ));
    std::fs::create_dir_all(&temporary).unwrap();
    let out = Command::new("bash")
        .arg(MAKE_PATCH)
        .arg(base)
        .arg(head)
        .arg(root)
        .current_dir(scratch.path())
        .env("TMPDIR", &temporary)
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

/// S3.17a.h: the patch the hook gives the judge is the bytes forge-warden's own patch maker
/// writes for the same change, run from the recorded copy of its script: a text file, a
/// binary file and a submodule's commit changed. The hook's options lacked `--full-index`,
/// `--submodule=short` and `--ignore-submodules=none`, so every index line differed from the
/// maker's (`index 7898192..6178079` against forty hexadecimal digits each).
#[test]
fn the_patch_is_the_bytes_forge_wardens_make_patch_writes() {
    use deep_diff_forge_patch::commit_patch;
    let scratch = Scratch::new("judge-patch-maker-parity");
    let root = fresh(&scratch, "repo");
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("src.rs"), "fn a() {}\n").unwrap();
    std::fs::write(root.join("data.bin"), b"bin\0ary\n").unwrap();
    git(&root, &["add", "src.rs", "data.bin"]);
    git(&root, &["commit", "--quiet", "-m", "first"]);
    let first = text(git(&root, &["rev-parse", "HEAD"]));
    // A submodule as the index holds it: a gitlink, at a commit of its own.
    let gitlink = |at: &str| {
        git(
            &root,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{at},vendor/lib"),
            ],
        );
    };
    gitlink(&first);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    let base = text(git(&root, &["rev-parse", "HEAD"]));
    std::fs::write(root.join("src.rs"), "fn a() { b(); }\n").unwrap();
    std::fs::write(root.join("data.bin"), b"bin\0ary2\n").unwrap();
    git(&root, &["add", "src.rs", "data.bin"]);
    gitlink(&base);
    git(&root, &["commit", "--quiet", "-m", "change"]);
    let head = text(git(&root, &["rev-parse", "HEAD"]));

    let theirs = patch_maker(&root, &scratch, &base, &head);
    let shown = String::from_utf8_lossy(&theirs).into_owned();
    let blob = |at: &str, path: &str| text(git(&root, &["rev-parse", &format!("{at}:{path}")]));
    for line in [
        format!(
            "index {}..{} 100644\n",
            blob(&base, "src.rs"),
            blob(&head, "src.rs")
        ),
        format!(
            "index {}..{} 100644\n",
            blob(&base, "data.bin"),
            blob(&head, "data.bin")
        ),
        format!("index {first}..{base} 160000\n"),
        "GIT binary patch\n".to_owned(),
        format!("+Subproject commit {base}\n"),
    ] {
        assert!(
            shown.contains(&line),
            "the maker wrote no {line:?}:\n{shown}"
        );
    }
    let ours = commit_patch(
        &root,
        Some(&base),
        &head,
        &fresh(&scratch, "ours"),
        &options(),
    )
    .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&ours),
        shown,
        "the judge's patch is not the bytes forge-warden's make_patch.sh writes"
    );
    assert_eq!(ours, theirs);
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
                Err(JudgePatchError::NoBase)
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

/// Review V4, VD4-M-2. Staging in the repository's own Git directory read its
/// `info/attributes`: one line there changed the bytes a judge was given, though nothing in the
/// change said so. Nothing under the Git directory but the object store and `info/exclude`
/// shapes the patch now; a `.gitattributes` in the working tree still does, and is in the patch.
#[test]
fn only_the_working_tree_and_the_ignore_rules_shape_the_patch() {
    let scratch = Scratch::new("judge-patch-local-state");
    let root = scratch.join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("file.txt"), "base\n").unwrap();
    std::fs::write(root.join("notes.md"), "a\n").unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    std::fs::write(root.join("file.txt"), "changed\r\n").unwrap();
    std::fs::write(root.join("notes.md"), "b\r\n").unwrap();
    std::fs::write(root.join("local.txt"), "kept out by info/exclude\n").unwrap();
    std::fs::write(root.join(".git/info/exclude"), "local.txt\n").unwrap();
    let before = untouched(&root);
    let plain = judge_patch(&root, "HEAD", &fresh(&scratch, "plain"), &options()).unwrap();
    let shown = String::from_utf8_lossy(&plain.bytes).into_owned();
    assert!(
        shown.contains("+changed\r\n") && shown.contains("+b\r\n"),
        "{shown}"
    );
    assert!(
        !shown.contains("local.txt"),
        "info/exclude was not honoured: {shown}"
    );

    // Vesper's reproduction, and the configuration that does the same to every text file
    std::fs::write(root.join(".git/info/attributes"), "*.txt text eol=lf\n").unwrap();
    git(&root, &["config", "--local", "core.autocrlf", "input"]);
    let local = judge_patch(&root, "HEAD", &fresh(&scratch, "local"), &options()).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&local.bytes),
        shown,
        "state under the Git directory changed the patch"
    );
    assert_eq!(local.tree, plain.tree);
    std::fs::remove_file(root.join(".git/info/attributes")).unwrap();
    git(&root, &["config", "--local", "--unset", "core.autocrlf"]);
    assert_eq!(
        untouched(&root),
        before,
        "making the patch changed the repository"
    );

    // The same rule in the working tree is part of the change: it converts, and the patch
    // shows the file that says so.
    std::fs::write(root.join(".gitattributes"), "*.txt text eol=lf\n").unwrap();
    let in_tree = judge_patch(&root, "HEAD", &fresh(&scratch, "in-tree"), &options()).unwrap();
    let shown = String::from_utf8_lossy(&in_tree.bytes).into_owned();
    assert!(
        shown.contains("+changed\n") && !shown.contains("+changed\r"),
        "{shown}"
    );
    assert!(
        shown.contains("+b\r\n"),
        "a file the rule does not name was converted: {shown}"
    );
    assert!(
        shown.contains("diff --git a/.gitattributes b/.gitattributes\n"),
        "{shown}"
    );
    // and that is the commit: with the rule in the tree, a commit records the same
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "change"]);
    assert_eq!(
        in_tree.tree,
        text(git(&root, &["rev-parse", "HEAD^{tree}"]))
    );
    assert_eq!(
        in_tree.bytes,
        patch_maker(&root, &scratch, "HEAD~1", "HEAD")
    );
}

/// Review V4 round 2, VD4R2-M-1. In a sparse checkout the files outside the pattern are absent
/// on purpose. A tree staged from the working tree showed each of them as deleted. Such a
/// working tree is refused by name, and so is one where a path was marked by hand; a failure
/// is not the same answer as "no base".
#[test]
fn a_working_tree_with_skipped_paths_is_refused_by_name() {
    let scratch = Scratch::new("judge-patch-sparse");
    let root = scratch.join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("kept.txt"), "kept base\n").unwrap();
    std::fs::write(root.join("hidden.txt"), "hidden base\n").unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    // control: an ordinary change is a patch of one file
    std::fs::write(root.join("kept.txt"), "kept changed\n").unwrap();
    let plain = judge_patch(&root, "HEAD", &fresh(&scratch, "plain"), &options()).unwrap();
    let shown = String::from_utf8_lossy(&plain.bytes).into_owned();
    assert!(
        shown.contains("kept.txt") && !shown.contains("hidden.txt"),
        "{shown}"
    );

    // Vesper's reproduction: a sparse checkout without `hidden.txt`
    git(&root, &["sparse-checkout", "set", "--no-cone", "/kept.txt"]);
    assert!(
        !root.join("hidden.txt").exists(),
        "the checkout is not sparse"
    );
    let before = untouched(&root);
    assert!(matches!(
        judge_patch(&root, "HEAD", &fresh(&scratch, "sparse"), &options()),
        Err(JudgePatchError::SkipWorktree)
    ));
    assert_eq!(
        untouched(&root),
        before,
        "the refusal changed the repository"
    );
    git(&root, &["sparse-checkout", "disable"]);
    assert!(judge_patch(&root, "HEAD", &fresh(&scratch, "full"), &options()).is_ok());

    // a path marked by hand
    git(&root, &["update-index", "--skip-worktree", "hidden.txt"]);
    assert!(matches!(
        judge_patch(&root, "HEAD", &fresh(&scratch, "marked"), &options()),
        Err(JudgePatchError::SkipWorktree)
    ));
    assert!(
        JudgePatchError::SkipWorktree
            .to_string()
            .contains("skip-worktree"),
        "the refusal does not name its reason"
    );
    assert!(matches!(
        JudgePatchError::from(GitAcquireError::InvalidSelection {
            detail: "x".to_owned()
        }),
        JudgePatchError::Failed(_)
    ));
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
        Err(JudgePatchError::Failed(GitAcquireError::BudgetExceeded {
            limit_bytes: 64
        }))
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

/// Independent review of the hooks, HR-M-2. The scratch index was filled with `git add --all`,
/// which leaves out what an ignore rule names, and the repository's own index was never read.
/// A file under an ignored pattern that was added with `git add -f` is in the next commit and
/// was not in the patch. Both of the reviewer's cases: a rule in `.gitignore`, and one in
/// `.git/info/exclude`.
#[test]
fn a_file_in_the_index_is_in_the_patch_whatever_an_ignore_rule_says() {
    let scratch = Scratch::new("judge-patch-forced");
    let root = scratch.join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("src.rs"), "fn a() {}\n").unwrap();
    std::fs::write(root.join(".gitignore"), "build/\n").unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    let shown = |name: &str| {
        let patch = judge_patch(&root, "HEAD", &fresh(&scratch, name), &options()).unwrap();
        String::from_utf8_lossy(&patch.bytes).into_owned()
    };

    std::fs::create_dir(root.join("build")).unwrap();
    std::fs::write(root.join("build/gen.rs"), "const K: u8 = 1;\n").unwrap();
    std::fs::write(root.join("build/odd name é.rs"), "const L: u8 = 2;\n").unwrap();
    std::fs::write(root.join("build/never-added.rs"), "const M: u8 = 3;\n").unwrap();
    std::fs::write(root.join("local.rs"), "const N: u8 = 4;\n").unwrap();
    std::fs::write(root.join(".git/info/exclude"), "local.rs\n").unwrap();
    // ignored and in no index: in no commit, and in no patch
    assert_eq!(shown("ignored"), "");

    git(&root, &["add", "-f", "build/gen.rs", "build/odd name é.rs"]);
    git(&root, &["add", "-f", "local.rs"]);
    let before = untouched(&root);
    let forced = judge_patch(&root, "HEAD", &fresh(&scratch, "forced"), &options()).unwrap();
    let text = String::from_utf8_lossy(&forced.bytes).into_owned();
    for added in [
        "diff --git a/build/gen.rs b/build/gen.rs\nnew file mode 100644\n",
        "+const K: u8 = 1;\n",
        "+const L: u8 = 2;\n",
        "diff --git a/local.rs b/local.rs\nnew file mode 100644\n",
        "+const N: u8 = 4;\n",
    ] {
        assert!(text.contains(added), "{added:?} is missing: {text}");
    }
    assert!(!text.contains("never-added"), "{text}");
    assert_eq!(
        untouched(&root),
        before,
        "making the patch changed the repository"
    );
    // The file as the working tree has it now, not as it was when it was added.
    std::fs::write(root.join("build/gen.rs"), "const K: u8 = 9;\n").unwrap();
    let later = shown("later");
    assert!(
        later.contains("+const K: u8 = 9;\n") && !later.contains("u8 = 1;"),
        "{later}"
    );
    std::fs::write(root.join("build/gen.rs"), "const K: u8 = 1;\n").unwrap();

    // and that is the commit: the bytes are the patch maker's for it
    git(&root, &["commit", "--quiet", "-m", "generated files"]);
    assert_eq!(forced.tree, text_of_tree(&root));
    assert_eq!(forced.bytes, patch_maker(&root, &scratch, "HEAD~1", "HEAD"));

    // A file that is in the index and gone from the working tree is not staged by name: the
    // working tree is what is judged, and a path that is not there must not stop the patch.
    std::fs::write(root.join("build/short-lived.rs"), "const S: u8 = 5;\n").unwrap();
    git(&root, &["add", "-f", "build/short-lived.rs"]);
    std::fs::remove_file(root.join("build/short-lived.rs")).unwrap();
    assert_eq!(shown("gone"), "");
}

fn text_of_tree(root: &Path) -> String {
    text(git(root, &["rev-parse", "HEAD^{tree}"]))
}

/// Independent review of the hooks, HR-L-5. The README said the tree was staged "with no
/// configuration"; the user's and the system's were read, and a filter the user's
/// configuration defined ran on every staged file, so the judge was given its output in place
/// of the file. Staging reads neither now, nor an attributes file of the user. One thing of
/// the user's is kept, passed by hand: the ignore file, so that what the user ignores in every
/// repository does not turn up in a patch.
#[test]
fn the_users_configuration_does_not_shape_the_patch_and_the_users_ignore_file_does() {
    let scratch = Scratch::new("judge-patch-user-config");
    let root = changed_repository(&scratch);
    // a file with CRLF line ends: `core.autocrlf` in the user's configuration would stage it
    // with others, and needs no attribute to do so (a mutation run left the environment of
    // the staging alive without its isolation)
    std::fs::write(root.join("crlf.txt"), "one\r\ntwo\r\n").unwrap();
    let plain = judge_patch(&root, "HEAD", &fresh(&scratch, "plain"), &options()).unwrap();
    let shown = String::from_utf8_lossy(&plain.bytes).into_owned();
    assert!(shown.contains("+fn a() { b(); }\n"), "{shown}");
    assert!(shown.contains("+one\r\n"), "{shown}");

    // the reviewer's case: a filter in the user's configuration, and a user attributes file
    // that names it for every source file (and a conversion for every text file)
    let marker = scratch.join("filter-ran");
    let attributes = scratch.write("user-attributes", "*.rs filter=hide\n*.txt text eol=crlf\n");
    let user_config = |more: &str| {
        scratch.write(
            "user-gitconfig",
            format!(
                "[filter \"hide\"]\n\tclean = \"sh -c 'touch {}; echo cleaned'\"\n\trequired = true\n[core]\n\tattributesFile = {}\n{more}",
                marker.display(),
                attributes.display()
            ),
        )
    };
    let with = |config: &Path| GitAcquireOptions {
        env: vec![
            (OsString::from("GIT_CONFIG_GLOBAL"), config.into()),
            (OsString::from("GIT_CONFIG_NOSYSTEM"), OsString::from("1")),
        ],
        ..GitAcquireOptions::default()
    };
    let filtered = judge_patch(
        &root,
        "HEAD",
        &fresh(&scratch, "filtered"),
        &with(&user_config("")),
    )
    .unwrap();
    assert!(!marker.exists(), "a filter of the user's configuration ran");
    assert_eq!(
        String::from_utf8_lossy(&filtered.bytes),
        shown,
        "the user's configuration changed the patch"
    );
    assert_eq!(filtered.tree, plain.tree);
    // nor does the diff itself: the lines of context and the order of the files are not the
    // user's to choose (a mutation run left the environment of what runs outside the
    // repository alive when emptied; and again on `07df120`, where the change's context was
    // the same with no lines of it: an order file of the user's reverses the files)
    let order = scratch.write("user-order", "new name.txt\ncrlf.txt\n");
    let narrow = judge_patch(
        &root,
        "HEAD",
        &fresh(&scratch, "narrow"),
        &with(&user_config(&format!(
            "\tautocrlf = true\n[diff]\n\tcontext = 0\n\torderFile = {}\n",
            order.display()
        ))),
    )
    .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&narrow.bytes),
        shown,
        "the user's configuration changed the diff"
    );

    // the user's ignore file keeps a file out, as it does for a commit
    assert!(shown.contains("b/new name.txt"), "{shown}");
    let ignore = scratch.write("user-ignore", "new name.txt\n");
    let ignoring = judge_patch(
        &root,
        "HEAD",
        &fresh(&scratch, "ignoring"),
        &with(&user_config(&format!(
            "\texcludesFile = {}\n",
            ignore.display()
        ))),
    )
    .unwrap();
    let without = String::from_utf8_lossy(&ignoring.bytes).into_owned();
    assert!(!without.contains("new name.txt"), "{without}");
    assert!(without.contains("+fn a() { b(); }\n"), "{without}");
    assert!(!marker.exists(), "a filter of the user's configuration ran");

    // U1 row S258 (lane T3a of 0.11.1): only an absolute ignore file is passed on. A relative
    // one would be read from wherever Git runs, here the working tree, whose own file then
    // shapes the patch; it is not passed on, and the file is judged like any other.
    std::fs::write(root.join("relative-ignore"), "new name.txt\n").unwrap();
    let relative = judge_patch(
        &root,
        "HEAD",
        &fresh(&scratch, "relative"),
        &with(&user_config("\texcludesFile = relative-ignore\n")),
    )
    .unwrap();
    let relative = String::from_utf8_lossy(&relative.bytes).into_owned();
    assert!(
        relative.contains("b/new name.txt") && relative.contains("b/relative-ignore"),
        "a relative core.excludesFile of the user's shaped the patch: {relative}"
    );
    std::fs::remove_file(root.join("relative-ignore")).unwrap();
}

/// U1 rows S256 and S261 (lane T3a of 0.11.1). The tests above plant the repository's
/// configuration and the user's file; these are the other ways configuration reaches Git, each
/// on its own: the system's file (`GIT_CONFIG_SYSTEM`, with `GIT_CONFIG_NOSYSTEM` not set),
/// configuration a parent Git passes down (`GIT_CONFIG_PARAMETERS`, and `GIT_CONFIG_COUNT` with
/// its keys), and the user's own attributes file (`$XDG_CONFIG_HOME/git/attributes`, read when
/// no `core.attributesFile` is set). Each would normalise a file with CRLF line ends or cut
/// the diff's context; the bytes must be the plain ones.
#[test]
fn configuration_of_the_system_a_parent_git_or_the_users_attributes_does_not_shape_the_patch() {
    let scratch = Scratch::new("judge-patch-inherited-config");
    let root = changed_repository(&scratch);
    std::fs::write(root.join("crlf.txt"), "one\r\ntwo\r\n").unwrap();
    let plain = judge_patch(&root, "HEAD", &fresh(&scratch, "plain"), &options()).unwrap();
    let shown = String::from_utf8_lossy(&plain.bytes).into_owned();
    assert!(shown.contains("+one\r\n"), "{shown}");
    let system = scratch.write(
        "system-gitconfig",
        "[core]\n\tautocrlf = true\n[diff]\n\tcontext = 0\n",
    );
    let xdg = scratch.join("xdg");
    std::fs::create_dir_all(xdg.join("git")).unwrap();
    std::fs::write(xdg.join("git/attributes"), "*.txt text\n").unwrap();
    let env = |pairs: &[(&str, &OsStr)]| GitAcquireOptions {
        env: [("GIT_CONFIG_GLOBAL", OsStr::new("/dev/null"))]
            .iter()
            .chain(pairs)
            .map(|&(k, v)| (OsString::from(k), v.to_os_string()))
            .collect(),
        ..GitAcquireOptions::default()
    };
    for (case, options) in [
        (
            "the system's configuration",
            env(&[("GIT_CONFIG_SYSTEM", system.as_os_str())]),
        ),
        (
            "GIT_CONFIG_PARAMETERS of a parent Git",
            env(&[
                ("GIT_CONFIG_NOSYSTEM", OsStr::new("1")),
                (
                    "GIT_CONFIG_PARAMETERS",
                    OsStr::new("'core.autocrlf'='true' 'diff.context'='0'"),
                ),
            ]),
        ),
        (
            "GIT_CONFIG_COUNT of a parent Git",
            env(&[
                ("GIT_CONFIG_NOSYSTEM", OsStr::new("1")),
                ("GIT_CONFIG_COUNT", OsStr::new("2")),
                ("GIT_CONFIG_KEY_0", OsStr::new("core.autocrlf")),
                ("GIT_CONFIG_VALUE_0", OsStr::new("true")),
                ("GIT_CONFIG_KEY_1", OsStr::new("diff.context")),
                ("GIT_CONFIG_VALUE_1", OsStr::new("0")),
            ]),
        ),
        (
            "the user's attributes file",
            env(&[
                ("GIT_CONFIG_NOSYSTEM", OsStr::new("1")),
                ("XDG_CONFIG_HOME", xdg.as_os_str()),
            ]),
        ),
    ] {
        let made = judge_patch(&root, "HEAD", &fresh(&scratch, case), &options).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&made.bytes),
            shown,
            "{case} changed the patch"
        );
    }
}

/// Round 11 of the review of the hooks. A directory in the working tree that is a Git
/// repository of its own is no file of the project. `git add --all` fails on one that has no
/// commit (`git init scratch/`), so every patch of such a tree failed and every stop was
/// UNJUDGED; one that has a commit was staged as a link to it. Both are left out of the staged
/// tree and named, whatever their names; one the index holds (a submodule) stays.
#[test]
fn nested_repositories_are_left_out_of_the_patch_and_named() {
    let scratch = Scratch::new("judge-patch-nested");
    let root = scratch.join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("src.rs"), "fn a() {}\n").unwrap();
    std::fs::create_dir(root.join("dir")).unwrap();
    std::fs::write(root.join("dir/kept.txt"), "x\n").unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    let nested = |path: &str, commit: bool| {
        let dir = root.join(path);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "--quiet"]);
        std::fs::write(dir.join("inner.rs"), "const INNER: u8 = 1;\n").unwrap();
        if commit {
            git(&dir, &["add", "--all"]);
            git(&dir, &["commit", "--quiet", "-m", "inner"]);
        }
    };
    nested("unborn", false);
    nested("dir/with commit", true);
    nested("odd [1]*?", false);
    nested("plain/deeper/repo", true);
    std::fs::write(root.join("plain/file.rs"), "const OUTER: u8 = 2;\n").unwrap();
    std::fs::write(root.join("src.rs"), "fn a() { b(); }\n").unwrap();
    // a directory whose name a careless rule for `odd [1]*?` would also match
    std::fs::create_dir(root.join("odd 1xy")).unwrap();
    std::fs::write(root.join("odd 1xy/file.rs"), "const NEAR: u8 = 3;\n").unwrap();

    let before = untouched(&root);
    let patch = judge_patch(&root, "HEAD", &fresh(&scratch, "nested"), &options()).unwrap();
    assert_eq!(
        untouched(&root),
        before,
        "making the patch changed the repository"
    );
    let named = |nested: &[std::path::PathBuf]| -> Vec<String> {
        let mut named: Vec<String> = nested
            .iter()
            .map(|path| path.to_str().unwrap().to_owned())
            .collect();
        named.sort();
        named
    };
    let left_out = named(&patch.nested);
    assert_eq!(
        left_out,
        [
            "dir/with commit",
            "odd [1]*?",
            "plain/deeper/repo",
            "unborn"
        ]
    );
    let shown = String::from_utf8_lossy(&patch.bytes).into_owned();
    for part in [
        "+fn a() { b(); }\n",
        "diff --git a/plain/file.rs b/plain/file.rs\n",
        "+const OUTER: u8 = 2;\n",
        "+const NEAR: u8 = 3;\n",
    ] {
        assert!(shown.contains(part), "{part:?} is missing: {shown}");
    }
    for part in ["INNER", "160000", "unborn", "with commit", "deeper"] {
        assert!(!shown.contains(part), "{part:?} is in the patch: {shown}");
    }

    // a tree with none names none
    let other = Scratch::new("judge-patch-nested-none");
    let plain = changed_repository(&other);
    let patch = judge_patch(&plain, "HEAD", &fresh(&scratch, "none"), &options()).unwrap();
    assert_eq!(patch.nested, Vec::<std::path::PathBuf>::new());

    // A nested repository the index holds is the project's (a submodule): it stays in the
    // tree, as the link a commit would record.
    git(&root, &["add", "dir/with commit"]);
    let patch = judge_patch(&root, "HEAD", &fresh(&scratch, "indexed"), &options()).unwrap();
    let shown = String::from_utf8_lossy(&patch.bytes).into_owned();
    assert!(
        shown.contains("new file mode 160000\n") && shown.contains("b/dir/with commit"),
        "{shown}"
    );
    assert_eq!(
        named(&patch.nested),
        ["odd [1]*?", "plain/deeper/repo", "unborn"]
    );
}

/// Second independent review of the hooks, HR2-H-1, at the patch: what the session committed
/// is judged from the commits themselves. The patch between two commits has the bytes the
/// patch of a working tree has when the tree is what the second commit holds, so the two are
/// one patch for a judge; and they are the patch maker's bytes.
#[test]
fn the_patch_of_commits_is_the_patch_of_the_tree_they_hold() {
    use deep_diff_forge_patch::commit_patch;
    let scratch = Scratch::new("judge-patch-commits");
    let root = changed_repository(&scratch);
    let base = text(git(&root, &["rev-parse", "HEAD"]));
    let tree = judge_patch(&root, "HEAD", &fresh(&scratch, "tree"), &options()).unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "work"]);
    let head = text(git(&root, &["rev-parse", "HEAD"]));
    let before = untouched(&root);
    let commits = commit_patch(
        &root,
        Some(&base),
        &head,
        &fresh(&scratch, "commits"),
        &options(),
    )
    .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&commits),
        String::from_utf8_lossy(&tree.bytes)
    );
    assert_eq!(commits, patch_maker(&root, &scratch, &base, &head));
    assert_eq!(
        untouched(&root),
        before,
        "making the patch changed the repository"
    );
    // the same commit twice is no change; with nothing to compare with, everything is added
    assert_eq!(
        commit_patch(
            &root,
            Some(&head),
            &head,
            &fresh(&scratch, "same"),
            &options()
        )
        .unwrap(),
        b""
    );
    let whole = commit_patch(&root, None, &base, &fresh(&scratch, "whole"), &options()).unwrap();
    let shown = String::from_utf8_lossy(&whole).into_owned();
    assert!(
        shown.contains("diff --git a/src.rs b/src.rs\nnew file mode 100644\n")
            && shown.contains("+fn a() {}\n"),
        "{shown}"
    );
    // a name that is no commit is refused by name, not read as "no change"
    for (from, to) in [
        (Some("no-such-commit"), head.as_str()),
        (Some(base.as_str()), "no-such"),
    ] {
        assert!(matches!(
            commit_patch(
                &root,
                from,
                to,
                &fresh(&scratch, &format!("bad-{to}")),
                &options()
            ),
            Err(JudgePatchError::NoBase)
        ));
    }
}

/// HR2-H-1, what names a session's commits: branches, remote-tracking refs, tags and the
/// stash, each with the commit it leads to; which of them lead to commits that were not there
/// before; and where the change of such a commit begins.
#[test]
fn the_refs_that_name_commits_and_what_lies_beyond_the_known_ones() {
    use deep_diff_forge_patch::{HeadState, has_commits_beyond, head_state, merge_base, ref_tips};
    let scratch = Scratch::new("judge-patch-refs");
    let root = changed_repository(&scratch);
    let base = text(git(&root, &["rev-parse", "HEAD"]));
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "work"]);
    let head = text(git(&root, &["rev-parse", "HEAD"]));
    // what names commits: branches, remote-tracking refs, tags (a tag is followed to its
    // commit) and the stash, in the order of their names, at most as many as asked for
    git(&root, &["branch", "side", &base]);
    git(&root, &["tag", "-a", "v1", "-m", "v1", &base]);
    git(&root, &["tag", "light"]);
    git(&root, &["update-ref", "refs/remotes/origin/main", &head]);
    git(&root, &["update-ref", "refs/notes/x", &head]);
    std::fs::write(root.join("src.rs"), "fn stashed() {}\n").unwrap();
    git(&root, &["stash", "--quiet"]);
    let stash = text(git(&root, &["rev-parse", "refs/stash"]));
    let tips = ref_tips(&root, 100, &options()).unwrap();
    assert!(!tips.partial);
    let named = |name: &str| {
        tips.tips
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, commit)| commit.as_str())
    };
    assert_eq!(named("refs/heads/main"), Some(head.as_str()));
    assert_eq!(named("refs/heads/side"), Some(base.as_str()));
    assert_eq!(named("refs/tags/v1"), Some(base.as_str()));
    assert_eq!(named("refs/tags/light"), Some(head.as_str()));
    assert_eq!(named("refs/remotes/origin/main"), Some(head.as_str()));
    assert_eq!(named("refs/stash"), Some(stash.as_str()));
    assert_eq!(named("refs/notes/x"), None);
    assert_eq!(tips.tips.len(), 6);
    let few = ref_tips(&root, 2, &options()).unwrap();
    assert!(few.partial);
    assert_eq!(few.tips.len(), 2);
    assert_eq!(few.tips, tips.tips[..2]);

    // which of them lead to commits that were not there, and where their change begins
    let known = vec![base.clone()];
    assert!(has_commits_beyond(&root, &head, &known, &options()).unwrap());
    assert!(has_commits_beyond(&root, &stash, &known, &options()).unwrap());
    assert!(!has_commits_beyond(&root, &base, &known, &options()).unwrap());
    assert!(!has_commits_beyond(&root, &head, std::slice::from_ref(&head), &options()).unwrap());
    // a known commit the repository no longer has is passed over, not an error
    let gone = "0".repeat(39) + "1";
    assert!(has_commits_beyond(&root, &head, &[gone, base.clone()], &options()).unwrap());
    assert_eq!(
        merge_base(&root, &base, &stash, &options()).unwrap(),
        Some(base.clone())
    );
    git(&root, &["checkout", "--quiet", "--orphan", "apart"]);
    assert_eq!(
        head_state(&root, &options()).unwrap(),
        HeadState::Unborn {
            repository_has_commits: true
        }
    );
    git(
        &root,
        &["commit", "--quiet", "--allow-empty", "-m", "apart"],
    );
    let apart = text(git(&root, &["rev-parse", "HEAD"]));
    assert_eq!(merge_base(&root, &base, &apart, &options()).unwrap(), None);
    assert!(merge_base(&root, &base, "no-such-commit", &options()).is_err());
}

/// Second independent review of the hooks, HR2-M-3, at the patch crate: a submodule whose own
/// working tree has changes that are not committed in it. The outer repository's patch shows a
/// submodule only as the commit it is at, so these are found and named apart.
#[test]
fn submodules_with_changes_that_are_not_committed_are_found() {
    use deep_diff_forge_patch::dirty_submodules;
    let scratch = Scratch::new("judge-patch-submodules");
    let inner = scratch.join("inner");
    std::fs::create_dir(&inner).unwrap();
    git(&inner, &["init", "--quiet"]);
    std::fs::write(inner.join("s.rs"), "fn s() {}\n").unwrap();
    git(&inner, &["add", "--all"]);
    git(&inner, &["commit", "--quiet", "-m", "inner"]);
    let root = scratch.join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    assert_eq!(
        dirty_submodules(&root, &options()).unwrap(),
        Vec::<String>::new()
    );
    for path in ["vendor/one", "vendor/t w o"] {
        git(
            &root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "--quiet",
                "add",
                inner.to_str().unwrap(),
                path,
            ],
        );
    }
    git(&root, &["commit", "--quiet", "-m", "submodules"]);
    assert_eq!(
        dirty_submodules(&root, &options()).unwrap(),
        Vec::<String>::new()
    );
    // a tracked file changed in one, an untracked file added in the other
    std::fs::write(root.join("vendor/one/s.rs"), "fn changed() {}\n").unwrap();
    assert_eq!(dirty_submodules(&root, &options()).unwrap(), ["vendor/one"]);
    std::fs::write(root.join("vendor/t w o/new.rs"), "fn new() {}\n").unwrap();
    let mut found = dirty_submodules(&root, &options()).unwrap();
    found.sort();
    assert_eq!(found, ["vendor/one", "vendor/t w o"]);
    // committed inside, a submodule is at another commit, which the outer patch shows
    git(
        &root.join("vendor/one"),
        &["commit", "--quiet", "-am", "inner work"],
    );
    assert_eq!(
        dirty_submodules(&root, &options()).unwrap(),
        ["vendor/t w o"]
    );
    // and a change of the outer repository's own files is not a submodule's
    std::fs::write(root.join("a.rs"), "fn a() { b(); }\n").unwrap();
    assert_eq!(
        dirty_submodules(&root, &options()).unwrap(),
        ["vendor/t w o"]
    );
}

/// A repository in which commits are named in every way a session's commits can be, with
/// the id of each.
struct NamedCommits {
    root: PathBuf,
    base: String,
    work: String,
    reverted: String,
    /// A commit that only a remote-tracking ref names: it was never on `HEAD`.
    fetched: String,
    /// The stash, the commit of its index, and the commit of its untracked files.
    stash: [String; 3],
    /// A commit on a detached `HEAD` that was left again: only the reflog of `HEAD` names it.
    left: String,
    /// The commit the `HEAD` of another working tree names, and one only that tree's reflog
    /// names.
    there: String,
    at: String,
}

fn named_commits_repository(scratch: &Scratch) -> NamedCommits {
    use deep_diff_forge_patch::{Named, named_commits};
    let root = changed_repository(scratch);
    let id = |rev: &str| text(git(&root, &["rev-parse", rev]));
    let base = id("HEAD");
    // with one commit, that commit is all that is named
    for which in [Named::Everything, Named::Own] {
        assert_eq!(
            named_commits(&root, which, &options()).unwrap(),
            std::slice::from_ref(&base)
        );
    }
    // a commit and its revert; an annotated tag, which is followed to its commit
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "work"]);
    let work = id("HEAD");
    git(&root, &["revert", "--no-edit", "HEAD"]);
    let reverted = id("HEAD");
    git(&root, &["tag", "-a", "v1", "-m", "v1", &base]);
    let fetched = text(git(
        &root,
        &["commit-tree", "HEAD^{tree}", "-p", "HEAD", "-m", "fetched"],
    ));
    git(&root, &["update-ref", "refs/remotes/origin/main", &fetched]);
    std::fs::write(root.join("src.rs"), "fn stashed() {}\n").unwrap();
    std::fs::write(root.join("untracked.txt"), "u\n").unwrap();
    git(&root, &["stash", "--quiet", "--include-untracked"]);
    let stash = [id("refs/stash"), id("refs/stash^2"), id("refs/stash^3")];
    git(&root, &["checkout", "--quiet", "--detach"]);
    git(&root, &["commit", "--quiet", "--allow-empty", "-m", "left"]);
    let left = id("HEAD");
    git(&root, &["checkout", "--quiet", "-"]);
    let linked = scratch.join("linked");
    git(
        &root,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            linked.to_str().unwrap(),
        ],
    );
    git(
        &linked,
        &["commit", "--quiet", "--allow-empty", "-m", "there"],
    );
    let there = text(git(&linked, &["rev-parse", "HEAD"]));
    git(&linked, &["commit", "--quiet", "--allow-empty", "-m", "at"]);
    let at = text(git(&linked, &["rev-parse", "HEAD"]));
    git(&linked, &["reset", "--quiet", "--hard", &there]);
    NamedCommits {
        root,
        base,
        work,
        reverted,
        fetched,
        stash,
        left,
        there,
        at,
    }
}

/// Review V4 round 11 and the third independent review of the hooks: what a session's commits
/// are told from. `named_commits` gives the id of every commit that something names: every
/// ref (a tag followed to its commit), the `HEAD` of every working tree, and every entry of
/// the reflogs of those `HEAD`s and of the stash; or only what the session's own kinds of refs
/// name, which leaves out a commit that was only fetched. No name of a ref is read and nothing
/// is cut off.
#[test]
fn the_commits_that_something_names_are_listed_by_their_ids() {
    use deep_diff_forge_patch::{Named, named_commits};
    let scratch = Scratch::new("judge-patch-named");
    let made = named_commits_repository(&scratch);
    let named = |which: Named| named_commits(&made.root, which, &options()).unwrap();
    // the stash names one commit; the two it leads to are not named, only led to
    let mut everything = vec![
        made.base.clone(),
        made.work.clone(),
        made.reverted.clone(),
        made.fetched.clone(),
        made.stash[0].clone(),
        made.left.clone(),
        made.there.clone(),
        made.at.clone(),
    ];
    everything.sort();
    assert_eq!(named(Named::Everything), everything);
    everything.retain(|id| *id != made.fetched);
    assert_eq!(named(Named::Own), everything);

    // U1 row S124 (lane T3a of 0.11.1): each carrier on its own. A second stash leaves the first
    // named by the stash's reflog alone, and a tree whose `HEAD` reflog is gone names its commit
    // by its `HEAD` alone (each was also named another way above, so neither was held).
    std::fs::write(made.root.join("src.rs"), "fn stashed_again() {}\n").unwrap();
    git(&made.root, &["stash", "--quiet"]);
    let admin = text(git(
        &scratch.join("linked"),
        &["rev-parse", "--path-format=absolute", "--git-dir"],
    ));
    std::fs::remove_file(Path::new(&admin).join("logs/HEAD")).unwrap();
    for which in [Named::Everything, Named::Own] {
        let now = named(which);
        assert!(
            now.contains(&made.stash[0]),
            "{which:?}: the older stash, named by the stash's reflog alone, is not listed: {now:?}"
        );
        assert!(
            now.contains(&made.there),
            "{which:?}: a linked tree's HEAD, with no reflog of its own, is not listed: {now:?}"
        );
    }
}

/// `commits_beyond` gives the commits that lead on from the known ones, parents before
/// children, each with its first parent: what a session made, when the known ones are what
/// was named as it began.
#[test]
fn the_commits_beyond_the_known_ones_are_listed_parents_first() {
    use deep_diff_forge_patch::{Named, commits_beyond, named_commits};
    let scratch = Scratch::new("judge-patch-beyond");
    let made = named_commits_repository(&scratch);
    let (root, base, stash) = (&made.root, &made.base, &made.stash);
    let own = named_commits(root, Named::Own, &options()).unwrap();
    let everything = named_commits(root, Named::Everything, &options()).unwrap();
    let list = scratch.join("list");
    let beyond = |sources: &[String], known: &[String]| {
        commits_beyond(root, sources, known, &list, &options()).unwrap()
    };
    // What the session's own names lead to beyond the base: every commit but the fetched
    // one, and the two commits the stash leads to.
    let found = beyond(&own, std::slice::from_ref(base));
    let position = |id: &String| {
        found
            .iter()
            .position(|(commit, _)| commit == id)
            .unwrap_or_else(|| panic!("{id} is missing: {found:?}"))
    };
    let parent = |id: &String| found[position(id)].1.as_ref();
    assert_eq!(found.len(), 8, "{found:?}");
    assert!(
        found
            .iter()
            .all(|(commit, _)| *commit != made.fetched && commit != base),
        "{found:?}"
    );
    assert_eq!(parent(&made.work), Some(base));
    assert_eq!(parent(&made.reverted), Some(&made.work));
    assert_eq!(parent(&stash[0]), Some(&made.reverted));
    assert_eq!(parent(&stash[1]), Some(&made.reverted));
    assert_eq!(
        parent(&stash[2]),
        None,
        "the commit of a stash's untracked files has no parent"
    );
    assert_eq!(parent(&made.at), Some(&made.there));
    for (child, of) in [
        (&made.reverted, &made.work),
        (&stash[0], &made.reverted),
        (&stash[0], &stash[1]),
        (&stash[0], &stash[2]),
        (&made.left, &made.reverted),
        (&made.at, &made.there),
    ] {
        assert!(position(of) < position(child), "{of} after {child}");
    }
    // nothing lies beyond what is known; an id the repository does not have is passed over;
    // with nothing to start from there is nothing; and a name is not an id
    let none: Vec<(String, Option<String>)> = Vec::new();
    assert_eq!(beyond(&own, &everything), none);
    let gone = "0123456789abcdef0123456789abcdef01234567".to_owned();
    assert_eq!(
        beyond(std::slice::from_ref(&made.work), &[gone, base.clone()]),
        [(made.work.clone(), Some(base.clone()))]
    );
    assert_eq!(beyond(&[], std::slice::from_ref(base)), none);
    assert!(commits_beyond(root, &["HEAD".to_owned()], &[], &list, &options()).is_err());
}

/// A mutation run on the hooks left three bounds of this module alive: no test came near
/// them, so each could be a thousand times smaller. A repository under a path of more than a
/// kibibyte, with an index whose listing is over a mebibyte and more than two kibibytes of its
/// own ignore rules, is read whole.
#[test]
#[cfg_attr(
    target_os = "macos",
    ignore = "a path over 1 KiB cannot exist by name where PATH_MAX is 1,024 (macOS)"
)]
fn a_long_path_a_large_index_and_many_ignore_rules_are_read_whole() {
    let scratch = Scratch::new("judge-patch-bounds");
    let mut deep = scratch.path().to_path_buf();
    for letter in ["a", "b", "c", "d", "e"] {
        deep.push(letter.repeat(250));
    }
    let root = deep.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("src.rs"), "fn a() {}\n").unwrap();
    for n in 0..6_000 {
        std::fs::write(root.join(format!("{n:04}-{}", "n".repeat(200))), "").unwrap();
    }
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    let listing = git(&root, &["ls-files", "-t", "-z"]).len();
    assert!(listing > 1024 * 1024 + 64, "{listing}");
    // the rule that matters comes last
    let rules = "# a line of the repository's own ignore file\n".repeat(80);
    assert!(rules.len() > 2 * 1024 + 1024);
    std::fs::write(root.join(".git/info/exclude"), format!("{rules}late.txt\n")).unwrap();
    std::fs::write(root.join("late.txt"), "ignored\n").unwrap();
    std::fs::write(root.join("src.rs"), "fn a() { b(); }\n").unwrap();

    let patch = judge_patch(&root, "HEAD", &fresh(&scratch, "work"), &options()).unwrap();
    let shown = String::from_utf8_lossy(&patch.bytes);
    assert!(shown.contains("+fn a() { b(); }\n"), "{shown}");
    assert_eq!(shown.matches("diff --git ").count(), 1, "{shown}");
    assert!(!shown.contains("late.txt"), "{shown}");
}

/// Whether `HEAD` leads back to a commit: yes for the commit it names and for one before it,
/// no for one it has left, and an error for a name that is no commit. (A mutation run left
/// the answer alive when it was always "no": a stop then always said that the branch had
/// moved, and no test read those words where they must not be.)
#[test]
fn head_contains_says_whether_head_leads_back_to_a_commit() {
    use deep_diff_forge_patch::head_contains;
    let scratch = Scratch::new("judge-patch-head-contains");
    let root = changed_repository(&scratch);
    let contains = |commit: &str| head_contains(&root, commit, &options());
    let base = text(git(&root, &["rev-parse", "HEAD"]));
    assert!(contains(&base).unwrap());
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "work"]);
    let work = text(git(&root, &["rev-parse", "HEAD"]));
    assert!(contains(&base).unwrap() && contains(&work).unwrap());
    git(&root, &["reset", "--quiet", "--hard", &base]);
    assert!(contains(&base).unwrap());
    assert!(!contains(&work).unwrap());
    assert!(contains("0123456789abcdef0123456789abcdef01234567").is_err());
}

/// The repository's own ignore file, `info/exclude`: one that is not there gives no rule, and
/// one that is there and cannot be read is refused, because dropping its rules would put files
/// in the patch that a commit leaves out. (A mutation run left "which failure is it" alive both
/// ways: no test had a repository without the file, or with one it could not read.)
#[test]
fn a_missing_ignore_file_is_no_rule_and_an_unreadable_one_is_refused() {
    let scratch = Scratch::new("judge-patch-exclude");
    let root = changed_repository(&scratch);
    let exclude = root.join(".git/info/exclude");
    let plain = judge_patch(&root, "HEAD", &fresh(&scratch, "plain"), &options()).unwrap();
    let _ = std::fs::remove_file(&exclude);
    let without = judge_patch(&root, "HEAD", &fresh(&scratch, "without"), &options()).unwrap();
    assert_eq!(without.bytes, plain.bytes);

    std::fs::write(&exclude, "new name.txt\n").unwrap();
    std::fs::set_permissions(&exclude, std::fs::Permissions::from_mode(0o000)).unwrap();
    // not as root, who reads it all the same
    if std::fs::read(&exclude).is_err() {
        let refused = judge_patch(&root, "HEAD", &fresh(&scratch, "unreadable"), &options());
        assert!(
            refused.is_err() && format!("{refused:?}").contains("info/exclude"),
            "{refused:?}"
        );
    }
    std::fs::set_permissions(&exclude, std::fs::Permissions::from_mode(0o644)).unwrap();
    let ruled = judge_patch(&root, "HEAD", &fresh(&scratch, "ruled"), &options()).unwrap();
    assert!(!String::from_utf8_lossy(&ruled.bytes).contains("new name.txt"));

    // The bound, to the byte (four changes of it were alive after a mutation run): a file of
    // exactly a mebibyte is read whole, its last rule included; one byte more is refused, not
    // cut, because a rule that was cut off would put a file in the patch.
    let rule = "new name.txt\n";
    let padded = |len: usize| format!("{}{rule}", "#".repeat(len - rule.len() - 1) + "\n");
    std::fs::write(&exclude, padded(1024 * 1024)).unwrap();
    assert_eq!(std::fs::metadata(&exclude).unwrap().len(), 1024 * 1024);
    let exact = judge_patch(&root, "HEAD", &fresh(&scratch, "exact"), &options()).unwrap();
    assert!(!String::from_utf8_lossy(&exact.bytes).contains("new name.txt"));
    std::fs::write(&exclude, padded(1024 * 1024 + 1)).unwrap();
    let over = judge_patch(&root, "HEAD", &fresh(&scratch, "over"), &options());
    assert!(
        over.is_err() && format!("{over:?}").contains("info/exclude is over 1 MiB"),
        "{over:?}"
    );
}

/// Fourth independent review of the hooks, HR4-H-1. `staged_patch` is the change the index
/// holds against a commit: what a plain `git commit` would record, whatever the working tree
/// holds now. With everything staged and nothing changed since, it is the working tree's
/// patch, byte for byte; nothing in the repository is written; and an index with a conflict
/// in it has no tree, which is an error and not an empty patch.
#[test]
fn the_staged_change_is_what_the_index_holds_not_what_the_tree_holds() {
    use deep_diff_forge_patch::staged_patch;
    let scratch = Scratch::new("judge-patch-staged");
    let root = changed_repository(&scratch);
    let staged = |name: &str| staged_patch(&root, "HEAD", &fresh(&scratch, name), &options());
    // nothing staged: the index is the commit
    assert_eq!(staged("none").unwrap(), b"");
    // everything staged: the working tree's patch
    git(&root, &["add", "--all"]);
    let before = untouched(&root);
    let tree = judge_patch(&root, "HEAD", &fresh(&scratch, "tree"), &options()).unwrap();
    assert_eq!(staged("all").unwrap(), tree.bytes);
    assert_eq!(before, untouched(&root), "the repository was written");
    // the working tree is put back and a file removed, without `git add`: the index still
    // holds the change, the tree holds another
    std::fs::write(root.join("src.rs"), "fn a() {}\n").unwrap();
    std::fs::remove_file(root.join("new name.txt")).unwrap();
    let index = staged("index").unwrap();
    assert_eq!(index, tree.bytes);
    let now = judge_patch(&root, "HEAD", &fresh(&scratch, "now"), &options()).unwrap();
    assert_ne!(now.bytes, index);
    assert!(!String::from_utf8_lossy(&now.bytes).contains("b();"));
    assert!(matches!(
        staged_patch(&root, "no-such-commit", &fresh(&scratch, "bad"), &options()),
        Err(JudgePatchError::NoBase)
    ));
    // An index with unmerged paths (both sides of a conflict, as a merge that stopped leaves
    // them) has no tree. That is said as what it is, with what to do: Git's own words for it
    // are "error building trees" (sixth independent review of the hooks, HR6-L-5).
    git(&root, &["checkout", "--quiet", "--force", "HEAD"]);
    let base = text(git(&root, &["rev-parse", "HEAD"]));
    let side = |branch: &str, body: &str| {
        git(&root, &["checkout", "--quiet", "-B", branch, base.trim()]);
        std::fs::write(root.join("src.rs"), body).unwrap();
        git(&root, &["commit", "--quiet", "--all", "-m", branch]);
        text(git(&root, &["rev-parse", "HEAD"]))
    };
    let (theirs, ours) = (
        side("theirs", "fn theirs() {}\n"),
        side("ours", "fn ours() {}\n"),
    );
    git(
        &root,
        &["read-tree", "-m", base.trim(), ours.trim(), theirs.trim()],
    );
    assert!(
        text(git(&root, &["ls-files", "--unmerged"])).contains("src.rs"),
        "the index was to hold a conflict"
    );
    let unmerged = staged("unmerged").unwrap_err();
    assert!(matches!(unmerged, JudgePatchError::Unmerged), "{unmerged}");
    assert!(
        unmerged
            .to_string()
            .starts_with("the index has unmerged paths (")
            && unmerged
                .to_string()
                .ends_with("resolve them and `git add` the files"),
        "{unmerged}"
    );
}

/// Fourth independent review of the hooks, HR4-L-1 and HR4-L-5. `worktrees` lists the working
/// trees with the commit each `HEAD` names, and says of each whether Git can be asked there:
/// not of one that was removed by hand, with or without a plain directory put at its path.
/// Such a tree is passed over when the named commits are listed (it used to make the listing
/// fail, and every session one that could not be recorded). `has_uncommitted_changes` says
/// whether a working tree holds anything that is not committed.
#[test]
fn the_working_trees_are_listed_and_one_that_is_gone_is_passed_over() {
    use deep_diff_forge_patch::{Named, has_uncommitted_changes, named_commits, worktrees};
    let scratch = Scratch::new("judge-patch-worktrees");
    let root = changed_repository(&scratch);
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "work"]);
    let head = text(git(&root, &["rev-parse", "HEAD"]));
    let (clean, gone, plain) = (
        scratch.join("clean"),
        scratch.join("gone"),
        scratch.join("plain"),
    );
    for tree in [&clean, &gone, &plain] {
        let path = tree.to_str().unwrap();
        git(&root, &["worktree", "add", "--quiet", "--detach", path]);
    }
    std::fs::remove_dir_all(&gone).unwrap();
    std::fs::remove_dir_all(&plain).unwrap();
    std::fs::create_dir(&plain).unwrap();

    let found = worktrees(&root, &options()).unwrap();
    let usable: Vec<(String, bool)> = found
        .iter()
        .map(|tree| {
            let name = tree
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            (name, tree.usable)
        })
        .collect();
    assert_eq!(
        usable[0],
        ("repo".to_owned(), true),
        "the main working tree is first"
    );
    let mut linked = usable[1..].to_vec();
    linked.sort();
    assert_eq!(
        linked,
        [
            ("clean".to_owned(), true),
            ("gone".to_owned(), false),
            ("plain".to_owned(), false)
        ]
    );
    assert!(found.iter().all(|tree| tree.head.as_ref() == Some(&head)));
    // the commits are listed all the same
    let named = named_commits(&root, Named::Everything, &options()).unwrap();
    assert!(named.contains(&head), "{named:?}");

    assert!(!has_uncommitted_changes(&clean, &options()).unwrap());
    std::fs::write(clean.join("untracked.txt"), "u\n").unwrap();
    assert!(has_uncommitted_changes(&clean, &options()).unwrap());
    std::fs::remove_file(clean.join("untracked.txt")).unwrap();
    std::fs::write(clean.join("src.rs"), "fn staged() {}\n").unwrap();
    git(&clean, &["add", "--all"]);
    git(&clean, &["checkout", "--quiet", "HEAD", "--", "src.rs"]);
    assert!(!has_uncommitted_changes(&clean, &options()).unwrap());
    std::fs::write(clean.join("src.rs"), "fn staged() {}\n").unwrap();
    git(&clean, &["add", "--all"]);
    assert!(has_uncommitted_changes(&clean, &options()).unwrap());
    assert!(has_uncommitted_changes(&plain, &options()).is_err());
}
