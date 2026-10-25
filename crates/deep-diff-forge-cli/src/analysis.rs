// DDF integration: R02, R09, R19, R21. Validate that a patch transforms the captured base into the captured candidate before attaching semantics; export explicit coverage and snapshot scope.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#shared-service
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-cli--analysis
//! Snapshot-pair CLI adapter for the shared service (R02/R09/R19/R21).
//! See `docs/RECOMMENDATION_INTEGRATION.md#shared-service` for vault/code backlinks.
use deep_diff_forge_pipeline::review::{SourcePair, analyze_review};
use std::{
    fs,
    io::Read as _,
    path::{Component, Path},
};

pub(crate) fn run(args: &[String]) -> Result<String, String> {
    if args.len() != 3 {
        return Err("usage: deep-diff-forge analyze <patch-file> <base-directory> <candidate-directory> (emits analysis.v1 JSON)".into());
    }
    let patch = super::read_capped_or_exit(
        fs::File::open(&args[0]).map_err(|e| e.to_string())?,
        "patch file",
    );
    let files = deep_diff_forge_patch::parse(&patch).map_err(|e| e.to_string())?;
    let old_root = fs::canonicalize(&args[1]).map_err(|e| e.to_string())?;
    let new_root = fs::canonicalize(&args[2]).map_err(|e| e.to_string())?;
    if !old_root.is_dir() || !new_root.is_dir() {
        return Err("source roots must be directories".into());
    }
    let mut pairs = Vec::new();
    let mut remaining = deep_diff_forge_patch::DEFAULT_BYTE_BUDGET;
    for file in files {
        if !matches!(
            file.status,
            deep_diff_forge_core::FileStatus::Added
                | deep_diff_forge_core::FileStatus::Deleted
                | deep_diff_forge_core::FileStatus::Modified
        ) {
            continue;
        }
        let old = if file.status == deep_diff_forge_core::FileStatus::Added {
            None
        } else {
            read_source(&old_root, &file.path, &mut remaining)?
        };
        let new = if file.status == deep_diff_forge_core::FileStatus::Deleted {
            None
        } else {
            read_source(&new_root, &file.path, &mut remaining)?
        };
        pairs.push(SourcePair {
            path: file.path,
            old,
            new,
        });
    }
    analyze_review(patch, &pairs)
        .map(|b| b.to_json())
        .map_err(|e| e.to_string())
}
fn read_source(
    root: &Path,
    relative: &str,
    remaining: &mut usize,
) -> Result<Option<String>, String> {
    let path = Path::new(relative);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
        || relative.is_empty()
    {
        return Err(format!("unsafe source path {relative:?}"));
    }
    let mut target = root.to_path_buf();
    for component in path.components() {
        target.push(component);
        match fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!("source symlink rejected: {relative:?}"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        }
    }
    let canonical = fs::canonicalize(&target).map_err(|e| e.to_string())?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(format!(
            "source is not a regular file under its root: {relative:?}"
        ));
    }
    // Caller-owned local snapshot roots must remain stable during capture. This
    // path check is not a race-free filesystem sandbox against a concurrent writer.
    let file = fs::File::open(canonical).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(*remaining as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > *remaining {
        return Err("aggregate source byte budget exceeded".into());
    }
    *remaining -= bytes.len();
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| format!("source is not UTF-8: {relative:?}"))
}
