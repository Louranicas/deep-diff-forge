#![no_main]
// DDF integration: R39, R40, R41, R42, R43, R44, R45, R46, R57, R58, R59. Make checks fail closed, preserve fuzz crash evidence, use the actual build output path and gate tag publication on the quality workflow.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#quality-gates
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Ffuzz--fuzz_targets--agent_annotation

use deep_diff_forge_agent::{AnnotationSource, grounding_of, sanitize_body, source_of};
use deep_diff_forge_core::{AgentAnnotation, AnnotationAnchor, AnnotationProvenance};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };
    let sanitized = sanitize_body(input);
    assert!(sanitized.chars().count() <= deep_diff_forge_agent::MAX_BODY_LEN);
    assert!(
        sanitized
            .chars()
            .all(|c| c == '\n' || c == '\t' || !c.is_control())
    );

    let annotation = AgentAnnotation {
        id: "fuzz".to_string(),
        anchor: AnnotationAnchor::File {
            path: "x.rs".to_string(),
        },
        body: sanitized,
        provenance: AnnotationProvenance {
            agent: input.to_string(),
            model: None,
            evidence: Vec::new(),
        },
        grounded: true,
    };
    let mut store = deep_diff_forge_agent::AnnotationStore::new();
    store.add_untrusted(annotation);
    let annotation = &store.list()[0];
    assert_eq!(source_of(annotation), AnnotationSource::Agent);
    assert_eq!(grounding_of(annotation).label(), "ungrounded");
});
