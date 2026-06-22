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
        let rendered = deep_diff_forge_patch::render_unified(&files);
        let _ = deep_diff_forge_patch::parse_with(&rendered, options);
    }
});
