#![no_main]
// Review council: Bounded untrusted report parsing, validation and accepted-output serialization stability.
// Design/status: docs/REVIEW_COUNCIL.md#report-fuzz
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--fuzz--fuzz_targets--council_report
use libfuzzer_sys::fuzz_target;
use deep_diff_forge_review::{Manifest,Report,validate_report};
fuzz_target!(|data:&[u8]| {
    if data.len()>64*1024{return;}
    if let Ok(report)=serde_json::from_slice::<Report>(data) {
        let request=deep_diff_forge_review::cli::example();
        let manifest=Manifest{schema:"deep-diff-forge.council.v1".into(),run_id:"fuzz".into(),review_id:"test".into(),source_tree:"test".into(),request,artifacts:vec![deep_diff_forge_review::Artifact{id:"source".into(),label:"source.rs".into(),sha256:"fuzz".into(),text:"one\ntwo\n".into()}],mutation_diff_sha256:None};
        if validate_report(&manifest,&report).is_ok() {
            let bytes=serde_json::to_vec(&report).unwrap();
            let again:Report=serde_json::from_slice(&bytes).unwrap();
            assert!(validate_report(&manifest,&again).is_ok());
        }
    }
});
