// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-sdk--lib
// DDF slice C: the stable Rust facade. Design/status: docs/ROADMAP.md#status-2026-10-26.
//! The stable Rust interface to Deep-Diff-Forge.
//!
//! Everything here is a thin layer over the crates the CLI and the daemon use, so an SDK caller gets
//! exactly their answers:
//!
//! - [`review`] parses a patch under the input contracts ([`Guards`]) and returns a [`Review`]
//!   whose [`Review::to_json`] is the `deep-diff-forge.review.v0` document
//!   `deep-diff-forge --stdin-patch --json` prints for the same bytes.
//! - [`rank`] orders a review's files by review risk.
//! - [`exit`] is the exit-code registry the CLI uses.
//! - `Client` (Unix only) calls a running daemon over its Unix socket.
//!
//! **Stability.** The SDK owns [`Guards`], [`Review`] and [`Error`]: the first two are opaque, and
//! [`Error`] is `#[non_exhaustive]`, so they can grow without breaking callers. [`ReviewFile`],
//! [`InputSeal`] and [`RankedFile`] are Deep-Diff-Forge's own model types, re-exported as the
//! review document's Rust form; they change only with that document's schema version. The public
//! signatures are pinned by `tests/api.rs`, and the review, rank and exit functions build on every
//! platform: only `Client` needs Unix.

use std::fmt;

pub use deep_diff_forge_core::exit;
pub use deep_diff_forge_core::{InputSeal, ReviewFile};
pub use deep_diff_forge_graph::RankedFile;

/// Which input contracts [`review`] applies: `Guards::none()`, then `require_files` and
/// `require_hunks`, as the CLI's `--require-files` and `--require-hunks`. There is no `Default`:
/// a caller names the contracts it means, starting from [`Guards::none`] or [`Guards::all`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Guards(deep_diff_forge_patch::InputGuards);

impl Guards {
    /// No contract: any patch that parses is reviewed, even an empty one.
    #[must_use]
    pub fn none() -> Self {
        Self(deep_diff_forge_patch::InputGuards {
            files: false,
            hunks: false,
        })
    }

    /// Both contracts, as a gate should apply them.
    #[must_use]
    pub fn all() -> Self {
        Self::none().require_files().require_hunks()
    }

    /// Refuse a patch with 0 files.
    #[must_use]
    pub fn require_files(mut self) -> Self {
        self.0.files = true;
        self
    }

    /// Refuse a patch with 0 hunks or 0 changed lines.
    #[must_use]
    pub fn require_hunks(mut self) -> Self {
        self.0.hunks = true;
        self
    }
}

/// Why a call into the SDK did not produce its result.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The patch could not be parsed (CLI exit 4, daemon error 4).
    Parse(String),
    /// A well-formed patch broke a contract the caller required (CLI exit 7, daemon error 7).
    Refused(&'static str),
    /// The daemon answered with a JSON-RPC error.
    Rpc { code: i64, message: String },
    /// The daemon could not be reached, or its answer was not a valid response to the request.
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
    /// The CLI exit code (engine family) this error corresponds to. A daemon's domain errors keep
    /// their meaning: error 4 is invalid input and error 7 a refused contract, as on the CLI
    /// (council sdc1).
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Parse(_) => exit::INVALID_INPUT,
            Self::Refused(_) => exit::INPUT_CONTRACT_REFUSED,
            Self::Rpc { code, .. } if *code == i64::from(exit::INVALID_INPUT) => {
                exit::INVALID_INPUT
            }
            Self::Rpc { code, .. } if *code == i64::from(exit::INPUT_CONTRACT_REFUSED) => {
                exit::INPUT_CONTRACT_REFUSED
            }
            Self::Rpc { .. } | Self::Transport(_) => exit::DAEMON_OR_TERMINAL,
        }
    }
}

/// A parsed patch, sealed over the exact bytes it was parsed from.
#[derive(Debug, Clone)]
pub struct Review {
    files: Vec<ReviewFile>,
    seal: InputSeal,
}

impl Review {
    /// The files, hunks and lines of the patch.
    #[must_use]
    pub fn files(&self) -> &[ReviewFile] {
        &self.files
    }

    /// The digest of the input bytes and the tool version that read them.
    #[must_use]
    pub fn seal(&self) -> &InputSeal {
        &self.seal
    }

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
pub fn review(patch: &str, guards: Guards) -> Result<Review, Error> {
    let files = deep_diff_forge_patch::parse(patch).map_err(|e| Error::Parse(e.to_string()))?;
    if let Some(reason) = guards.0.refusal(&files) {
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

#[cfg(unix)]
pub use client::Client;

#[cfg(unix)]
mod client {
    use super::{Error, parse_response};
    use serde_json::Value;
    use std::path::Path;

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
        /// [`Error::Rpc`] when the daemon answers with an error; [`Error::Transport`] when it
        /// cannot be reached or its answer is not a valid response to this request.
        pub fn call(&self, method: &str, params: &Value) -> Result<Value, Error> {
            const ID: i64 = 1;
            let line =
                serde_json::json!({"jsonrpc": "2.0", "id": ID, "method": method, "params": params})
                    .to_string();
            let answer = deep_diff_forge_daemon::request(&self.location, &line)
                .map_err(|e| Error::Transport(e.to_string()))?;
            parse_response(&answer, ID)
        }
    }
}

/// Validate a JSON-RPC 2.0 response to request `id` and take its result: the version, the id and
/// the error object's shape are all checked, so an unrelated or malformed answer is never taken
/// for this call's (council sdc1). Only `Client` and the tests use it, so non-Unix builds omit it.
#[cfg(any(unix, test))]
fn parse_response(answer: &str, id: i64) -> Result<serde_json::Value, Error> {
    use serde_json::Value;
    let bad = |what: &str| Error::Transport(format!("not a valid response: {what}"));
    let mut response: Value =
        serde_json::from_str(answer.trim()).map_err(|e| bad(&e.to_string()))?;
    if response.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(bad("jsonrpc is not \"2.0\""));
    }
    // An invalid request is answered with id null (JSON-RPC 2.0 §5); one request per exchange makes
    // that error this call's, so only an error may carry it.
    let null_error = response.get("id") == Some(&Value::Null) && response.get("result").is_none();
    if response.get("id").and_then(Value::as_i64) != Some(id) && !null_error {
        return Err(bad(&format!("its id is not {id}")));
    }
    match (
        response.get_mut("result").map(Value::take),
        response.get("error"),
    ) {
        (Some(result), None) => Ok(result),
        (None, Some(error)) => match (
            error.get("code").and_then(Value::as_i64),
            error.get("message").and_then(Value::as_str),
        ) {
            (Some(code), Some(message)) => Err(Error::Rpc {
                code,
                message: message.to_owned(),
            }),
            _ => Err(bad("the error has no integer code and string message")),
        },
        _ => Err(bad("it must have exactly one of result and error")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n";

    #[test]
    fn review_seals_the_bytes_and_applies_the_contracts() {
        let review = review(PATCH, Guards::all()).unwrap();
        assert_eq!(review.files().len(), 1);
        assert!(
            review
                .to_json()
                .contains("\"schema\": \"deep-diff-forge.review.v0\"")
        );
        assert_eq!(
            review.seal(),
            &InputSeal::of(PATCH.as_bytes(), env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(rank(&review).len(), 1);
        assert_eq!(
            super::review("", Guards::all()).unwrap_err(),
            Error::Refused("0 files in input (--require-files)")
        );
        assert_eq!(
            super::review("", Guards::none().require_hunks()).unwrap_err(),
            Error::Refused("0 hunks in input (--require-hunks)")
        );
        assert!(super::review("", Guards::none().require_files()).is_err());
        assert!(super::review("diff --git a/x b/x\n", Guards::none().require_files()).is_ok());
        assert!(super::review("", Guards::none()).is_ok());
        let err = super::review("--- a/x\n+++ b/x\n+stray\n", Guards::all()).unwrap_err();
        assert!(matches!(err, Error::Parse(_)), "{err}");
    }

    /// The daemon's domain codes for a parse failure and a refused contract are the CLI's exit
    /// codes, which is what lets `Error::exit_code` match them by the registry's names.
    #[cfg(unix)]
    #[test]
    fn the_daemons_domain_codes_are_the_cli_exit_codes() {
        use deep_diff_forge_daemon::{INPUT_CONTRACT_REFUSED, PATCH_PARSE_FAILED};
        assert_eq!(PATCH_PARSE_FAILED, i64::from(exit::INVALID_INPUT));
        assert_eq!(
            INPUT_CONTRACT_REFUSED,
            i64::from(exit::INPUT_CONTRACT_REFUSED)
        );
    }

    #[test]
    fn errors_map_to_the_registry_and_read_well() {
        assert_eq!(Error::Parse("x".into()).exit_code(), 4);
        assert_eq!(Error::Refused("r").exit_code(), 7);
        assert_eq!(
            Error::Rpc {
                code: 4,
                message: "m".into()
            }
            .exit_code(),
            4
        );
        assert_eq!(
            Error::Rpc {
                code: 7,
                message: "m".into()
            }
            .exit_code(),
            7
        );
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
    fn responses_are_validated_before_they_are_believed() {
        assert_eq!(
            parse_response(r#"{"jsonrpc":"2.0","id":1,"result":{"a":1}}"#, 1).unwrap()["a"],
            1
        );
        assert_eq!(
            parse_response(
                r#"{"jsonrpc":"2.0","id":1,"error":{"code":7,"message":"refused: x"}}"#,
                1
            ),
            Err(Error::Rpc {
                code: 7,
                message: "refused: x".into()
            })
        );
        // An invalid request is answered with id null (JSON-RPC 2.0 §5): still the daemon's error.
        assert_eq!(
            parse_response(
                r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"bad method"}}"#,
                1
            ),
            Err(Error::Rpc {
                code: -32600,
                message: "bad method".into()
            })
        );
        for bad in [
            r#"{"jsonrpc":"2.0","id":null,"result":{}}"#,
            r#"{"jsonrpc":"2.0","error":{"code":-32600,"message":"no id at all"}}"#,
            r#"{"jsonrpc":"2.0","id":2,"result":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"error":{"code":4,"message":"other call"}}"#,
            r#"{"jsonrpc":"1.0","id":1,"result":{}}"#,
            r#"{"id":1,"result":{}}"#,
            r#"{"jsonrpc":"2.0","result":{}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"message":"no code"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":4}}"#,
            r#"{"jsonrpc":"2.0","id":1}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":1,"error":{"code":1,"message":"m"}}"#,
            "not json",
        ] {
            assert!(
                matches!(parse_response(bad, 1), Err(Error::Transport(_))),
                "{bad}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_daemon_is_a_transport_error() {
        let client = Client::at(std::path::Path::new("/nonexistent/deep-diff-forge.sock"));
        assert!(matches!(
            client.call("daemon.health", &serde_json::Value::Null),
            Err(Error::Transport(_))
        ));
    }
}
