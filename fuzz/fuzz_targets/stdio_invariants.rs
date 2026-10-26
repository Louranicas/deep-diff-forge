// Shared output invariants for the stdio fuzz targets (`stdio_mcp`, `stdio_rpc`).
// Design/status: docs/API_AND_IPC.md (stdio bindings).

/// Every output line is one JSON-RPC 2.0 response object: an `id`, and exactly one of `result`
/// or `error`, the error carrying an integer `code`. Returns the number of responses.
pub fn check_responses(output: &[u8]) -> usize {
    let text = std::str::from_utf8(output).expect("stdio output must be UTF-8");
    if text.is_empty() {
        return 0;
    }
    assert!(
        text.ends_with('\n'),
        "every response must end with a newline"
    );
    let mut count = 0;
    for line in text[..text.len() - 1].split('\n') {
        let value: serde_json::Value =
            serde_json::from_str(line).expect("each stdio line must be JSON");
        let object = value
            .as_object()
            .expect("each response must be a JSON object");
        assert_eq!(object.get("jsonrpc").and_then(|v| v.as_str()), Some("2.0"));
        assert!(object.contains_key("id"), "a response must carry an id");
        assert_ne!(
            object.contains_key("result"),
            object.contains_key("error"),
            "a response carries exactly one of result and error"
        );
        if let Some(error) = object.get("error") {
            assert!(error.get("code").is_some_and(serde_json::Value::is_i64));
            assert!(
                error
                    .get("message")
                    .is_some_and(serde_json::Value::is_string)
            );
        }
        count += 1;
    }
    count
}

/// The cap for one run: the first input byte picks a small cap that exercises the oversize path,
/// or (for 255) the server's real cap.
pub fn cap_and_input(data: &[u8], real_cap: usize) -> Option<(usize, &[u8])> {
    let (&first, rest) = data.split_first()?;
    let cap = if first == u8::MAX {
        real_cap
    } else {
        1 + usize::from(first) * 16
    };
    Some((cap, rest))
}

/// Input lines, counting a trailing unterminated line: no server may answer more than this.
pub fn line_count(input: &[u8]) -> usize {
    input
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .count()
}
