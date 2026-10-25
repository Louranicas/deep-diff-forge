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

/// Read a controller artifact. `O_NOFOLLOW` refuses a symlink leaf and `O_NONBLOCK` keeps a FIFO from
/// blocking the open; `fstat` then refuses anything that is not a regular file before a byte is read.
pub(crate) fn read_bytes(path: &Path, cap: u64) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| match error.raw_os_error() {
            Some(nix::libc::ELOOP) => ReviewFailure("symlink artifact rejected".into()),
            _ => error.into(),
        })?;
    require(file.metadata()?.is_file(), "non-regular artifact rejected")?;
    let mut data = Vec::new();
    file.take(cap + 1).read_to_end(&mut data)?;
    require(
        data.len() as u64 <= cap,
        "artifact exceeds acquisition budget",
    )?;
    Ok(data)
}
/// Metadata of `root/relative` without following a symlink in any component below the trusted
/// `root`; `None` if a component is absent. Every intermediate component must be a real directory.
pub(crate) fn untrusted_metadata(root: &Path, relative: &Path) -> Result<Option<fs::Metadata>> {
    let components = relative.components().collect::<Vec<_>>();
    require(
        !components.is_empty()
            && components
                .iter()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
        "unsafe relative artifact path",
    )?;
    let mut path = root.to_path_buf();
    for (i, component) in components.iter().enumerate() {
        path.push(component);
        let meta = match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            other => other?,
        };
        if i + 1 == components.len() {
            return Ok(Some(meta));
        }
        require(
            meta.is_dir(),
            "symlink or non-directory component in an artifact path",
        )?;
    }
    Ok(None)
}
/// Read a regular file a sandboxed job could have written, under a trusted `root`, refusing a symlink
/// in any component and any FIFO, socket or device. Callers read only after the container is gone.
pub(crate) fn read_untrusted(root: &Path, relative: &Path, cap: u64) -> Result<Vec<u8>> {
    let meta = untrusted_metadata(root, relative)?
        .ok_or_else(|| ReviewFailure("sandbox artifact missing".into()))?;
    require(meta.is_file(), "sandbox artifact is not a regular file")?;
    read_bytes(&root.join(relative), cap)
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
    pub(crate) fn paths(&self) -> impl Iterator<Item = &str> {
        self.paths.iter().map(String::as_str)
    }
}
pub(crate) const SNAPSHOT_DIR: &str = "source";
/// The candidate's changes since `mutation_base`, sealed at prepare for the diff-scoped mutation gate.
pub(crate) const MUTATION_DIFF: &str = "mutation.diff";
pub(crate) const INVENTORY_FILE: &str = "source-inventory.json";
/// Largest sealed inventory a gate reads back. Written as a shift, so the only arithmetic mutant
/// (a zero cap) refuses every gate.
pub(crate) const INVENTORY_CAP: u64 = 4 << 20;
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
        match untrusted_metadata(root, Path::new(relative))? {
            None => framed.extend_from_slice(b"deleted\0"),
            Some(meta) => {
                require(
                    meta.is_file(),
                    "source inventory contains symlink or non-file",
                )?;
                let bytes = read_bytes(&root.join(relative), remaining)?;
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
        let Some(meta) = untrusted_metadata(from, Path::new(relative))? else {
            continue;
        };
        require(
            meta.is_file(),
            "source inventory contains symlink or non-file",
        )?;
        let target = to.join(relative);
        if let Some(parent) = target.parent() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
        }
        write_new(
            &target,
            &read_bytes(&from.join(relative), 64 * 1024 * 1024)?,
        )?;
        fs::set_permissions(
            &target,
            fs::Permissions::from_mode(meta.permissions().mode()),
        )?;
    }
    Ok(())
}
/// Identity of a directory tree that a sandboxed job produced and the controller sealed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TreeSeal {
    pub(crate) sha256: String,
    pub(crate) files: u64,
    pub(crate) bytes: u64,
}
/// Budget for a sealed tree; the measured DDF vendor tree is 9,008 files and 204 MB. Directories
/// count against the entry budget too, so an output of empty directories is bounded.
const SEAL_ENTRIES: u64 = 100_000;
const SEAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Walk `root` in sorted order, accepting only real directories and regular files within the budget,
/// and digest names, kinds, executable bits and contents. With `copy`, also recreate the tree there
/// as a new controller-owned directory with read-only files. The walk never follows a symlink.
fn walk_tree(root: &Path, copy: Option<&Path>) -> Result<TreeSeal> {
    require(
        fs::symlink_metadata(root)?.is_dir(),
        "sandbox output root is not a real directory",
    )?;
    if let Some(to) = copy {
        make_dir(to)?;
    }
    let mut seal = TreeSeal {
        sha256: String::new(),
        files: 0,
        bytes: 0,
    };
    let mut framed = Vec::new();
    let mut entries = 0_u64;
    let mut pending = vec![PathBuf::new()];
    while let Some(directory) = pending.pop() {
        // Listed lazily up to the remaining budget, so a huge directory is refused, not collected.
        let remaining = usize::try_from(SEAL_ENTRIES.saturating_sub(entries)).unwrap_or(usize::MAX);
        let mut names = fs::read_dir(root.join(&directory))?
            .take(remaining.saturating_add(1))
            .map(|entry| entry.map(|e| e.file_name()))
            .collect::<std::io::Result<Vec<_>>>()?;
        entries += names.len() as u64;
        require(
            entries <= SEAL_ENTRIES,
            "sandbox output exceeds entry budget",
        )?;
        names.sort_unstable();
        let mut subdirectories = Vec::new();
        for name in names {
            let relative = directory.join(name);
            let path = root.join(&relative);
            let meta = fs::symlink_metadata(&path)?;
            let encoded = relative.as_os_str().as_encoded_bytes();
            if meta.is_dir() {
                framed.push(b'd');
                if let Some(to) = copy {
                    make_dir(&to.join(&relative))?;
                }
                subdirectories.push(relative.clone());
            } else if meta.is_file() {
                seal.files += 1;
                let bytes = read_bytes(&path, SEAL_BYTES.saturating_sub(seal.bytes))?;
                seal.bytes += bytes.len() as u64;
                let executable = meta.permissions().mode() & 0o111 != 0;
                framed.push(if executable { b'x' } else { b'f' });
                framed.extend_from_slice(sha256_hex(&bytes).as_bytes());
                if let Some(to) = copy {
                    // Not synced per file: the seal digest is re-verified before every use instead.
                    OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(if executable { 0o500 } else { 0o400 })
                        .open(to.join(&relative))?
                        .write_all(&bytes)?;
                }
            } else {
                return Err(ReviewFailure(format!(
                    "sandbox output contains a symlink or special file: {}",
                    relative.display()
                )));
            }
            framed.extend_from_slice(&(encoded.len() as u64).to_be_bytes());
            framed.extend_from_slice(encoded);
        }
        // Reversed onto the stack, so subdirectories are also visited in sorted order.
        pending.extend(subdirectories.into_iter().rev());
    }
    seal.sha256 = sha256_hex(&framed);
    Ok(seal)
}
/// Copy a tree a sandboxed job wrote into a new controller-owned directory, refusing symlinks,
/// special files and oversized output; returns the seal that later mounts are verified against.
pub(crate) fn seal_tree(from: &Path, to: &Path) -> Result<TreeSeal> {
    // Validate before creating anything, so a refused output leaves no partial copy behind.
    let seal = walk_tree(from, None)?;
    require(
        walk_tree(from, Some(to))? == seal && walk_tree(to, None)? == seal,
        "sealed copy does not match the sandbox output",
    )?;
    Ok(seal)
}
/// Re-measure a sealed tree.
pub(crate) fn tree_seal(root: &Path) -> Result<TreeSeal> {
    walk_tree(root, None)
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
    // Captured inside the same unchanged-workspace window as everything else.
    let mutation_diff =
        crate::mutation_scope::capture(&repository, &inventory, request.mutation_base.as_deref())?;
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
        mutation_diff_sha256: mutation_diff.as_deref().map(sha256_hex),
    };
    manifest.run_id = manifest_digest(&manifest)?;
    make_dir(directory)?;
    make_dir(&directory.join("jobs"))?;
    make_dir(&directory.join("reports"))?;
    make_dir(&directory.join("workspace"))?;
    write_json(&directory.join("manifest.json"), &manifest)?;
    write_json(&directory.join("report.schema.json"), &report_schema())?;
    if let Some(diff) = &mutation_diff {
        write_new(&directory.join(MUTATION_DIFF), diff)?;
    }
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
    require(
        manifest.request.mutation_base.is_some() == manifest.mutation_diff_sha256.is_some(),
        "mutation base and sealed mutation diff must appear together",
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
                    let witness =
                        read_untrusted(&jobdir, Path::new(MUTATION_WITNESS), 16 * 1024 * 1024)?;
                    require(sha256_hex(&witness) == expected, "mutation witness changed")?;
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
