//! One path from input bytes to a sealed review (anchor C-05): decode, parse, apply the caller's
//! input contracts, and seal the same bytes. The CLI, the daemon and the SDK each wrote this
//! sequence out; with one constructor, a seal cannot be paired with files parsed from other bytes.

use crate::{InputGuards, ParseOptions, PatchParseError, decode, parse_decoded};
use deep_diff_forge_core::{InputSeal, ReviewFile};

/// Why [`SealedReview::new`] made no review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewError {
    /// The input is not a patch.
    Parse(PatchParseError),
    /// The patch parsed but broke a contract the caller required; the reason names it.
    Refused(&'static str),
}

impl std::fmt::Display for ReviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(error) => write!(f, "patch parse failed: {error}"),
            Self::Refused(reason) => write!(f, "refused: {reason}"),
        }
    }
}

impl std::error::Error for ReviewError {}

/// A parsed patch and the seal over the exact bytes it was parsed from. Only
/// [`SealedReview::new`] makes one, so the two always describe the same input.
///
/// ```
/// use deep_diff_forge_patch::{InputGuards, SealedReview};
/// let none = InputGuards { files: false, hunks: false };
/// let review = SealedReview::new(b"", none, "0.0.0").unwrap();
/// assert!(review.files().is_empty());
/// ```
///
/// ```compile_fail
/// let _ = deep_diff_forge_patch::SealedReview { files: Vec::new(), seal: todo!() };
/// ```
#[derive(Debug, Clone)]
pub struct SealedReview {
    files: Vec<ReviewFile>,
    seal: InputSeal,
}

impl SealedReview {
    /// Decode `input` (lossily per file, as `decode` does), parse it, refuse it if it breaks
    /// `guards`, and seal the same bytes with `version`.
    ///
    /// # Errors
    ///
    /// [`ReviewError::Parse`] when `input` is not a patch; [`ReviewError::Refused`] when it breaks
    /// a requested contract.
    pub fn new(input: &[u8], guards: InputGuards, version: &str) -> Result<Self, ReviewError> {
        let files =
            parse_decoded(&decode(input), ParseOptions::default()).map_err(ReviewError::Parse)?;
        if let Some(reason) = guards.refusal(&files) {
            return Err(ReviewError::Refused(reason));
        }
        Ok(Self {
            files,
            seal: InputSeal::of(input, version),
        })
    }

    /// Pair `files` with a seal over `input`, for a path inside this crate that parsed `files` from
    /// exactly `input` another way (the Git capture).
    pub(crate) fn from_capture(files: Vec<ReviewFile>, input: &[u8], version: &str) -> Self {
        Self {
            files,
            seal: InputSeal::of(input, version),
        }
    }

    /// The files, hunks and lines of the patch.
    #[must_use]
    pub fn files(&self) -> &[ReviewFile] {
        &self.files
    }

    /// The digest of the input bytes and the version that read them.
    #[must_use]
    pub fn seal(&self) -> &InputSeal {
        &self.seal
    }

    /// The `deep-diff-forge.review.v0` document.
    #[cfg(feature = "json")]
    #[must_use]
    pub fn to_json(&self) -> String {
        crate::to_json_sealed(&self.files, &self.seal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &[u8] = b"diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";
    const NONE: InputGuards = InputGuards {
        files: false,
        hunks: false,
    };

    #[test]
    fn the_seal_covers_the_bytes_the_files_came_from() {
        let review = SealedReview::new(PATCH, NONE, "1.2.3").unwrap();
        assert_eq!(review.files().len(), 1);
        assert_eq!(review.seal(), &InputSeal::of(PATCH, "1.2.3"));
        #[cfg(feature = "json")]
        assert_eq!(
            review.to_json(),
            crate::to_json_sealed(review.files(), review.seal())
        );
    }

    #[test]
    fn parse_failures_and_refusals_are_named() {
        let stray = SealedReview::new(b"--- a/x\n+++ b/x\n+stray\n", NONE, "v").unwrap_err();
        assert!(matches!(stray, ReviewError::Parse(_)), "{stray}");
        assert!(stray.to_string().starts_with("patch parse failed: "));
        let guards = InputGuards {
            files: true,
            hunks: false,
        };
        let empty = SealedReview::new(b"", guards, "v").unwrap_err();
        assert_eq!(
            empty,
            ReviewError::Refused("0 files in input (--require-files)")
        );
        assert_eq!(
            empty.to_string(),
            "refused: 0 files in input (--require-files)"
        );
    }

    #[test]
    fn non_utf8_input_is_reviewed_lossily_and_sealed_exactly() {
        let latin = b"diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-caf\xe9\n+cafe\n";
        let review = SealedReview::new(latin, NONE, "v").unwrap();
        assert_eq!(review.files()[0].patch_twin.lossy_lines, 1);
        assert_eq!(review.seal(), &InputSeal::of(latin, "v"));
    }

    /// The lossy line itself is marked, so a consumer knows which text is not the input's;
    /// before, only the per-file count said so. A literal U+FFFD in valid input is not lossy, and
    /// a lossy header line marks no body line.
    #[test]
    fn the_lossily_decoded_line_is_the_one_marked() {
        // Line 5 is Latin-1 (lossy); lines 6 and 7 hold a real U+FFFD (valid UTF-8).
        let bytes = b"diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n-caf\xe9\n-\xef\xbf\xbd\n+\xef\xbf\xbd\n+cafe\n";
        let review = SealedReview::new(bytes, NONE, "v").unwrap();
        let lines = &review.files()[0].patch_twin.hunks[0].lines;
        let marked: Vec<bool> = lines.iter().map(|l| l.lossy).collect();
        assert_eq!(marked, [true, false, false, false]);
        let header =
            b"diff --git a/caf\xe9 b/caf\xe9\n--- a/caf\xe9\n+++ b/caf\xe9\n@@ -1 +1 @@\n-a\n+b\n";
        let review = SealedReview::new(header, NONE, "v").unwrap();
        assert!(
            review.files()[0].patch_twin.hunks[0]
                .lines
                .iter()
                .all(|l| !l.lossy)
        );
        assert!(review.files()[0].patch_twin.lossy_lines > 0);
    }
}
