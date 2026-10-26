// DDF slice C: the stable Rust facade. Design/status: docs/ROADMAP.md#status-2026-10-26.
//! The stable Rust interface to Deep-Diff-Forge.
//!
//! Everything here is a thin layer over the crates the CLI and the daemon use, so an SDK caller gets
//! exactly their answers:
//!
//! - [`review`] parses a patch under the input contracts ([`InputGuards`]) and returns a
//!   [`Review`] whose [`Review::to_json`] is the `deep-diff-forge.review.v0` document
//!   `deep-diff-forge --stdin-patch --json` prints for the same bytes.
//! - [`rank`] orders a review's files by review risk.
//! - [`exit`] is the exit-code registry the CLI uses.
//! - [`Client`] calls a running daemon over its Unix socket.
//!
//! The public signatures are pinned by `tests/api.rs`: changing one is a breaking change and must
//! be a deliberate edit there.

use serde_json::Value;
use std::fmt;
use std::path::Path;

pub use deep_diff_forge_core::exit;
pub use deep_diff_forge_core::{InputSeal, ReviewFile};
pub use deep_diff_forge_graph::RankedFile;
pub use deep_diff_forge_patch::InputGuards;

/// Why a call into the SDK did not produce its result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The patch could not be parsed (CLI exit 4, daemon error 4).
    Parse(String),
    /// A well-formed patch broke a contract the caller required (CLI exit 7, daemon error 7).
    Refused(&'static str),
    /// The daemon answered with a JSON-RPC error.
    Rpc { code: i64, message: String },
    /// The daemon could not be reached, or its answer was not a JSON-RPC response.
    Transport(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(message) => write!(f, "patch parse failed: {message}"),
            Self::Refused(reason) => write!(f, "refused: {reason}"),
            Self::Rpc { code, message } => write!(f, "daemon error {code}: {message}"),
            Self::Transport(message) => write!(f, "daemon transport failed: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    /// The CLI exit code (engine family) this error corresponds to.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Parse(_) => exit::INVALID_INPUT,
            Self::Refused(_) => exit::INPUT_CONTRACT_REFUSED,
            Self::Rpc { .. } | Self::Transport(_) => exit::DAEMON_OR_TERMINAL,
        }
    }
}

/// A parsed patch, sealed over the exact bytes it was parsed from.
#[derive(Debug, Clone)]
pub struct Review {
    /// The files, hunks and lines of the patch.
    pub files: Vec<ReviewFile>,
    /// The digest of the input bytes and the tool version that read them.
    pub seal: InputSeal,
}

impl Review {
    /// The `deep-diff-forge.review.v0` document, byte for byte what the CLI prints for the same
    /// patch.
    #[must_use]
    pub fn to_json(&self) -> String {
        deep_diff_forge_patch::to_json_sealed(&self.files, &self.seal)
    }
}

/// Parse `patch` and apply the requested input contracts, as `--stdin-patch` does.
///
/// # Errors
///
/// [`Error::Parse`] for a patch that does not parse; [`Error::Refused`] for one that breaks a
/// contract in `guards`.
pub fn review(patch: &str, guards: InputGuards) -> Result<Review, Error> {
    let files = deep_diff_forge_patch::parse(patch).map_err(|e| Error::Parse(e.to_string()))?;
    if let Some(reason) = guards.refusal(&files) {
        return Err(Error::Refused(reason));
    }
    Ok(Review {
        files,
        seal: InputSeal::of(patch.as_bytes(), env!("CARGO_PKG_VERSION")),
    })
}

/// The review's files ordered by review risk (the daemon's `diff.plan`).
#[must_use]
pub fn rank(review: &Review) -> Vec<RankedFile> {
    deep_diff_forge_graph::rank(&review.files)
}

/// A client for a running `deep-diff-forge daemon`. Each call is one request on a fresh
/// connection, under the daemon's own size and time limits.
#[derive(Debug, Clone)]
pub struct Client {
    location: deep_diff_forge_daemon::SocketLocation,
}

impl Client {
    /// A client for the daemon listening at `socket`.
    #[must_use]
    pub fn at(socket: &Path) -> Self {
        Self {
            location: deep_diff_forge_daemon::SocketLocation::at(socket),
        }
    }

    /// Call `method` with `params` (an object, or null) and return its result.
    ///
    /// # Errors
    ///
    /// [`Error::Rpc`] when the daemon answers with an error; [`Error::Transport`] when it cannot be
    /// reached or its answer is not a JSON-RPC response.
    pub fn call(&self, method: &str, params: &Value) -> Result<Value, Error> {
        let line =
            serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
                .to_string();
        let answer = deep_diff_forge_daemon::request(&self.location, &line)
            .map_err(|e| Error::Transport(e.to_string()))?;
        let mut response: Value = serde_json::from_str(answer.trim())
            .map_err(|e| Error::Transport(format!("not JSON: {e}")))?;
        if let Some(error) = response.get("error") {
            return Err(Error::Rpc {
                code: error["code"].as_i64().unwrap_or_default(),
                message: error["message"].as_str().unwrap_or_default().to_owned(),
            });
        }
        response
            .get_mut("result")
            .map(Value::take)
            .ok_or_else(|| Error::Transport("a response with neither result nor error".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n";

    #[test]
    fn review_seals_the_bytes_and_applies_the_contracts() {
        let both = InputGuards {
            files: true,
            hunks: true,
        };
        let review = review(PATCH, both).unwrap();
        assert_eq!(review.files.len(), 1);
        assert!(
            review
                .to_json()
                .contains("\"schema\": \"deep-diff-forge.review.v0\"")
        );
        assert_eq!(
            review.seal,
            InputSeal::of(PATCH.as_bytes(), env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(rank(&review).len(), 1);
        assert_eq!(
            super::review("", both).unwrap_err(),
            Error::Refused("0 files in input (--require-files)")
        );
        assert!(super::review("", InputGuards::default()).is_ok());
        let err = super::review("--- a/x\n+++ b/x\n+stray\n", both).unwrap_err();
        assert!(matches!(err, Error::Parse(_)), "{err}");
    }

    #[test]
    fn errors_map_to_the_registry_and_read_well() {
        assert_eq!(Error::Parse("x".into()).exit_code(), 4);
        assert_eq!(Error::Refused("r").exit_code(), 7);
        assert_eq!(
            Error::Rpc {
                code: 1,
                message: "m".into()
            }
            .exit_code(),
            6
        );
        assert_eq!(Error::Transport("t".into()).exit_code(), 6);
        assert_eq!(Error::Refused("0 files").to_string(), "refused: 0 files");
        assert_eq!(
            Error::Parse("p".into()).to_string(),
            "patch parse failed: p"
        );
        assert_eq!(
            Error::Rpc {
                code: 4,
                message: "m".into()
            }
            .to_string(),
            "daemon error 4: m"
        );
        assert_eq!(
            Error::Transport("t".into()).to_string(),
            "daemon transport failed: t"
        );
    }

    #[test]
    fn a_missing_daemon_is_a_transport_error() {
        let client = Client::at(Path::new("/nonexistent/deep-diff-forge.sock"));
        assert!(matches!(
            client.call("daemon.health", &Value::Null),
            Err(Error::Transport(_))
        ));
    }
}
