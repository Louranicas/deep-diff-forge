#![no_main]
// DDF 0.11.1: the Request JSON that `council prepare` reads, through the review crate's production
// model and admission policy. Design/status: docs/REVIEW_COUNCIL.md#report-fuzz.
//
// The property, for any byte string under the production request cap: no panic while parsing or
// validating; serialization cannot change whether a parsed request passes admission.
use deep_diff_forge_review::{Request, validate_request};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 128 * 1024 {
        return;
    }
    let Ok(request) = serde_json::from_slice::<Request>(data) else {
        return;
    };
    let accepted = validate_request(&request).is_ok();
    let encoded = serde_json::to_vec(&request).expect("a parsed council request serializes");
    let again = serde_json::from_slice(&encoded).expect("the serialized request parses");
    assert_eq!(validate_request(&again).is_ok(), accepted);
});
