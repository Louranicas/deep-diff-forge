#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };
    let options = deep_diff_forge_patch::ParseOptions {
        byte_budget: 256 * 1024,
    };
    if let Ok(files) = deep_diff_forge_patch::parse_with(input, options) {
        let json = deep_diff_forge_patch::to_json(&files);
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .expect("review JSON emitted by deep-diff-forge-patch must stay valid");
        assert_eq!(
            parsed.get("schema").and_then(serde_json::Value::as_str),
            Some("deep-diff-forge.review.v0")
        );
    }
});
