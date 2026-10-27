//! Anchor C-12: a non-UTF-8 line no longer refuses a whole review. It is decoded lossily, only its
//! own file is marked, and the seal covers the exact bytes.
use deep_diff_forge_patch::{CapturedPatch, ParseOptions, decode, parse_decoded};

const TWO_FILES: &[u8] = b"diff --git a/ok.rs b/ok.rs\n--- a/ok.rs\n+++ b/ok.rs\n@@ -1 +1 @@\n-a\n+b\n\
diff --git a/latin.txt b/latin.txt\n--- a/latin.txt\n+++ b/latin.txt\n@@ -1,2 +1,2 @@\n caf\xe9\n-na\xefve\n+naive\n";

#[test]
fn a_non_utf8_line_marks_only_its_own_file() {
    let decoded = decode(TWO_FILES);
    assert_eq!(decoded.lossy_lines(), &[11, 12]);
    let files = parse_decoded(&decoded, ParseOptions::default()).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(
        (files[0].path.as_str(), files[0].patch_twin.lossy_lines),
        ("ok.rs", 0)
    );
    assert_eq!(
        (files[1].path.as_str(), files[1].patch_twin.lossy_lines),
        ("latin.txt", 2)
    );
    let lines: Vec<&str> = files[1].patch_twin.hunks[0]
        .lines
        .iter()
        .map(|l| l.text.as_str())
        .collect();
    assert_eq!(lines, ["caf\u{fffd}", "na\u{fffd}ve", "naive"]);
}

#[test]
fn the_capture_seals_the_exact_bytes_not_the_decoded_text() {
    let captured = CapturedPatch::capture_bytes(TWO_FILES.to_vec()).unwrap();
    assert!(!captured.is_exact());
    assert_eq!(captured.original_bytes(), TWO_FILES);
    assert_eq!(
        captured.digest(),
        deep_diff_forge_core::sha256_hex(TWO_FILES)
    );
    assert_ne!(captured.original().as_bytes(), TWO_FILES);
    // Valid UTF-8 takes the exact path: nothing is marked and the text is the input.
    let exact =
        CapturedPatch::capture_bytes(b"--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n".to_vec()).unwrap();
    assert!(exact.is_exact());
    assert_eq!(exact.original().as_bytes(), exact.original_bytes());
    assert_eq!(exact.files()[0].patch_twin.lossy_lines, 0);
}

#[test]
fn the_byte_budget_applies_to_the_original_bytes() {
    // Each invalid byte decodes to a 3-byte U+FFFD, so the text outgrows the bytes.
    let mut bytes = b"--- a/x\n+++ b/x\n@@ -1 +1 @@\n-".to_vec();
    bytes.extend(std::iter::repeat_n(0xff, 40));
    bytes.extend_from_slice(b"\n+b\n");
    let decoded = decode(&bytes);
    assert!(decoded.text().len() > bytes.len());
    let at_budget = ParseOptions {
        byte_budget: bytes.len(),
    };
    assert!(parse_decoded(&decoded, at_budget).is_ok());
    let below = ParseOptions {
        byte_budget: bytes.len() - 1,
    };
    assert!(parse_decoded(&decoded, below).is_err());
}
