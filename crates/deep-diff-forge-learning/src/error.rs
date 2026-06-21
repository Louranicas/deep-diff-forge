//! Error type for the learning loop.
//!
//! The loop is fail-soft by design: a missing or unreadable learning store must
//! never break a review run. Callers that want best-effort behaviour can treat
//! any error as "no learning data yet" and proceed with built-in defaults.

use std::fmt;

/// Errors the learning loop can surface.
#[derive(Debug)]
pub enum LearningError {
    /// The home/state directory could not be resolved (no `$XDG_STATE_HOME`,
    /// no `$HOME`).
    NoStateDir,
    /// A filesystem operation failed.
    Io(std::io::Error),
    /// A receipt or learning record could not be serialized.
    Serialize(String),
    /// A stored line could not be parsed back into a record.
    Deserialize(String),
}

impl fmt::Display for LearningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoStateDir => {
                write!(
                    f,
                    "could not resolve a state directory ($XDG_STATE_HOME or $HOME)"
                )
            }
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Serialize(e) => write!(f, "serialize error: {e}"),
            Self::Deserialize(e) => write!(f, "deserialize error: {e}"),
        }
    }
}

impl std::error::Error for LearningError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for LearningError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_no_state_dir() {
        let s = LearningError::NoStateDir.to_string();
        assert!(s.contains("state directory"));
    }

    #[test]
    fn display_io() {
        let e = LearningError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "nope"));
        assert!(e.to_string().contains("io error"));
        assert!(e.to_string().contains("nope"));
    }

    #[test]
    fn display_serialize_and_deserialize() {
        assert!(
            LearningError::Serialize("x".into())
                .to_string()
                .contains("serialize")
        );
        assert!(
            LearningError::Deserialize("y".into())
                .to_string()
                .contains("deserialize")
        );
    }

    #[test]
    fn from_io_error() {
        let e: LearningError =
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied").into();
        assert!(matches!(e, LearningError::Io(_)));
    }

    #[test]
    fn io_error_exposes_source() {
        use std::error::Error as _;
        let e = LearningError::Io(std::io::Error::other("boom"));
        assert!(e.source().is_some());
    }

    #[test]
    fn non_io_errors_have_no_source() {
        use std::error::Error as _;
        assert!(LearningError::NoStateDir.source().is_none());
        assert!(LearningError::Serialize("x".into()).source().is_none());
    }
}
