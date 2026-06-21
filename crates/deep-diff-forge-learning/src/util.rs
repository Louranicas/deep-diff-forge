//! Small dependency-free helpers shared across the learning loop.
//!
//! The headline helper is [`redacted_id`]: the learning loop's privacy contract
//! ("prefer hashes, counts, timings, and local-only receipts") is enforced by
//! never storing a path or source line in the first place. Callers hash the
//! path/source identity into a stable, non-reversible token and store *that*.

/// 64-bit FNV-1a offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// 64-bit FNV-1a prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a hash of `bytes` (deterministic, no allocation, no external deps).
///
/// FNV-1a is not cryptographic — it is a fast, stable, well-distributed hash.
/// That is exactly the right tool here: receipts need a *stable* id for a file
/// across runs, and a *non-reversible* one so the stored learning data cannot
/// reconstruct a path. It is not used for any security decision.
#[must_use]
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// A stable, non-reversible identity token for `input`, rendered as a 16-char
/// lowercase hex string. Used as `StrategyReceipt::file_hash` so receipts never
/// carry a path.
#[must_use]
pub fn redacted_id(input: &str) -> String {
    format!("{:016x}", fnv1a(input.as_bytes()))
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
        let id = redacted_id("crates/deep-diff-forge-core/src/lib.rs");
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
        let id = redacted_id(path);
        assert!(!id.contains("secret"));
        assert!(!id.contains('/'));
    }

    #[test]
    fn redacted_id_distinguishes_paths() {
        assert_ne!(redacted_id("a.rs"), redacted_id("b.rs"));
    }

    #[test]
    fn redacted_id_handles_empty() {
        let id = redacted_id("");
        assert_eq!(id.len(), 16);
    }

    #[test]
    fn redacted_id_handles_unicode() {
        let id = redacted_id("crates/café/源.rs");
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
