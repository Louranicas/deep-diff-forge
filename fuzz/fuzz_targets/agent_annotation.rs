#![no_main]

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
    assert!(sanitized.len() <= deep_diff_forge_agent::MAX_BODY_LEN);
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
    assert_eq!(source_of(&annotation), AnnotationSource::Agent);
    assert_eq!(grounding_of(&annotation).label(), "ungrounded");
});
