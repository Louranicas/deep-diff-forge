#![no_main]
// DDF 0.11.1: caller-supplied analysis JSON as `council prepare` captures, parses, verifies against
// the request and candidate, and seals. Design/status: docs/REVIEW_COUNCIL.md#report-fuzz.
//
// A fixed private Git repository and valid request are made once per process. Each input replaces
// only the analysis bytes and calls the production `prepare` path. The property: no panic; a
// successful prepare writes a loadable sealed manifest, and a refusal leaves no partial run.
use deep_diff_forge_review::{Profile, Request, cli, load_manifest, prepare};
use libfuzzer_sys::fuzz_target;
use std::{fs, path::PathBuf, process::Command, sync::OnceLock};

struct Fixture {
    analysis: PathBuf,
    request: Request,
    run: PathBuf,
}

static FIXTURE: OnceLock<Fixture> = OnceLock::new();

#[expect(
    clippy::disallowed_methods,
    reason = "the fuzz target's door: its private scratch directory and one fixed Git repository"
)]
fn fixture() -> Fixture {
    let Some(scratch) = std::env::var_os("DDF_FUZZ_SCRATCH") else {
        eprintln!("council_analysis: run it through scripts/fuzz_target.sh (DDF_FUZZ_SCRATCH)");
        std::process::exit(2);
    };
    let root = PathBuf::from(scratch).join(format!("analysis-{}", std::process::id()));
    let repository = root.join("repo");
    let analysis = root.join("analysis.json");
    let decision = root.join("decision.md");
    let run = root.join("run");
    fs::create_dir_all(&repository).expect("create analysis fuzz repository");
    fs::write(repository.join("x.rs"), "fn b() {}\n").expect("write analysis fuzz source");
    for args in [
        &["init", "-q"][..],
        &["add", "x.rs"],
        &[
            "-c",
            "user.name=Fuzz",
            "-c",
            "user.email=fuzz@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "fixture",
        ],
    ] {
        let output = Command::new("git")
            .args(args)
            .current_dir(&repository)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("run Git for analysis fuzz fixture");
        assert!(
            output.status.success(),
            "Git fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::write(&decision, "Keep dissent and require falsification.\n")
        .expect("write analysis fuzz decision");
    let mut request = cli::example();
    request.profile = Profile::Roster;
    request.reviewers.truncate(2);
    request.repository = repository.display().to_string();
    request.analysis_file = analysis.display().to_string();
    request.decision_file = decision.display().to_string();
    Fixture {
        analysis,
        request,
        run,
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() > 512 * 1024 {
        return;
    }
    let fixture = FIXTURE.get_or_init(fixture);
    let _ = fs::remove_dir_all(&fixture.run);
    fs::write(&fixture.analysis, data).expect("write fuzzed analysis");
    match prepare(fixture.request.clone(), &fixture.run) {
        Ok(_) => {
            load_manifest(&fixture.run).expect("prepare writes a loadable sealed manifest");
            fs::remove_dir_all(&fixture.run).expect("remove prepared analysis fuzz run");
        }
        Err(_) => assert!(!fixture.run.exists(), "a refusal left a partial run"),
    }
});
