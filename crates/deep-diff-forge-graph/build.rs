//! The scorer's identity (S3.15.b): `tool.scorer_sha256` of `rank.v0` is the SHA-256 of every
//! file under `src/` except `lib.rs`: the rule tables, the secret prefixes and their byte
//! classes, the readers (`rust_tokens.rs`, `lexical.rs`), the path rules and weights (`risk.rs`),
//! the order (`rank.rs`) and the vocabulary. It moves on ANY edit of those files, a comment or a
//! test included, so two scorers never share one identity; `lib.rs` holds the re-exports and the
//! document renderer, which decide no label. The input is the files in byte order of their
//! `/`-separated path under `src/`, each as `path NUL length NUL bytes` with CRLF read as LF: no
//! time, no absolute path, no host, so two builds of one tree give one digest.

use std::path::Path;

#[expect(
    clippy::disallowed_methods,
    reason = "build script: Cargo passes OUT_DIR through the environment"
)]
fn main() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    println!("cargo:rerun-if-changed=src");
    let mut files = Vec::new();
    walk(&src, "", &mut files);
    files.sort();
    let mut input = Vec::new();
    for name in files.iter().filter(|name| *name != "lib.rs") {
        let bytes = std::fs::read(src.join(name)).expect("read a scorer source");
        let text = String::from_utf8(bytes).expect("a scorer source is UTF-8");
        let text = text.replace("\r\n", "\n");
        input.extend_from_slice(format!("{name}\0{}\0", text.len()).as_bytes());
        input.extend_from_slice(text.as_bytes());
    }
    let out = Path::new(&std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("scorer_sha256");
    std::fs::write(out, hex(&sha256(&input))).expect("write the scorer digest");
}

fn walk(dir: &Path, prefix: &str, files: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("list a scorer directory") {
        let entry = entry.expect("read a scorer directory entry");
        let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
        if entry.file_type().expect("a file type").is_dir() {
            walk(&entry.path(), &format!("{name}/"), files);
        } else if Path::new(&name).extension().is_some_and(|ext| ext == "rs") {
            files.push(name);
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(2 * bytes.len());
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// SHA-256's round constants and initial state (FIPS 180-4, 4.2.2 and 5.3.3).
#[rustfmt::skip]
const K: [u32; 64] = [
    0x428a_2f98, 0x7137_4491, 0xb5c0_fbcf, 0xe9b5_dba5, 0x3956_c25b, 0x59f1_11f1, 0x923f_82a4, 0xab1c_5ed5,
    0xd807_aa98, 0x1283_5b01, 0x2431_85be, 0x550c_7dc3, 0x72be_5d74, 0x80de_b1fe, 0x9bdc_06a7, 0xc19b_f174,
    0xe49b_69c1, 0xefbe_4786, 0x0fc1_9dc6, 0x240c_a1cc, 0x2de9_2c6f, 0x4a74_84aa, 0x5cb0_a9dc, 0x76f9_88da,
    0x983e_5152, 0xa831_c66d, 0xb003_27c8, 0xbf59_7fc7, 0xc6e0_0bf3, 0xd5a7_9147, 0x06ca_6351, 0x1429_2967,
    0x27b7_0a85, 0x2e1b_2138, 0x4d2c_6dfc, 0x5338_0d13, 0x650a_7354, 0x766a_0abb, 0x81c2_c92e, 0x9272_2c85,
    0xa2bf_e8a1, 0xa81a_664b, 0xc24b_8b70, 0xc76c_51a3, 0xd192_e819, 0xd699_0624, 0xf40e_3585, 0x106a_a070,
    0x19a4_c116, 0x1e37_6c08, 0x2748_774c, 0x34b0_bcb5, 0x391c_0cb3, 0x4ed8_aa4a, 0x5b9c_ca4f, 0x682e_6ff3,
    0x748f_82ee, 0x78a5_636f, 0x84c8_7814, 0x8cc7_0208, 0x90be_fffa, 0xa450_6ceb, 0xbef9_a3f7, 0xc671_78f2,
];
#[rustfmt::skip]
const H0: [u32; 8] = [
    0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a, 0x510e_527f, 0x9b05_688c, 0x1f83_d9ab, 0x5be0_cd19,
];

/// FIPS 180-4 SHA-256 (the build script has no dependencies; a test of the crate checks it
/// against `deep_diff_forge_core::sha256_hex` over the same input).
fn sha256(message: &[u8]) -> [u8; 32] {
    let mut data = message.to_vec();
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&(message.len() as u64 * 8).to_be_bytes());
    let mut hash = H0;
    for block in data.as_chunks::<64>().0 {
        compress(&mut hash, block);
    }
    let mut digest = [0u8; 32];
    for (chunk, word) in digest.as_chunks_mut::<4>().0.iter_mut().zip(hash) {
        *chunk = word.to_be_bytes();
    }
    digest
}

fn compress(hash: &mut [u32; 8], block: &[u8; 64]) {
    let mut words = [0u32; 64];
    for (slot, word) in words.iter_mut().zip(block.as_chunks::<4>().0) {
        *slot = u32::from_be_bytes(*word);
    }
    for i in 16..64 {
        let (a, b) = (words[i - 15], words[i - 2]);
        let s0 = a.rotate_right(7) ^ a.rotate_right(18) ^ (a >> 3);
        let s1 = b.rotate_right(17) ^ b.rotate_right(19) ^ (b >> 10);
        words[i] = words[i - 16]
            .wrapping_add(s0)
            .wrapping_add(words[i - 7])
            .wrapping_add(s1);
    }
    let mut v = *hash;
    for (round, word) in K.iter().zip(words) {
        let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
        let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
        let t1 = v[7]
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(*round)
            .wrapping_add(word);
        let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
        let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
        let t2 = s0.wrapping_add(maj);
        v = [
            t1.wrapping_add(t2),
            v[0],
            v[1],
            v[2],
            v[3].wrapping_add(t1),
            v[4],
            v[5],
            v[6],
        ];
    }
    for (state, value) in hash.iter_mut().zip(v) {
        *state = state.wrapping_add(value);
    }
}
