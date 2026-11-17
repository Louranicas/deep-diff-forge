#![no_main]
// DDF 0.9.0, task S2.5: the JSON a client writes on the standard input of `agent hook <event>`,
// read by the CLI's own `hook` module (compiled here from its source, as the stdio targets do).
// Design/status: docs/TESTING_GOLD_STANDARD.md (fuzz targets).
//
// The property: no panic and no hang on any bytes, and an input the hook cannot read is refused
// with the hooks' own refusal (`did_not_run_output`), which says that nothing was judged and,
// for a stop, UNJUDGED and "not a pass". An input is read only when it is a JSON object that
// names the event and a session and a directory, which this target checks on its own.
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

#[expect(
    dead_code,
    reason = "the fuzz target drives only the reading of the hook input and the refusal"
)]
#[path = "../../crates/deep-diff-forge-cli/src/hook.rs"]
mod hook;
#[expect(
    dead_code,
    reason = "the hook module's verdict check, compiled beside it and not driven here"
)]
#[path = "../../crates/deep-diff-forge-cli/src/verdict_v2.rs"]
mod verdict_v2;

use hook::Event;

/// What `agent hook` prints when it did not run must be its refusal, never a decision.
fn assert_refusal(event: Event, reason: &str) {
    let out = hook::did_not_run_output(Some(event), reason);
    let value: Value = serde_json::from_str(&out).expect("a refusal is JSON");
    let object = value.as_object().expect("a refusal is an object");
    assert_eq!(
        object.keys().collect::<Vec<_>>(),
        ["systemMessage"],
        "a refusal is a message for the user and nothing else: {out}"
    );
    let message = object["systemMessage"].as_str().expect("the message is text");
    assert!(message.contains("Nothing was judged"), "{message}");
    if event == Event::Stop {
        assert!(
            message.contains("UNJUDGED") && message.contains("not a pass"),
            "{message}"
        );
    }
}

/// Whether `text` is a hook input the hook may read for `event`, checked here without the
/// hook module: a JSON object naming the event, a session and a directory.
fn readable(event: Event, text: &str) -> Option<(String, String, bool)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let object = value.as_object()?;
    let text_of = |name: &str| object.get(name).and_then(Value::as_str);
    if text_of("hook_event_name")? != event.claude_name() {
        return None;
    }
    let (session, cwd) = (text_of("session_id")?, text_of("cwd")?);
    if session.is_empty() || cwd.is_empty() {
        return None;
    }
    let active = object
        .get("stop_hook_active")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some((session.to_owned(), cwd.to_owned(), active))
}

fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }
    for event in [Event::SessionStart, Event::PostTool, Event::Stop] {
        // The hook reads its input as UTF-8 text; bytes that are not are refused there.
        let Ok(text) = std::str::from_utf8(data) else {
            assert_refusal(
                event,
                "its input could not be read: stream did not contain valid UTF-8",
            );
            continue;
        };
        match (hook::parse_input(event, text), readable(event, text)) {
            (Ok(input), Some((session, cwd, active))) => {
                assert_eq!(input.session, session);
                assert_eq!(input.cwd, cwd);
                assert_eq!(input.stop_hook_active, active);
            }
            (Err(reason), None) => {
                assert_ne!(reason, "");
                assert_refusal(event, &reason);
            }
            (Ok(input), None) => panic!("an input that names no {event:?} was read: {input:?}"),
            (Err(reason), Some(_)) => panic!("a whole {event:?} input was refused: {reason}"),
        }
    }
});
