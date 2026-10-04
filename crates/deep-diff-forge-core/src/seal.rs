//! Input sealing for machine-readable output.
//!
//! An orchestrator that consumes a `--json` document as evidence needs to know
//! *which bytes* the document describes and *which tool* produced it, so a
//! verdict can be tied to an exact input rather than to "whatever was on stdin
//! at the time". [`InputSeal`] carries both: the SHA-256 of the raw input bytes
//! (hashed before any parsing or normalisation) and the emitting tool's name and
//! version. SHA-256 is implemented here in pure Rust (FIPS 180-4), no `unsafe`,
//! no external dependency — consistent with the workspace's zero-dependency
//! posture and the hand-rolled FNV-1a in the learning crate.

use std::fmt::Write as _;

/// SHA-256 round constants (FIPS 180-4 §4.2.2).
const K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

/// SHA-256 initial hash value (FIPS 180-4 §5.3.3).
const H0: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

/// One SHA-256 compression step over a 64-byte `block`.
fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for (slot, word) in w.iter_mut().zip(block.as_chunks::<4>().0) {
        *slot = u32::from_be_bytes(*word);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }

    // Working variables `v[0..8]` are FIPS 180-4's a, b, c, d, e, f, g, h.
    let mut v = *state;
    for (round_k, round_w) in K.iter().zip(w) {
        let big_s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
        let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
        let t1 = v[7]
            .wrapping_add(big_s1)
            .wrapping_add(ch)
            .wrapping_add(*round_k)
            .wrapping_add(round_w);
        let big_s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
        let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
        let t2 = big_s0.wrapping_add(maj);
        // Shift h←g, g←f, f←e, e←d, d←c, c←b, b←a, then set e = d + t1 and
        // a = t1 + t2.
        v.rotate_right(1);
        v[4] = v[4].wrapping_add(t1);
        v[0] = t1.wrapping_add(t2);
    }
    for (acc, val) in state.iter_mut().zip(v) {
        *acc = acc.wrapping_add(val);
    }
}

/// SHA-256 digest of `bytes` (FIPS 180-4). Pure, allocation-free apart from the
/// fixed padding buffer, and panic-free on any input length.
#[must_use]
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut state = H0;
    let (blocks, rem) = bytes.as_chunks::<64>();
    for block in blocks {
        compress(&mut state, block);
    }

    // Padding: 0x80, zeros to 56 mod 64, then the bit length as a big-endian
    // u64. The tail is at most two blocks.
    let mut tail = [0u8; 128];
    tail[..rem.len()].copy_from_slice(rem);
    tail[rem.len()] = 0x80;
    let tail_len = if rem.len() < 56 { 64 } else { 128 };
    // A slice can never hold more than 2^61 bytes, so the bit length fits.
    let bit_len = u64::try_from(bytes.len())
        .unwrap_or(u64::MAX)
        .wrapping_shl(3);
    tail[tail_len - 8..tail_len].copy_from_slice(&bit_len.to_be_bytes());
    for block in tail[..tail_len].as_chunks::<64>().0 {
        compress(&mut state, block);
    }

    let mut out = [0u8; 32];
    for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        *chunk = word.to_be_bytes();
    }
    out
}

/// Lowercase hex SHA-256 of `bytes` — the `input_sha256` wire form.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(64);
    for byte in sha256(bytes) {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Provenance seal attached to a machine-readable document: the SHA-256 of the
/// exact input bytes the document was computed from, plus the emitting tool.
///
/// The hash is taken over the raw bytes **before** parsing or normalisation, so
/// a consumer can verify `sha256sum < input` against `input_sha256` and know the
/// document describes precisely that input. Rendered as two additive top-level
/// JSON members (`input_sha256`, `tool`) via [`InputSeal::json_members`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSeal {
    /// Lowercase hex SHA-256 of the raw input bytes.
    pub input_sha256: String,
    /// Version of the emitting tool (its `CARGO_PKG_VERSION`).
    pub tool_version: String,
}

impl InputSeal {
    /// The `tool.name` every document carries.
    pub const TOOL_NAME: &'static str = "deep-diff-forge";

    /// Seal `input` as produced by tool version `tool_version`.
    #[must_use]
    pub fn of(input: &[u8], tool_version: &str) -> Self {
        Self {
            input_sha256: sha256_hex(input),
            tool_version: tool_version.to_string(),
        }
    }

    /// The seal as two top-level members of a 2-space-indented JSON object —
    /// each on its own line, comma-terminated — ready to splice in directly
    /// after the `"schema"` line:
    ///
    /// ```text
    ///   "input_sha256": "<64 hex>",
    ///   "tool": {"name": "deep-diff-forge", "version": "<version>"},
    /// ```
    #[must_use]
    pub fn json_members(&self) -> String {
        format!(
            "  \"input_sha256\": {},\n  \"tool\": {{\"name\": {}, \"version\": {}}},\n",
            crate::json_escape(&self.input_sha256),
            crate::json_escape(Self::TOOL_NAME),
            crate::json_escape(&self.tool_version)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reference digests: FIPS 180-4 / NIST CAVP vectors and `sha256sum`.
    #[test]
    fn sha256_empty_input_matches_nist() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_abc_matches_nist() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_two_block_message_matches_nist() {
        // 56 bytes: forces the padding into a second block.
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn sha256_exact_block_boundary_pads_into_new_block() {
        // 64 bytes: the whole padding block is appended.
        assert_eq!(
            sha256_hex(&[b'a'; 64]),
            "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"
        );
    }

    #[test]
    fn sha256_million_a_matches_nist() {
        let input = vec![b'a'; 1_000_000];
        assert_eq!(
            sha256_hex(&input),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn sha256_padding_length_55_and_63_edges() {
        // 55 bytes: 0x80 + length fit in one block; 63 bytes: they do not.
        assert_eq!(
            sha256_hex(&[b'a'; 55]),
            "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318"
        );
        assert_eq!(
            sha256_hex(&[b'a'; 63]),
            "7d3e74a05d7db15bce4ad9ec0658ea98e3f06eeecf16b4c6fff2da457ddc2f34"
        );
    }

    #[test]
    fn sha256_is_deterministic_and_input_sensitive() {
        let a = sha256_hex(b"--- a/x\n+++ b/x\n");
        assert_eq!(a, sha256_hex(b"--- a/x\n+++ b/x\n"));
        assert_ne!(a, sha256_hex(b"--- a/x\n+++ b/x"));
    }

    #[test]
    fn hex_is_64_lowercase_chars() {
        let hex = sha256_hex(b"anything");
        assert_eq!(hex.len(), 64);
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn seal_of_hashes_raw_bytes_and_records_version() {
        let seal = InputSeal::of(b"abc", "9.9.9");
        assert_eq!(
            seal.input_sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(seal.tool_version, "9.9.9");
    }

    #[test]
    fn json_members_render_two_comma_terminated_lines() {
        let seal = InputSeal::of(b"", "0.0.1");
        let members = seal.json_members();
        assert_eq!(
            members,
            "  \"input_sha256\": \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\",\n  \"tool\": {\"name\": \"deep-diff-forge\", \"version\": \"0.0.1\"},\n"
        );
    }

    #[test]
    fn json_members_escape_a_hostile_version_string() {
        let seal = InputSeal::of(b"", "1.0\"\u{1b}[2J");
        let members = seal.json_members();
        assert!(members.contains("\"version\": \"1.0\\\"\\u001b[2J\""));
    }
}
