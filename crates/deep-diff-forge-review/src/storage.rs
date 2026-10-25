// Review council: Bounded private context capture, canonical candidate snapshot verification, source identity and immutable qualified artifacts.
// Design/status: docs/REVIEW_COUNCIL.md#capture-and-storage
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-review--src--storage
//! Private, append-only run artifacts. Hashes establish local consistency, not signer authentication.
use crate::model::Qualification;
use crate::{
    Artifact, Execution, Manifest, Outcome, Profile, Report, Request, Result, ReviewFailure, Round,
    report_schema, require, validate_report, validate_request,
};
use deep_diff_forge_core::sha256_hex;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
pub(crate) const REPORT_CAP: u64 = 128 * 1024;
const CONTEXT_CAP: usize = 512 * 1024;
const POLICY: &[(&str, &str)] = &[
    (
        "policy:pstack-rubric",
        include_str!("../policy/pstack/review-rubric.md"),
    ),
    (
        "policy:pstack-structure",
        include_str!("../policy/pstack/encode-lessons.md"),
    ),
    (
        "policy:pstack-verification",
        include_str!("../policy/pstack/prove-it-works.md"),
    ),
    (
        "policy:antipattern-index",
        include_str!("../policy/antipatterns.json"),
    ),
];

pub(crate) fn read_bytes(path: &Path, cap: u64) -> Result<Vec<u8>> {
    require(
        !fs::symlink_metadata(path)?.file_type().is_symlink(),
        "symlink artifact rejected",
    )?;
    let mut data = Vec::new();
    File::open(path)?.take(cap + 1).read_to_end(&mut data)?;
    require(
        data.len() as u64 <= cap,
        "artifact exceeds acquisition budget",
    )?;
    Ok(data)
}
pub(crate) fn read_json<T: serde::de::DeserializeOwned>(path: &Path, cap: u64) -> Result<T> {
    Ok(serde_json::from_slice(&read_bytes(path, cap)?)?)
}
pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    File::open(path.parent().unwrap_or(Path::new(".")))?.sync_all()?;
    Ok(())
}
pub(crate) fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    write_new(path, &serde_json::to_vec_pretty(value)?)
}
pub(crate) fn make_dir(path: &Path) -> Result<()> {
    fs::DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}
fn capture(id: String, path: &Path, remaining: &mut usize) -> Result<Artifact> {
    let data = read_bytes(path, *remaining as u64)?;
    *remaining -= data.len();
    let text =
        String::from_utf8(data).map_err(|_| ReviewFailure("context must be UTF-8".into()))?;
    Ok(Artifact {
        id,
        label: path.display().to_string(),
        sha256: sha256_hex(text.as_bytes()),
        text,
    })
}
pub(crate) fn git_bytes(repository: &Path, args: &[&str], cap: u64) -> Result<Vec<u8>> {
    let mut child = Command::new("git")
        .args([
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .current_dir(repository)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut bytes = Vec::new();
    let output = child
        .stdout
        .take()
        .ok_or_else(|| ReviewFailure("missing Git output stream".into()))?;
    output.take(cap + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > cap {
        let _ = child.kill();
        let _ = child.wait();
        return Err(ReviewFailure(
            "Git output acquisition budget exceeded".into(),
        ));
    }
    require(child.wait()?.success(), "Git acquisition command failed")?;
    Ok(bytes)
}
/// The fingerprinted source file set: Git HEAD plus tracked and non-ignored untracked paths.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Inventory {
    head: String,
    paths: std::collections::BTreeSet<String>,
}
impl Inventory {
    pub(crate) fn contains(&self, path: &str) -> bool {
        self.paths.contains(path)
    }
}
pub(crate) const SNAPSHOT_DIR: &str = "source";
pub(crate) const INVENTORY_FILE: &str = "source-inventory.json";
/// Relative to a mutation gate's job directory; written inside the sandbox, read after it exits.
pub(crate) const MUTATION_WITNESS: &str = "work/mutation-results/mutants.out/outcomes.json";

/// Fingerprint tracked and non-ignored untracked workspace files, including mode and HEAD.
/// Scope excludes ignored build output and external dependencies; it is not a hermetic build identity.
/// # Errors
/// Rejects non-Git workspaces, symlinks, non-UTF-8 names and sources above 64 MiB.
pub fn source_fingerprint(repository: &Path) -> Result<String> {
    fingerprint_tree(repository, &inventory(repository)?)
}
pub(crate) fn inventory(repository: &Path) -> Result<Inventory> {
    let head = String::from_utf8(git_bytes(repository, &["rev-parse", "HEAD"], 1024)?)
        .map_err(|_| ReviewFailure("non-UTF-8 Git HEAD".into()))?;
    let top = String::from_utf8(git_bytes(
        repository,
        &["rev-parse", "--show-toplevel"],
        8192,
    )?)
    .map_err(|_| ReviewFailure("non-UTF-8 repository root".into()))?;
    require(
        fs::canonicalize(top.trim_end())? == fs::canonicalize(repository)?,
        "repository must be the Git top-level, not a subtree",
    )?;
    let text = String::from_utf8(git_bytes(
        repository,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
        4 * 1024 * 1024,
    )?)
    .map_err(|_| ReviewFailure("non-UTF-8 source path unsupported".into()))?;
    let paths: std::collections::BTreeSet<_> = text
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    require(
        !paths.is_empty() && paths.len() <= 20_000,
        "empty or oversized source inventory",
    )?;
    Ok(Inventory { head, paths })
}
/// Fingerprint `root` over a fixed inventory, so a live checkout and a snapshot copy compare exactly.
pub(crate) fn fingerprint_tree(root: &Path, inventory: &Inventory) -> Result<String> {
    let mut remaining = 64 * 1024 * 1024;
    let mut framed = inventory.head.as_bytes().to_vec();
    for relative in &inventory.paths {
        let path = root.join(relative);
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                framed.extend_from_slice(b"deleted\0");
            }
            Err(e) => return Err(e.into()),
            Ok(meta) => {
                require(
                    meta.is_file() && !meta.file_type().is_symlink(),
                    "source inventory contains symlink or non-file",
                )?;
                let bytes = read_bytes(&path, remaining)?;
                remaining -= bytes.len() as u64;
                framed.extend_from_slice(&meta.permissions().mode().to_be_bytes());
                framed.extend_from_slice(sha256_hex(&bytes).as_bytes());
            }
        }
        framed.extend_from_slice(&(relative.len() as u64).to_be_bytes());
        framed.extend_from_slice(relative.as_bytes());
    }
    Ok(sha256_hex(&framed))
}
/// Copy exactly the inventoried files from `from` into a new private tree, preserving mode bits.
/// Inventoried-but-deleted paths stay absent, matching the fingerprint's `deleted` framing.
pub(crate) fn copy_tree(from: &Path, to: &Path, inventory: &Inventory) -> Result<()> {
    make_dir(to)?;
    for relative in &inventory.paths {
        let source = from.join(relative);
        let meta = match fs::symlink_metadata(&source) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            other => other?,
        };
        require(
            meta.is_file() && !meta.file_type().is_symlink(),
            "source inventory contains symlink or non-file",
        )?;
        let target = to.join(relative);
        if let Some(parent) = target.parent() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
        }
        write_new(&target, &read_bytes(&source, 64 * 1024 * 1024)?)?;
        fs::set_permissions(
            &target,
            fs::Permissions::from_mode(meta.permissions().mode()),
        )?;
    }
    Ok(())
}
fn verify_candidate(
    analysis: &serde_json::Value,
    repository: &Path,
    inventory: &Inventory,
    artifacts: &mut Vec<Artifact>,
    remaining: &mut usize,
) -> Result<()> {
    let files = analysis["review"]["files"]
        .as_array()
        .ok_or_else(|| ReviewFailure("missing candidate files".into()))?;
    let mut files = files.iter().collect::<Vec<_>>();
    files.sort_by_key(|f| f["path"].as_str());
    let mut captured = Vec::new();
    for file in files {
        let path = file["path"]
            .as_str()
            .ok_or_else(|| ReviewFailure("invalid file path".into()))?;
        require(
            !path.is_empty()
                && Path::new(path)
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_))),
            "unsafe candidate path",
        )?;
        require(
            matches!(
                file["status"].as_str(),
                Some("added" | "deleted" | "modified")
            ),
            "candidate adapter requires added/deleted/modified source files",
        )?;

        if file["status"] == "deleted" {
            require(
                !repository.join(path).exists(),
                "deleted candidate path still exists",
            )?;
            captured.push((path, None));
        } else {
            // An ignored file outside the fingerprint could change without staling the run.
            require(
                inventory.contains(path),
                "candidate file is outside the fingerprinted source inventory",
            )?;
            let absolute = fs::canonicalize(repository.join(path))?;
            require(
                absolute.starts_with(repository),
                "candidate escaped repository",
            )?;
            let artifact = capture(format!("candidate:{path}"), &absolute, remaining)?;
            captured.push((path, Some(artifacts.len())));
            artifacts.push(artifact);
        }
    }
    require(
        analysis["candidate_snapshot"].as_str()
            == Some(
                deep_diff_forge_core::changed_source_snapshot_id(
                    captured
                        .iter()
                        .map(|(path, index)| (*path, index.map(|i| artifacts[i].text.as_bytes()))),
                )
                .as_str(),
            ),
        "analysis candidate does not match the current workspace source bytes",
    )
}

fn manifest_digest(manifest: &Manifest) -> Result<String> {
    let mut unsealed = manifest.clone();
    unsealed.run_id.clear();
    Ok(sha256_hex(&serde_json::to_vec(&unsealed)?))
}
fn verify_analysis(analysis: &serde_json::Value, request: &Request) -> Result<String> {
    let review_id = analysis["review_id"]
        .as_str()
        .ok_or_else(|| ReviewFailure("missing review_id".into()))?
        .to_owned();
    require(
        review_id.len() == 64 && review_id.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid review_id",
    )?;
    let coverage = analysis["coverage"]
        .as_array()
        .ok_or_else(|| ReviewFailure("missing analysis coverage".into()))?;
    let files = analysis["review"]["files"]
        .as_array()
        .ok_or_else(|| ReviewFailure("missing changed files".into()))?;
    require(
        !files.is_empty() && coverage.len() == files.len(),
        "empty or mismatched analysis coverage",
    )?;
    let paths: std::collections::BTreeSet<_> =
        files.iter().filter_map(|f| f["path"].as_str()).collect();
    let covered: std::collections::BTreeSet<_> =
        coverage.iter().filter_map(|f| f["path"].as_str()).collect();
    require(
        paths == covered && paths.len() == files.len(),
        "coverage paths do not uniquely match files",
    )?;
    require(
        request.profile != Profile::ImportantDecision
            || coverage.iter().all(|f| f["coverage"] == "analyzed"),
        "important decision has missing/partial/unavailable source analysis",
    )?;
    Ok(review_id)
}
/// Capture a validated analysis, decision and caller-selected context into a new private run.
/// # Errors
/// Refuses pre-existing output, incomplete analysis, invalid roster or oversized/changed inputs.
pub fn prepare(mut request: Request, directory: &Path) -> Result<Manifest> {
    validate_request(&request)?;
    let repository = fs::canonicalize(&request.repository)?;
    require(repository.is_dir(), "repository must be a directory")?;
    request.repository = repository.display().to_string();
    let parent = fs::canonicalize(directory.parent().unwrap_or(Path::new(".")))?;
    require(
        !parent.starts_with(&repository),
        "run artifacts must be outside the reviewed repository",
    )?;
    require(
        !directory.exists(),
        "run directory already exists; never overwrite a review",
    )?;
    let inventory = inventory(&repository)?;
    let tree_before = fingerprint_tree(&repository, &inventory)?;
    let policy_bytes = POLICY.iter().map(|(_, text)| text.len()).sum::<usize>();
    require(
        policy_bytes < CONTEXT_CAP,
        "built-in policy exceeds context budget",
    )?;
    let mut remaining = CONTEXT_CAP - policy_bytes;
    let mut artifacts = vec![
        capture(
            "analysis".into(),
            Path::new(&request.analysis_file),
            &mut remaining,
        )?,
        capture(
            "decision".into(),
            Path::new(&request.decision_file),
            &mut remaining,
        )?,
    ];
    let analysis: serde_json::Value = serde_json::from_str(&artifacts[0].text)?;
    require(
        analysis["schema"] == "deep-diff-forge.analysis.v1",
        "expected shared-service analysis.v1",
    )?;
    let review_id = verify_analysis(&analysis, &request)?;
    require(
        !artifacts[1].text.trim().is_empty(),
        "decision/intent must be explicit",
    )?;
    for (i, path) in request.context_files.iter().enumerate() {
        artifacts.push(capture(
            format!("context-{i}"),
            Path::new(path),
            &mut remaining,
        )?);
    }
    verify_candidate(
        &analysis,
        &repository,
        &inventory,
        &mut artifacts,
        &mut remaining,
    )?;
    for (id, text) in POLICY {
        artifacts.push(Artifact {
            id: (*id).into(),
            label: "DDF pinned pstack / advisory antipattern policy".into(),
            sha256: sha256_hex(text.as_bytes()),
            text: (*text).into(),
        });
    }
    let tree_after = source_fingerprint(&repository)?;
    require(
        tree_before == tree_after,
        "workspace changed during capture",
    )?;
    let mut manifest = Manifest {
        schema: "deep-diff-forge.council.v1".into(),
        run_id: String::new(),
        review_id,
        source_tree: tree_before,
        request,
        artifacts,
    };
    manifest.run_id = manifest_digest(&manifest)?;
    make_dir(directory)?;
    make_dir(&directory.join("jobs"))?;
    make_dir(&directory.join("reports"))?;
    make_dir(&directory.join("workspace"))?;
    write_json(&directory.join("manifest.json"), &manifest)?;
    write_json(&directory.join("report.schema.json"), &report_schema())?;
    // Gates run on copies of this sealed snapshot, never on the mutable checkout.
    let snapshot = directory.join(SNAPSHOT_DIR);
    copy_tree(&repository, &snapshot, &inventory)?;
    write_json(&directory.join(INVENTORY_FILE), &inventory)?;
    require(
        fingerprint_tree(&snapshot, &inventory)? == manifest.source_tree,
        "workspace changed during snapshot export",
    )?;
    Ok(manifest)
}
/// Load a sealed local manifest and verify every embedded artifact digest.
/// # Errors
/// Refuses modified manifests, invalid requests and changed embedded context.
pub fn load_manifest(directory: &Path) -> Result<Manifest> {
    let manifest: Manifest = read_json(&directory.join("manifest.json"), 2 * 1024 * 1024)?;
    validate_request(&manifest.request)?;
    require(
        manifest.schema == "deep-diff-forge.council.v1"
            && manifest_digest(&manifest)? == manifest.run_id,
        "manifest seal mismatch",
    )?;
    for artifact in &manifest.artifacts {
        require(
            sha256_hex(artifact.text.as_bytes()) == artifact.sha256,
            "captured artifact digest mismatch",
        )?;
    }
    Ok(manifest)
}
pub(crate) fn report_path(directory: &Path, id: &str, round: Round) -> PathBuf {
    directory
        .join("reports")
        .join(format!("{id}-{}.json", round.label()))
}
pub(crate) fn reports(directory: &Path, manifest: &Manifest) -> Result<Vec<Report>> {
    let mut result = Vec::new();
    for reviewer in &manifest.request.reviewers {
        for round in [Round::Independent, Round::Challenge] {
            let path = report_path(directory, &reviewer.id, round);
            if path.exists() {
                let report = read_json(&path, REPORT_CAP)?;
                validate_report(manifest, &report)?;
                let qpath = directory
                    .join("jobs")
                    .join(format!(
                        "review-{}-{}",
                        report.reviewer_id,
                        report.round.label()
                    ))
                    .join("qualification.json");
                if qpath.exists() {
                    let q: Qualification = read_json(&qpath, REPORT_CAP)?;
                    require(
                        q.report_sha256.as_deref()
                            == Some(sha256_hex(&read_bytes(&path, REPORT_CAP)?).as_str()),
                        "accepted report changed after qualification",
                    )?;
                }
                result.push(report);
            }
        }
    }
    Ok(result)
}
pub(crate) fn executions(directory: &Path, manifest: &Manifest) -> Result<Vec<Execution>> {
    let mut result = Vec::new();
    for entry in fs::read_dir(directory.join("jobs"))? {
        let jobdir = entry?.path();
        let path = jobdir.join("execution.json");
        if path.exists() {
            let mut receipt: Execution = read_json(&path, REPORT_CAP)?;
            if receipt
                .argv
                .first()
                .is_some_and(|a| a == "controller:all-required-targets")
            {
                let children: Vec<Execution> =
                    read_json(&jobdir.join("children.json"), REPORT_CAP)?;
                require(
                    !children.is_empty() && children.iter().all(|c| c.run_id == receipt.run_id),
                    "invalid aggregate receipt",
                )?;
                if !children.iter().all(|c| c.outcome == Outcome::Passed) {
                    receipt.outcome = Outcome::Failed;
                }
            } else {
                require(
                    sha256_hex(&read_bytes(&jobdir.join("stdout.log"), 16 * 1024 * 1024)?)
                        == receipt.stdout_sha256,
                    "stdout artifact changed",
                )?;
                require(
                    sha256_hex(&read_bytes(&jobdir.join("stderr.log"), 16 * 1024 * 1024)?)
                        == receipt.stderr_sha256,
                    "stderr artifact changed",
                )?;
            }
            // Refusal markers are written create-new by the controller; a qualification may lower
            // an outcome but must never restore Passed over one.
            let refusal = if jobdir.join("protocol-error.txt").exists() {
                Some(Outcome::Failed)
            } else if jobdir.join("qualification-error.txt").exists() {
                Some(Outcome::Inconclusive)
            } else {
                None
            };
            let qualification_path = jobdir.join("qualification.json");
            if qualification_path.exists() {
                let q: Qualification = read_json(&qualification_path, REPORT_CAP)?;
                if let Some(expected) = q.witness_sha256 {
                    let witness = jobdir.join(MUTATION_WITNESS);
                    require(
                        sha256_hex(&read_bytes(&witness, 16 * 1024 * 1024)?) == expected,
                        "mutation witness changed",
                    )?;
                }
                receipt.outcome = q.outcome;
            } else if receipt.outcome == Outcome::Passed
                && receipt
                    .argv
                    .first()
                    .is_none_or(|a| a != "controller:all-required-targets")
            {
                receipt.outcome = Outcome::Inconclusive;
            }
            if let Some(refused) = refusal
                && receipt.outcome == Outcome::Passed
            {
                receipt.outcome = refused;
            }
            result.push(receipt);
        }
    }
    if let Some(index) = result.iter().position(|r| {
        r.job == "gate-fuzz"
            && r.argv
                .first()
                .is_some_and(|a| a == "controller:all-required-targets")
    }) {
        let passed = (0..manifest.request.fuzz_targets.len()).all(|i| {
            result
                .iter()
                .any(|r| r.job == format!("gate-fuzz-{i}") && r.outcome == Outcome::Passed)
        });
        result[index].outcome = if passed {
            Outcome::Passed
        } else {
            Outcome::Inconclusive
        };
    }
    Ok(result)
}
