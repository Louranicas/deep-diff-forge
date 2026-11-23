#![no_main]
// DDF 0.10.0, task S3.32.a: a judge's verdict document (`forge-warden.verdict.v2` and `.v3`), as
// the stop hook reads it, through the CLI's own `hook` and `verdict_v2` modules (compiled here
// from their source, as `hook_input` does). Seeds: forge-warden's v2 and v3 fixtures of the CLI's
// tests. Design/status: docs/TESTING_GOLD_STANDARD.md (fuzz targets).
//
// The property, for any byte string up to 256 KiB: no panic and no hang in the duplicate-key
// walk, the reading of either version or the judging of exits 0, 7 and 8 (sealed to the seals
// the document itself names, so a sealed document reaches every check); a document is whole for
// at most the version it names; a pass, hold or fail is given only on a whole document that says
// that outcome; and exit 7, the judge's fail, is never taken as a pass or a hold.
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

#[expect(
    dead_code,
    reason = "the fuzz target drives only the reading of a verdict"
)]
#[path = "../../crates/deep-diff-forge-cli/src/hook.rs"]
mod hook;
#[path = "../../crates/deep-diff-forge-cli/src/verdict_v2.rs"]
mod verdict_v2;

use hook::{Judged, Sealed};
use verdict_v2::{Version, defect_in};

fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let _ = hook::has_duplicate_key(text);
    let Ok(document) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let named = Version::named_by(&document);
    for version in [Version::V2, Version::V3] {
        if defect_in(&document, version).is_none() {
            assert_eq!(named, Some(version), "whole for a version it does not name");
        }
    }
    let seal = |member: &str| document["seals"][member].as_str().unwrap_or_default();
    let sealed = Sealed {
        policy: seal("policy_sha256"),
        patch: seal("patch_sha256"),
        ranker: document["seals"]["ranked_by"]["sha256"]
            .as_str()
            .unwrap_or_default(),
    };
    for (code, outcome) in [(0, "pass"), (7, "fail"), (8, "hold")] {
        let judged = hook::judged(Some(code), Some(&document), None, sealed);
        if !matches!(judged, Judged::Unjudged { .. }) {
            let version = named.expect("a decision on a document of no version this build reads");
            assert_eq!(defect_in(&document, version), None, "a decision on a part");
            assert_eq!(document["verdict"], outcome, "{judged:?} on exit {code}");
        }
        let stop = hook::stop_judged(Some(code), Some(&document), None, sealed);
        assert!(
            code != 7 || matches!(stop, Judged::Fail { .. }),
            "{stop:?} on exit 7"
        );
    }
});
