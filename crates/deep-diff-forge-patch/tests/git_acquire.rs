// DDF integration: R02, R03, R07. Structured Git acquisition against a real temporary repository; Git is the application oracle for the authoritative patch bytes.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#patch-truth
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-patch--tests--git_acquire
//! Real-repository fixture: Unicode, tab and newline filenames, a binary file,
//! a rename with modification, mode changes (one under a path that display
//! headers split wrongly), a deletion, a no-final-newline edit, CRLF content and
//! untracked/ignored files. Unix-only: the fixture needs byte filenames and
//! executable bits.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use deep_diff_forge_patch::{
    GitAcquireError, GitAcquireOptions, GitAcquisition, GitChange, GitChangeKind, GitChangeOrigin,
    GitPath, GitSelection, StructuredDiff, acquire_git,
};
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

const ISOLATE: [(&str, &str); 2] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

fn git_out(root: &Path, args: &[&str]) -> Vec<u8> {
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

fn git_text(root: &Path, args: &[&str]) -> String {
    String::from_utf8(git_out(root, args))
        .unwrap()
        .trim()
        .to_owned()
}

fn options(include_untracked: bool) -> GitAcquireOptions {
    GitAcquireOptions {
        include_untracked,
        env: ISOLATE
            .iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
            .collect(),
        ..GitAcquireOptions::default()
    }
}

struct Fixture {
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

const TAB: &str = "tab\there.txt";
const NEWLINE: &str = "new\nline.txt";
const SPACEY: &str = "x b/y";

fn chmod(path: &Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// Base commit plus a dirty working tree covering every required case.
fn fixture(name: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("ddf-git-acq-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("x b")).unwrap();
    let fx = Fixture { root };
    let r = &fx.root;
    git_out(r, &["init", "-q"]);
    let w = |p: &str, b: &[u8]| std::fs::write(r.join(p), b).unwrap();
    w("café.txt", b"old\n");
    w(TAB, b"tab old\n");
    w(NEWLINE, b"newline old\n");
    w("bin.dat", &(0..=255u8).collect::<Vec<_>>());
    w(
        "rename-src.txt",
        (1..=20)
            .fold(String::new(), |mut text, n| {
                use std::fmt::Write as _;
                let _ = writeln!(text, "line {n}");
                text
            })
            .as_bytes(),
    );
    w("mode.sh", b"echo hi\n");
    w(SPACEY, b"spacey\n");
    w("gone.txt", b"bye\n");
    w("noeol.txt", b"a\nb\n");
    w("crlf.txt", b"one\r\ntwo\r\n");
    w(".gitignore", b"ignored.log\n");
    w("kind.txt", b"regular\n");
    chmod(&r.join("mode.sh"), 0o644);
    chmod(&r.join(SPACEY), 0o644);
    git_out(r, &["add", "-A"]);
    git_out(r, &["commit", "-q", "-m", "base"]);
    // Staged rename + modification.
    git_out(r, &["mv", "rename-src.txt", "renamed.txt"]);
    let mut renamed = std::fs::read(r.join("renamed.txt")).unwrap();
    renamed.extend_from_slice(b"line 21\n");
    w("renamed.txt", &renamed);
    git_out(r, &["add", "renamed.txt"]);
    // Unstaged edits.
    w("café.txt", b"new\n");
    w(TAB, b"tab new\n");
    w(NEWLINE, b"newline new\n");
    w("bin.dat", &(0..=255u8).rev().collect::<Vec<_>>());
    chmod(&r.join("mode.sh"), 0o755);
    chmod(&r.join(SPACEY), 0o755);
    std::fs::remove_file(r.join("gone.txt")).unwrap();
    w("noeol.txt", b"a\nc");
    w("crlf.txt", b"one\r\nTWO\r\n");
    // Type change: regular file -> symlink.
    std::fs::remove_file(r.join("kind.txt")).unwrap();
    std::os::unix::fs::symlink("crlf.txt", r.join("kind.txt")).unwrap();
    // Untracked (one Unicode, one nested) and ignored.
    std::fs::create_dir_all(r.join("nested")).unwrap();
    w("ünew.txt", b"fresh\n");
    w("nested/u.txt", b"deep\n");
    w("ignored.log", b"never\n");
    fx
}

fn find<'a>(acq: &'a GitAcquisition, path: &str) -> &'a GitChange {
    acq.diff()
        .changes()
        .iter()
        .find(|c| c.review_path().map(GitPath::as_bytes) == Some(path.as_bytes()))
        .unwrap_or_else(|| panic!("no change for {path:?}"))
}

fn blob_at_head(r: &Path, path: &str) -> String {
    git_text(r, &["rev-parse", &format!("HEAD:{path}")])
}

fn blob_of_file(r: &Path, path: &str) -> String {
    git_text(r, &["hash-object", "--no-filters", "--", path])
}

/// Negative control: revert raw-record pairing and the paths/statuses/blob IDs
/// asserted here would have to come from display headers, which split `x b/y`
/// wrongly and never see untracked files at all.
#[test]
fn working_tree_records_carry_exact_paths_and_statuses() {
    let fx = fixture("wt");
    let r = &fx.root;
    let acq = acquire_git(r, &GitSelection::WorkingTree, &options(true)).unwrap();
    assert_eq!(acq.base(), git_text(r, &["rev-parse", "HEAD"]));
    assert_eq!(acq.target(), None);

    let mut seen: Vec<(Vec<u8>, &str, &str)> = acq
        .diff()
        .changes()
        .iter()
        .map(|c| {
            (
                c.review_path().unwrap().as_bytes().to_vec(),
                c.kind.label(),
                c.origin.label(),
            )
        })
        .collect();
    seen.sort();
    let mut want: Vec<(Vec<u8>, &str, &str)> = vec![
        ("bin.dat", "modified", "tracked"),
        ("café.txt", "modified", "tracked"),
        ("crlf.txt", "modified", "tracked"),
        ("gone.txt", "deleted", "tracked"),
        ("kind.txt", "type_changed", "tracked"),
        ("mode.sh", "modified", "tracked"),
        ("nested/u.txt", "added", "untracked"),
        ("new\nline.txt", "modified", "tracked"),
        ("noeol.txt", "modified", "tracked"),
        ("renamed.txt", "renamed", "tracked"),
        ("tab\there.txt", "modified", "tracked"),
        ("x b/y", "modified", "tracked"),
        ("ünew.txt", "added", "untracked"),
    ]
    .into_iter()
    .map(|(p, k, o)| (p.as_bytes().to_vec(), k, o))
    .collect();
    want.sort();
    assert_eq!(seen, want, "ignored.log must be absent, every case present");
}

/// Negative control: without raw-record identities there are no blob IDs or
/// modes at all, and a null working-tree ID would not be resolved from the
/// paired `index` line.
#[test]
fn working_tree_records_carry_exact_blob_ids_and_modes() {
    let fx = fixture("blobs");
    let r = &fx.root;
    let acq = acquire_git(r, &GitSelection::WorkingTree, &options(true)).unwrap();
    // Exact path bytes (UTF-8 é is 0xC3 0xA9, not a C-quoted octal escape).
    assert_eq!(
        find(&acq, "café.txt").new_path.as_ref().unwrap().as_bytes(),
        b"caf\xc3\xa9.txt"
    );
    let rename = find(&acq, "renamed.txt");
    assert_eq!(
        rename.old_path.as_ref().unwrap().as_bytes(),
        b"rename-src.txt"
    );
    assert!(rename.score.is_some_and(|s| s > 50 && s < 100));
    assert_eq!(
        rename.old_blob.as_deref(),
        Some(blob_at_head(r, "rename-src.txt").as_str())
    );
    assert_eq!(
        rename.new_blob.as_deref(),
        Some(blob_of_file(r, "renamed.txt").as_str())
    );

    for path in ["café.txt", TAB, NEWLINE, "bin.dat", "noeol.txt", "crlf.txt"] {
        let c = find(&acq, path);
        assert_eq!(
            c.old_blob.as_deref(),
            Some(blob_at_head(r, path).as_str()),
            "{path:?}"
        );
        assert_eq!(
            c.new_blob.as_deref(),
            Some(blob_of_file(r, path).as_str()),
            "{path:?}"
        );
        assert_eq!((c.old_mode, c.new_mode), (Some(0o100_644), Some(0o100_644)));
    }
    assert!(find(&acq, "bin.dat").binary);
    assert!(!find(&acq, "crlf.txt").binary);

    for path in ["mode.sh", SPACEY] {
        let c = find(&acq, path);
        assert_eq!(
            (c.old_mode, c.new_mode),
            (Some(0o100_644), Some(0o100_755)),
            "{path}"
        );
        assert_eq!(c.old_blob.as_deref(), Some(blob_at_head(r, path).as_str()));
    }

    let kind = find(&acq, "kind.txt");
    assert_eq!(
        (kind.old_mode, kind.new_mode, kind.block_count),
        (Some(0o100_644), Some(0o120_000), 2)
    );
    assert_eq!(
        kind.old_blob.as_deref(),
        Some(blob_at_head(r, "kind.txt").as_str())
    );
    // A symlink's blob is its target text.
    let link_blob = r.join(".git/ddf-link-blob");
    std::fs::write(&link_blob, b"crlf.txt").unwrap();
    let link_blob = git_text(
        r,
        &[
            "hash-object",
            "--no-filters",
            "--",
            &link_blob.to_string_lossy(),
        ],
    );
    assert_eq!(kind.new_blob.as_deref(), Some(link_blob.as_str()));

    let gone = find(&acq, "gone.txt");
    assert_eq!(
        (
            gone.new_path.as_ref(),
            gone.new_mode,
            gone.new_blob.as_ref()
        ),
        (None, None, None)
    );
    assert_eq!(
        gone.old_blob.as_deref(),
        Some(blob_at_head(r, "gone.txt").as_str())
    );

    let fresh = find(&acq, "ünew.txt");
    assert_eq!(fresh.old_path, None);
    assert_eq!(fresh.new_mode, Some(0o100_644));
    assert_eq!(
        fresh.new_blob.as_deref(),
        Some(blob_of_file(r, "ünew.txt").as_str())
    );
}

/// Negative control: a re-rendered or lossily decoded patch would normalise
/// CRLF or the missing final newline.
#[test]
fn authoritative_patch_keeps_line_endings_verbatim() {
    let fx = fixture("eol");
    let acq = acquire_git(&fx.root, &GitSelection::WorkingTree, &options(false)).unwrap();
    // CRLF bytes survive verbatim in the authoritative patch.
    let crlf = acq.diff().change_patch(find(&acq, "crlf.txt"));
    assert!(crlf.windows(6).any(|w| w == b"+TWO\r\n"));
    let noeol = acq.diff().change_patch(find(&acq, "noeol.txt"));
    assert!(noeol.ends_with(b"+c\n\\ No newline at end of file\n"));
}

/// Negative control: reverting to display-header paths names the mode-only
/// change `y` (the `diff --git a/x b/y b/x b/y` header is ambiguous); the raw
/// record keeps `x b/y`.
#[test]
fn review_paths_come_from_raw_records_not_display_headers() {
    let fx = fixture("paths");
    let acq = acquire_git(&fx.root, &GitSelection::WorkingTree, &options(true)).unwrap();
    let captured = acq.diff().capture().unwrap();
    assert_eq!(captured.original().as_bytes(), acq.diff().patch_bytes());
    let files = acq.diff().review_files(&captured).unwrap();
    let mut names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    names.sort_unstable();
    assert!(names.contains(&SPACEY) && names.contains(&TAB) && names.contains(&NEWLINE));
    let display: Vec<&str> = captured.files().iter().map(|f| f.path.as_str()).collect();
    assert!(
        !display.contains(&SPACEY),
        "fixture no longer exercises the display-header defect: {display:?}"
    );
}

/// Negative control: if the patch bytes were a re-rendered projection, or if
/// untracked additions were missing, the clean clone would not end up equal to
/// the working tree.
#[test]
fn authoritative_patch_applies_in_a_clean_clone() {
    let fx = fixture("apply");
    let r = &fx.root;
    let acq = acquire_git(r, &GitSelection::WorkingTree, &options(true)).unwrap();
    let clone = r.with_file_name(format!(
        "{}-clone",
        r.file_name().unwrap().to_string_lossy()
    ));
    let _ = std::fs::remove_dir_all(&clone);
    git_out(
        r.parent().unwrap(),
        &[
            "clone",
            "-q",
            &r.to_string_lossy(),
            &clone.to_string_lossy(),
        ],
    );
    let guard = Fixture {
        root: clone.clone(),
    };
    let patch = clone.join("../ddf-acq.patch");
    std::fs::write(&patch, acq.diff().patch_bytes()).unwrap();
    let patch_arg = patch.to_string_lossy().into_owned();
    git_out(&clone, &["apply", "--check", &patch_arg]);
    git_out(&clone, &["apply", &patch_arg]);
    std::fs::remove_file(&patch).unwrap();
    for path in [
        "café.txt",
        TAB,
        NEWLINE,
        "bin.dat",
        "renamed.txt",
        "noeol.txt",
        "crlf.txt",
        "ünew.txt",
        "nested/u.txt",
    ] {
        assert_eq!(
            std::fs::read(clone.join(path)).unwrap(),
            std::fs::read(r.join(path)).unwrap(),
            "{path:?}"
        );
    }
    for path in ["mode.sh", SPACEY] {
        let mode = std::fs::metadata(clone.join(path))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "{path} must be executable after apply");
    }
    assert_eq!(
        std::fs::read_link(clone.join("kind.txt")).unwrap(),
        Path::new("crlf.txt")
    );
    assert!(!clone.join("gone.txt").exists());
    assert!(!clone.join("rename-src.txt").exists());
    drop(guard);
}

/// Negative control: an implementation that ran `git add -N` (or refreshed the
/// index) to see untracked files would change the index bytes or the status.
#[test]
fn untracked_capture_does_not_mutate_the_repository() {
    let fx = fixture("readonly");
    let r = &fx.root;
    // `status` may refresh stat data in the index, so take the baseline after it.
    let status_before = git_out(
        r,
        &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
    );
    let index_before = std::fs::read(r.join(".git/index")).unwrap();
    acquire_git(r, &GitSelection::WorkingTree, &options(true)).unwrap();
    assert_eq!(std::fs::read(r.join(".git/index")).unwrap(), index_before);
    let status_after = git_out(
        r,
        &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
    );
    assert_eq!(status_after, status_before);
    // Untracked files are still reported as untracked (`?`), not intent-to-add.
    assert!(
        status_after
            .windows(11)
            .any(|w| w == "? ünew.txt".as_bytes())
    );
}

/// Negative control: without untracked capture (or with it enabled for the
/// wrong selection) these files would be silently absent or wrongly present.
#[test]
fn untracked_files_are_opt_in_and_staged_selection_sees_only_the_index() {
    let fx = fixture("staged");
    let r = &fx.root;
    let tracked_only = acquire_git(r, &GitSelection::WorkingTree, &options(false)).unwrap();
    assert!(
        tracked_only
            .diff()
            .changes()
            .iter()
            .all(|c| c.origin == GitChangeOrigin::Tracked)
    );
    assert_eq!(tracked_only.diff().changes().len(), 11);

    let staged = acquire_git(r, &GitSelection::Staged, &options(false)).unwrap();
    let kinds: Vec<GitChangeKind> = staged.diff().changes().iter().map(|c| c.kind).collect();
    assert_eq!(kinds, vec![GitChangeKind::Renamed]);

    let err = acquire_git(r, &GitSelection::Staged, &options(true)).unwrap_err();
    assert!(
        matches!(err, GitAcquireError::InvalidSelection { .. }),
        "{err}"
    );
}

/// Negative control: a range that did not resolve to commit IDs (or accepted
/// option-like revisions) would fail these identity and refusal checks.
#[test]
fn range_selection_is_bound_to_resolved_commits() {
    let fx = fixture("range");
    let r = &fx.root;
    let base = git_text(r, &["rev-parse", "HEAD"]);
    git_out(r, &["add", "-A"]);
    git_out(r, &["commit", "-q", "-m", "candidate"]);
    let target = git_text(r, &["rev-parse", "HEAD"]);
    let sel = GitSelection::parse_range("HEAD~1..HEAD").unwrap();
    let acq = acquire_git(r, &sel, &options(false)).unwrap();
    assert_eq!(
        (acq.base(), acq.target()),
        (base.as_str(), Some(target.as_str()))
    );
    let fresh = find(&acq, "ünew.txt");
    assert_eq!(fresh.kind, GitChangeKind::Added);
    assert_eq!(
        fresh.new_blob.as_deref(),
        Some(blob_of_file(r, "ünew.txt").as_str())
    );
    assert_eq!(find(&acq, "x b/y").new_mode, Some(0o100_755));
    for bad in [
        "HEAD",
        "..HEAD",
        "HEAD..",
        "A...B",
        "--output=x..HEAD",
        "a..b..c",
    ] {
        assert!(GitSelection::parse_range(bad).is_err(), "{bad}");
    }
    let missing = GitSelection::parse_range("HEAD..no-such-rev").unwrap();
    assert!(matches!(
        acquire_git(r, &missing, &options(false)),
        Err(GitAcquireError::InvalidSelection { .. })
    ));
}

/// Negative control: real Git output with its patch blocks re-ordered, or with
/// one block dropped, must be refused; a parser that zipped records with blocks
/// by position (or that ignored counts) would accept and mislabel it.
#[test]
fn mismatched_raw_patch_pairing_from_real_git_output_is_refused() {
    let fx = fixture("mismatch");
    let r = &fx.root;
    let out = git_out(
        r,
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--raw",
            "-z",
            "--patch",
            "--binary",
            "--full-index",
            "--no-abbrev",
            "HEAD",
            "--",
            "café.txt",
            "crlf.txt",
        ],
    );
    assert_eq!(StructuredDiff::parse(&out).unwrap().changes().len(), 2);
    let split = out.windows(2).position(|w| w == b"\0\0").unwrap() + 2;
    let (raw, patch) = out.split_at(split);
    let second = patch
        .windows(12)
        .position(|w| w == b"\ndiff --git ")
        .unwrap()
        + 1;
    let swapped = [raw, &patch[second..], &patch[..second]].concat();
    assert!(matches!(
        StructuredDiff::parse(&swapped),
        Err(GitAcquireError::IdentityMismatch { record: 0, .. })
    ));
    let dropped = [raw, &patch[..second]].concat();
    assert!(matches!(
        StructuredDiff::parse(&dropped),
        Err(GitAcquireError::CountMismatch {
            records: 2,
            expected_blocks: 2,
            blocks: 1
        })
    ));
}

/// Negative control: an unbounded reader would return the output instead of
/// the typed budget refusal.
#[test]
fn output_over_budget_is_refused() {
    let fx = fixture("budget");
    let opts = GitAcquireOptions {
        byte_budget: 64,
        ..options(false)
    };
    assert!(matches!(
        acquire_git(&fx.root, &GitSelection::WorkingTree, &opts),
        Err(GitAcquireError::BudgetExceeded { limit_bytes: 64 })
    ));
}

/// A Git that never finishes is stopped at the acquisition's time limit, and the caller gets
/// `TimedOut` instead of hanging (the git-timeout residual risk of 2026-10-28). A Git that
/// finishes in time is unaffected by the limit.
#[test]
fn a_stalled_git_is_stopped_at_the_time_limit() {
    let fx = fixture("stall");
    let stub = fx.root.join("stalled-git");
    std::fs::write(&stub, "#!/bin/sh\nexec sleep 30\n").unwrap();
    chmod(&stub, 0o755);
    let opts = GitAcquireOptions {
        git_program: stub.into_os_string(),
        time_limit: std::time::Duration::from_millis(300),
        ..options(false)
    };
    let started = std::time::Instant::now();
    let result = acquire_git(&fx.root, &GitSelection::WorkingTree, &opts);
    let took = started.elapsed();
    assert!(
        matches!(
            result,
            Err(GitAcquireError::TimedOut {
                command: "git rev-parse",
                limit_ms: 300
            })
        ),
        "{result:?}"
    );
    assert!(took < std::time::Duration::from_secs(5), "{took:?}");
    let in_time = GitAcquireOptions {
        time_limit: std::time::Duration::from_secs(60),
        ..options(false)
    };
    assert!(acquire_git(&fx.root, &GitSelection::WorkingTree, &in_time).is_ok());
}

/// The time limit bounds the caller even when Git's own child outlives the kill and keeps the
/// pipes open: the stub forks a long sleep that inherits stdout (bug hunt 2026-10-28, H1-03).
#[test]
fn a_surviving_child_cannot_stretch_the_time_limit() {
    let fx = fixture("orphan");
    let stub = fx.root.join("forking-git");
    std::fs::write(&stub, "#!/bin/sh\nsleep 20 &\nexec sleep 20\n").unwrap();
    chmod(&stub, 0o755);
    let opts = GitAcquireOptions {
        git_program: stub.into_os_string(),
        time_limit: std::time::Duration::from_millis(300),
        ..options(false)
    };
    let started = std::time::Instant::now();
    let result = acquire_git(&fx.root, &GitSelection::WorkingTree, &opts);
    assert!(
        matches!(result, Err(GitAcquireError::TimedOut { .. })),
        "{result:?}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

/// Many local filters (a long `git config` listing) and a deep repository path (long
/// `rev-parse` answers) are read in full: the listing and path budgets hold real repositories.
#[test]
fn long_filter_listings_and_deep_paths_are_read() {
    let fx = fixture("many-filters");
    let r = &fx.root;
    for i in 0..80 {
        git_out(
            r,
            &[
                "config",
                &format!("filter.local-filter-with-a-long-name-{i:03}.clean"),
                "cat",
            ],
        );
    }
    acquire_git(r, &GitSelection::WorkingTree, &options(false)).unwrap();
    // Four 201-byte components stay under macOS's 1024-byte PATH_MAX yet make the three
    // `rev-parse` answer lines well over a kibibyte together.
    let deep = (0..4).fold(r.clone(), |p, i| p.join(format!("{}{i}", "d".repeat(200))));
    assert!(deep.as_os_str().len() > 800, "{}", deep.display());
    std::fs::create_dir_all(&deep).unwrap();
    git_out(&deep, &["init", "-q"]);
    let dirs = deep_diff_forge_patch::repository_dirs(&deep, &options(false)).unwrap();
    assert_eq!(dirs.top, std::fs::canonicalize(&deep).unwrap());
    assert!(dirs.git_dir.ends_with(".git") && !dirs.has_alternates);
}

/// A filter that the repository's own configuration defines never runs: reviewing a repository
/// someone handed you must not run their code (bug hunt 2026-10-28, H2-01).
#[test]
fn a_repository_local_filter_never_runs() {
    let fx = fixture("filter");
    let r = &fx.root;
    let marker = r.join("filter-ran");
    std::fs::write(r.join(".gitattributes"), "*.txt filter=evil\n").unwrap();
    git_out(
        r,
        &[
            "config",
            "filter.evil.clean",
            &format!("sh -c 'touch {}; cat'", marker.display()),
        ],
    );
    git_out(
        r,
        &[
            "config",
            "filter.evil.process",
            &format!("sh -c 'touch {}'", marker.display()),
        ],
    );
    std::fs::write(r.join("tracked.txt"), b"changed in the working tree\n").unwrap();
    for include_untracked in [false, true] {
        acquire_git(r, &GitSelection::WorkingTree, &options(include_untracked)).unwrap();
    }
    acquire_git(r, &GitSelection::Staged, &options(false)).unwrap();
    assert!(!marker.exists(), "a repository-local filter ran");
}

/// Negative control: resolving `HEAD` unconditionally would fail on a fresh
/// repository; the port compares against the empty tree instead.
#[test]
fn unborn_head_compares_against_the_empty_tree() {
    let root = std::env::temp_dir().join(format!("ddf-git-acq-unborn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fx = Fixture { root };
    git_out(&fx.root, &["init", "-q"]);
    std::fs::write(fx.root.join("first.txt"), b"1\n").unwrap();
    git_out(&fx.root, &["add", "first.txt"]);
    let acq = acquire_git(&fx.root, &GitSelection::WorkingTree, &options(false)).unwrap();
    assert_eq!(
        acq.base(),
        git_text(&fx.root, &["hash-object", "-t", "tree", "/dev/null"])
    );
    assert_eq!(acq.diff().changes()[0].kind, GitChangeKind::Added);
}

/// Negative control: without the pinned argv (`--src-prefix`/`--dst-prefix`,
/// `--no-color`, `-c core.quotePath=false`, `--no-ext-diff`) repository config
/// would change the headers, and acquisition would be refused or differ.
#[test]
fn repository_config_cannot_reshape_the_acquisition() {
    let fx = fixture("config");
    let r = &fx.root;
    let before = acquire_git(r, &GitSelection::WorkingTree, &options(true)).unwrap();
    for (key, value) in [
        ("diff.noprefix", "true"),
        ("diff.mnemonicPrefix", "true"),
        ("core.quotePath", "true"),
        ("color.diff", "always"),
        ("diff.external", "false"),
        ("diff.relative", "true"),
    ] {
        git_out(r, &["config", key, value]);
    }
    let after = acquire_git(r, &GitSelection::WorkingTree, &options(true)).unwrap();
    assert_eq!(after.diff(), before.diff());
}

/// Anchor C-12: committed non-UTF-8 content is acquired, lossily decoded and marked on its own file,
/// with the exact bytes sealed. Before, the whole acquisition failed ("not UTF-8", exit 3).
#[test]
fn non_utf8_content_is_acquired_marked_and_sealed_exactly() {
    let root = std::env::temp_dir().join(format!("ddf-git-acq-latin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    git_out(&root, &["init", "-q"]);
    std::fs::write(root.join("latin.txt"), b"caf\xe9\n").unwrap();
    std::fs::write(root.join("ok.txt"), b"a\n").unwrap();
    git_out(&root, &["add", "-A"]);
    git_out(&root, &["commit", "-q", "-m", "base"]);
    std::fs::write(root.join("latin.txt"), b"caf\xe9s\n").unwrap();
    std::fs::write(root.join("ok.txt"), b"b\n").unwrap();
    let acq = acquire_git(&root, &GitSelection::WorkingTree, &options(false)).unwrap();
    let captured = acq.diff().capture().unwrap();
    assert!(!captured.is_exact());
    assert_eq!(captured.original_bytes(), acq.diff().patch_bytes());
    let files = acq.diff().review_files(&captured).unwrap();
    let lossy: Vec<(&str, u32)> = files
        .iter()
        .map(|f| (f.path.as_str(), f.patch_twin.lossy_lines))
        .collect();
    assert_eq!(lossy, [("latin.txt", 2), ("ok.txt", 0)]);
    let _ = std::fs::remove_dir_all(&root);
}

/// Anchor A-02: acquisition in a partial clone must not fetch. Before, `git diff` lazily ran
/// `git fetch origin` against the promisor remote (a network effect from a read-only command);
/// now transports are off, so a missing blob is an error and no object arrives.
#[test]
fn a_partial_clone_is_never_fetched_from() {
    let root = std::env::temp_dir().join(format!("ddf-git-acq-promisor-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let origin = root.join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    git_out(&origin, &["init", "-q"]);
    for n in 1..=2 {
        std::fs::write(origin.join("f.txt"), format!("version {n}\n")).unwrap();
        git_out(&origin, &["add", "-A"]);
        git_out(&origin, &["commit", "-q", "-m", "c"]);
    }
    git_out(&origin, &["config", "uploadpack.allowFilter", "true"]);
    let url = format!("file://{}", origin.display());
    git_out(
        &root,
        &[
            "clone",
            "-q",
            "--filter=blob:none",
            "--no-checkout",
            &url,
            "clone",
        ],
    );
    let clone = root.join("clone");
    let objects = || git_text(&clone, &["count-objects", "-v"]);
    let before = objects();
    let range = GitSelection::parse_range("HEAD~1..HEAD").unwrap();
    assert!(acquire_git(&clone, &range, &options(false)).is_err());
    assert_eq!(
        objects(),
        before,
        "acquisition fetched from the promisor remote"
    );
    let _ = std::fs::remove_dir_all(&root);
}
