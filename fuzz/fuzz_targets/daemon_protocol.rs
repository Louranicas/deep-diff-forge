#![no_main]

use deep_diff_forge_daemon::{Engine, process_line};
use libfuzzer_sys::fuzz_target;
use std::sync::Mutex;

fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }
    let Ok(line) = std::str::from_utf8(data) else {
        return;
    };
    let engine = Mutex::new(Engine::new());
    let response = process_line(line.trim(), &engine);
    let parsed: serde_json::Value = serde_json::from_str(&response)
        .expect("daemon protocol responses must always be valid JSON-RPC JSON");
    assert_eq!(
        parsed.get("jsonrpc").and_then(serde_json::Value::as_str),
        Some("2.0")
    );
});
