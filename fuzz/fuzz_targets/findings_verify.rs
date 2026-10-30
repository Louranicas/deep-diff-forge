#![no_main]
// Verified agent findings (plan 2026-10-30, Feature B): for any patch and any findings bytes,
// verification never panics, every non-blank line gets exactly one verdict (accepted, refused,
// or counted over the limit), every accepted anchor resolves to a hunk of its own file and a
// line of that hunk, and no accepted body keeps a control, bidi or invisible character.
//
// Input: `patch NUL findings`; without a NUL the bytes are findings against a fixed patch.

use deep_diff_forge_agent::findings::{document, verify};
use libfuzzer_sys::fuzz_target;

const PATCH: &str = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ fn main()\n fn main() {\n-    let a = 1;\n+    let a = 2;\n }\n--- a/README.md\n+++ b/README.md\n@@ -1 +1 @@\n-old\n+new\n";

fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }
    let text = String::from_utf8_lossy(data);
    let (patch, findings) = text.split_once('\0').unwrap_or((PATCH, text.as_ref()));
    let Ok(files) = deep_diff_forge_patch::parse(patch) else {
        return;
    };
    let lines = findings.lines().filter(|l| !l.trim().is_empty()).count();
    let verdict = verify(&files, "seal", findings.lines());
    assert_eq!(
        verdict.accepted.len() + verdict.refused.len() + verdict.over_limit,
        lines,
        "every non-blank line gets one verdict"
    );
    for accepted in &verdict.accepted {
        let file = files
            .iter()
            .find(|f| f.path == accepted.path)
            .expect("an accepted finding names a file of the review");
        if let Some(address) = &accepted.address {
            let hunk = file
                .patch_twin
                .hunks
                .iter()
                .find(|h| &h.address(&file.path) == address)
                .expect("an accepted address names a hunk of its file");
            if let Some(line) = &accepted.line {
                assert!(hunk.lines.iter().any(|l| l.new_line == Some(line.number)
                    || l.old_line == Some(line.number)));
            }
        } else {
            assert!(accepted.line.is_none(), "a file-level finding has no line");
        }
        for text in [&accepted.title, &accepted.body, &accepted.agent] {
            assert!(
                !text.chars().any(deep_diff_forge_core::is_unsafe_in_untrusted_text),
                "an unsafe character survived sanitising"
            );
        }
    }
    let doc = document(&verdict, "seal", "f", "0");
    assert_eq!(doc["claim_verified"], false);
    let _ = doc.to_string();
});
