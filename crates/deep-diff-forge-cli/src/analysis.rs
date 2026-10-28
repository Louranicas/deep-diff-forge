// DDF integration: R02, R09, R19, R21. Validate that a patch transforms the captured base into the captured candidate before attaching semantics; export explicit coverage and snapshot scope.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#shared-service
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-cli--analysis
//! Snapshot-pair CLI adapter for the shared service (R02/R09/R19/R21).
//! See `docs/RECOMMENDATION_INTEGRATION.md#shared-service` for vault/code backlinks.
use deep_diff_forge_pipeline::review::{SourcePair, analyze_review};
use std::{
    fmt, fs,
    io::Read as _,
    os::unix::fs::OpenOptionsExt as _,
    path::{Component, Path},
};

#[derive(Debug)]
pub(crate) enum Error {
    Io(String),
    Invalid(String),
}

impl Error {
    pub(crate) fn exit_code(&self) -> i32 {
        match self {
            Self::Io(_) => deep_diff_forge_core::exit::IO,
            Self::Invalid(_) => deep_diff_forge_core::exit::INVALID_INPUT,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) | Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

pub(crate) fn run(args: &[String]) -> Result<String, Error> {
    if args.len() != 3 {
        return Err(Error::Invalid("usage: deep-diff-forge analyze <patch-file> <base-directory> <candidate-directory> (emits analysis.v1 JSON)".into()));
    }
    let patch = read_patch(Path::new(&args[0]))?;
    let files =
        deep_diff_forge_patch::parse(&patch).map_err(|error| Error::Invalid(error.to_string()))?;
    let old_root = fs::canonicalize(&args[1]).map_err(|error| Error::Io(error.to_string()))?;
    let new_root = fs::canonicalize(&args[2]).map_err(|error| Error::Io(error.to_string()))?;
    if !old_root.is_dir() || !new_root.is_dir() {
        return Err(Error::Invalid("source roots must be directories".into()));
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
        .map_err(|error| Error::Invalid(error.to_string()))
}

fn open_regular(path: &Path, what: &str) -> Result<fs::File, Error> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| Error::Io(error.to_string()))?;
    if !file
        .metadata()
        .map_err(|error| Error::Io(error.to_string()))?
        .is_file()
    {
        return Err(Error::Io(format!("{what} is not a regular file")));
    }
    Ok(file)
}

fn read_patch(path: &Path) -> Result<String, Error> {
    let file = open_regular(path, "patch file")?;
    let cap = deep_diff_forge_patch::DEFAULT_BYTE_BUDGET;
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::Io(error.to_string()))?;
    if bytes.len() > cap {
        return Err(Error::Invalid(format!(
            "patch file exceeds the {cap}-byte input budget"
        )));
    }
    String::from_utf8(bytes).map_err(|_| Error::Io("patch file is not valid UTF-8".into()))
}

fn read_source(
    root: &Path,
    relative: &str,
    remaining: &mut usize,
) -> Result<Option<String>, Error> {
    let path = Path::new(relative);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
        || relative.is_empty()
    {
        return Err(Error::Invalid(format!("unsafe source path {relative:?}")));
    }
    let mut target = root.to_path_buf();
    for component in path.components() {
        target.push(component);
        match fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(Error::Invalid(format!(
                    "source symlink rejected: {relative:?}"
                )));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::Io(error.to_string())),
        }
    }
    let canonical = fs::canonicalize(&target).map_err(|error| Error::Io(error.to_string()))?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(Error::Io(format!(
            "source is not a regular file under its root: {relative:?}"
        )));
    }
    // Caller-owned local snapshot roots must remain stable during capture. This
    // path check is not a race-free filesystem sandbox against a concurrent writer.
    let file = open_regular(&canonical, "source")?;
    let mut bytes = Vec::new();
    file.take(*remaining as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::Io(error.to_string()))?;
    if bytes.len() > *remaining {
        return Err(Error::Invalid(
            "aggregate source byte budget exceeded".into(),
        ));
    }
    *remaining -= bytes.len();
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| Error::Invalid(format!("source is not UTF-8: {relative:?}")))
}
