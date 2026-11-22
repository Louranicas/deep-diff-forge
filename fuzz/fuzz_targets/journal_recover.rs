#![no_main]
// DDF 0.10.0, task S3.32.b: the review crate's decision journal as `import_tuicr` reads and repairs
// it, on a run prepared once per process in the scratch directory `scripts/fuzz_target.sh` makes
// under the corpus root (DDF_FUZZ_SCRATCH). Design/status: docs/TESTING_GOLD_STANDARD.md.
//
// Each input damages the journal and checkpoint the run's first import wrote: byte 0 says what
// becomes of the checkpoint (kept, emptied as 0.8.3 to 0.9.0 leave it, or removed) and whether a
// temporary file is left beside it, bytes 1 and 2 where the journal is cut (big-endian, at most
// its length), and the rest are arbitrary bytes written after the cut. The property: no panic; a
// dry run and the import after it plan as many events and announce the same repairs, or both
// refuse; after an import that succeeds, nothing is left to repair or append.
use deep_diff_forge_review::{ImportOptions, JOURNAL_FILE, Profile, cli, import_tuicr, prepare};
use libfuzzer_sys::fuzz_target;
use std::{fs, path::PathBuf, process::Command, sync::OnceLock};

const SOURCE: &str = "fn a() {}\n";
const CHECKPOINTS: &str = "decisions.checkpoints";
const ANALYSIS: &str = r#"{"schema": "deep-diff-forge.analysis.v1", "review_id": "@ID@",
  "candidate_snapshot": "@SNAPSHOT@", "coverage": [{"path": "x.rs", "coverage": "analyzed"}],
  "review": {"files": [{"path": "x.rs", "status": "modified"}]}}"#;
const SESSION: &str = r#"{"id": "00000000-0000-4000-8000-000000000001", "version": "1.3",
  "repo_path": @REPO@, "branch_name": null, "base_commit": "@HEAD@", "diff_source": "working_tree",
  "commit_range": null, "pr_session_key": null, "created_at": "2026-10-25T00:00:00Z",
  "updated_at": "2026-10-25T00:00:00Z", "session_notes": null, "review_comments":
  [{"id": "a", "content": "a", "author": "F"}, {"id": "b", "content": "b", "author": "F"}],
  "files": {"x.rs": {"path": "x.rs", "reviewed": false, "status": "modified", "file_comments": [],
  "line_comments": {}, "reviewed_hunks": [], "content_hash": 0}}}"#;
const GIT: &str = "git init -q && git add x.rs && git -c user.name=F -c user.email=f@example.invalid \
  -c commit.gpgsign=false commit -qm f && git rev-parse HEAD";

/// The run directory, the session, the journal and the checkpoint (name, bytes) it first wrote.
struct Run(PathBuf, PathBuf, Vec<u8>, (String, Vec<u8>));
static RUN: OnceLock<Run> = OnceLock::new();

#[expect(
    clippy::disallowed_methods,
    reason = "the fuzz target's door: its scratch directory and Git, once per process"
)]
fn prepared() -> Run {
    let Some(scratch) = std::env::var_os("DDF_FUZZ_SCRATCH") else {
        eprintln!("journal_recover: run it through scripts/fuzz_target.sh (DDF_FUZZ_SCRATCH)");
        std::process::exit(2);
    };
    let root = PathBuf::from(scratch).join(format!("run-{}", std::process::id()));
    let (repo, dir, session) = (
        root.join("repo"),
        root.join("run"),
        root.join("session.json"),
    );
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("x.rs"), SOURCE).unwrap();
    let head = Command::new("sh")
        .current_dir(&repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["-c", GIT])
        .output()
        .unwrap();
    let snapshot =
        deep_diff_forge_core::changed_source_snapshot_id([("x.rs", Some(SOURCE.as_bytes()))]);
    let analysis = ANALYSIS
        .replace("@ID@", &"0".repeat(64))
        .replace("@SNAPSHOT@", &snapshot);
    fs::write(root.join("analysis.json"), analysis).unwrap();
    fs::write(root.join("decision.md"), "Keep dissent.\n").unwrap();
    let mut request = cli::example();
    request.profile = Profile::Roster;
    request.reviewers.truncate(2);
    request.repository = repo.display().to_string();
    request.analysis_file = root.join("analysis.json").display().to_string();
    request.decision_file = root.join("decision.md").display().to_string();
    prepare(request, &dir).unwrap();
    let head = String::from_utf8(head.stdout).unwrap();
    let text = SESSION.replace("@REPO@", &serde_json::to_string(&repo).unwrap());
    fs::write(&session, text.replace("@HEAD@", head.trim())).unwrap();
    import_tuicr(&dir, &session, ImportOptions { execute: true }).unwrap();
    let [checkpoint]: [_; 1] = fs::read_dir(dir.join(CHECKPOINTS))
        .unwrap()
        .map(|e| e.unwrap())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let checkpoint = (
        checkpoint.file_name().into_string().unwrap(),
        fs::read(checkpoint.path()).unwrap(),
    );
    let journal = fs::read(dir.join(JOURNAL_FILE)).unwrap();
    Run(dir, session, journal, checkpoint)
}

fuzz_target!(|data: &[u8]| {
    let Run(dir, session, journal, (name, bytes)) = RUN.get_or_init(prepared);
    let Some((&how, rest)) = data.split_first() else {
        return;
    };
    let (cut, tail) = rest.split_at(rest.len().min(2));
    let cut = cut
        .iter()
        .fold(0, |n, b| n << 8 | usize::from(*b))
        .min(journal.len());
    // Back to the first import's state, without the torn records the last input left.
    for entry in fs::read_dir(dir).unwrap().map(|e| e.unwrap()) {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("decisions.torn-")
        {
            fs::remove_file(entry.path()).unwrap();
        }
    }
    let checkpoints = dir.join(CHECKPOINTS);
    let _ = fs::remove_dir_all(&checkpoints);
    fs::create_dir(&checkpoints).unwrap();
    match how & 3 {
        0 | 3 => fs::write(checkpoints.join(name), bytes).unwrap(),
        1 => fs::write(checkpoints.join(name), "").unwrap(),
        _ => {}
    }
    if how & 4 != 0 {
        fs::write(
            checkpoints.join(format!(".ddf-tmp.{name}.2147483647-0")),
            tail,
        )
        .unwrap();
    }
    fs::write(dir.join(JOURNAL_FILE), [&journal[..cut], tail].concat()).unwrap();
    let import = |execute| import_tuicr(dir, session, ImportOptions { execute });
    match (import(false), import(true)) {
        (Ok(dry), Ok(done)) => {
            assert_eq!(
                dry.repairs, done.repairs,
                "the dry run announced other repairs"
            );
            assert_eq!(
                dry.events.len(),
                done.events.len(),
                "it planned other events"
            );
            let again = import(false).expect("a journal an import repaired reads back");
            assert!(
                again.repairs.is_empty() && again.events.is_empty(),
                "{:?}",
                again.repairs
            );
        }
        (Err(_), Err(_)) => {}
        (dry, done) => panic!(
            "dry run {:?}, import {:?}",
            dry.map(|d| d.repairs),
            done.err()
        ),
    }
});
