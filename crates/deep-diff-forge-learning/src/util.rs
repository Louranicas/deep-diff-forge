//! Small dependency-free helpers shared across the learning loop.
//!
//! The headline helper is [`redacted_id`]: the learning loop's privacy contract
//! ("prefer hashes, counts, timings, and local-only receipts") is enforced by
//! never storing a path or source line in the first place. Callers hash the
//! path/source identity into a stable, pseudonymous token and store *that*.

/// 64-bit FNV-1a offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// 64-bit FNV-1a prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a hash of `bytes` (deterministic, no allocation, no external deps).
///
/// FNV-1a is not cryptographic — it is a fast, stable, well-distributed hash.
/// It is used only to correlate local receipts and never for a security
/// decision. It is not one-way: an attacker who obtains a token can cheaply
/// test likely paths from a dictionary. Confidentiality therefore comes from
/// the owner-only local store, not from this hash. Callers needing one-way
/// pseudonyms must use a keyed construction such as HMAC-SHA-256 instead.
#[must_use]
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// A redacted file identity: 16 to 64 lowercase hexadecimal characters, never a path. Only
/// [`redacted_id`] makes one from a path, and deserializing one checks its form, so a
/// [`StrategyReceipt`](crate::StrategyReceipt) cannot carry a path at all (anchor C-07). On the
/// wire it is the bare string.
///
/// ```
/// let id = deep_diff_forge_learning::redacted_id("src/lib.rs");
/// assert_eq!(id.as_str().len(), 16);
/// ```
///
/// ```compile_fail
/// let _ = deep_diff_forge_learning::RedactedId(String::from("src/lib.rs"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RedactedId(String);

impl RedactedId {
    /// The hexadecimal token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RedactedId {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let valid = (16..=64).contains(&value.len())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if valid {
            Ok(Self(value))
        } else {
            Err("file_hash must be a 16-64 character lowercase hexadecimal redacted id")
        }
    }
}

impl From<RedactedId> for String {
    fn from(id: RedactedId) -> Self {
        id.0
    }
}

impl std::fmt::Display for RedactedId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A stable, pseudonymous identity token for `input`, rendered as a 16-character
/// lowercase hex string. Used as `StrategyReceipt::file_hash` so receipts never
/// carry a path.
/// See [`fnv1a`] for its dictionary-guessing and collision caveats.
#[must_use]
pub fn redacted_id(input: &str) -> RedactedId {
    RedactedId(format!("{:016x}", fnv1a(input.as_bytes())))
}

/// Clamp `value` into the inclusive range `[lo, hi]`.
///
/// Used by the learners to keep derived weights and scores bounded regardless of
/// pathological input distributions.
#[must_use]
pub fn clamp_f64(value: f64, lo: f64, hi: f64) -> f64 {
    if value < lo {
        lo
    } else if value > hi {
        hi
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_empty_is_offset_basis() {
        assert_eq!(fnv1a(b""), FNV_OFFSET);
    }

    #[test]
    fn fnv1a_is_deterministic() {
        assert_eq!(fnv1a(b"src/lib.rs"), fnv1a(b"src/lib.rs"));
    }

    #[test]
    fn fnv1a_distinguishes_inputs() {
        assert_ne!(fnv1a(b"src/lib.rs"), fnv1a(b"src/main.rs"));
    }

    #[test]
    fn fnv1a_single_byte_difference_changes_hash() {
        assert_ne!(fnv1a(b"a"), fnv1a(b"b"));
    }

    #[test]
    fn fnv1a_order_sensitive() {
        assert_ne!(fnv1a(b"ab"), fnv1a(b"ba"));
    }

    #[test]
    fn redacted_id_is_sixteen_hex_chars() {
        let id = redacted_id("crates/deep-diff-forge-core/src/lib.rs").to_string();
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(id.chars().all(|c| !c.is_ascii_uppercase()));
    }

    #[test]
    fn redacted_id_is_stable_across_calls() {
        assert_eq!(redacted_id("a/b/c.rs"), redacted_id("a/b/c.rs"));
    }

    #[test]
    fn redacted_id_does_not_contain_the_path() {
        let path = "secret/internal/path.rs";
        let id = redacted_id(path).to_string();
        assert!(!id.contains("secret"));
        assert!(!id.contains('/'));
    }

    #[test]
    fn redacted_id_distinguishes_paths() {
        assert_ne!(redacted_id("a.rs"), redacted_id("b.rs"));
    }

    #[test]
    fn redacted_id_handles_empty() {
        let id = redacted_id("").to_string();
        assert_eq!(id.len(), 16);
    }

    #[test]
    fn redacted_id_handles_unicode() {
        let id = redacted_id("crates/café/源.rs").to_string();
        assert_eq!(id.len(), 16);
    }

    #[test]
    fn clamp_below_low() {
        assert!((clamp_f64(-1.0, 0.0, 1.0) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn clamp_above_high() {
        assert!((clamp_f64(2.0, 0.0, 1.0) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn clamp_within_range_is_identity() {
        assert!((clamp_f64(0.5, 0.0, 1.0) - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn clamp_at_bounds_is_identity() {
        assert!((clamp_f64(0.0, 0.0, 1.0) - 0.0).abs() < f64::EPSILON);
        assert!((clamp_f64(1.0, 0.0, 1.0) - 1.0).abs() < f64::EPSILON);
    }
}
