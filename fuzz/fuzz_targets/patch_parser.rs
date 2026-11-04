#![no_main]
// DDF integration: R39, R40, R41, R42, R43, R44, R45, R46, R57, R58, R59. Make checks fail closed, preserve fuzz crash evidence, use the actual build output path and gate tag publication on the quality workflow.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#quality-gates
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Ffuzz--fuzz_targets--patch_parser

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }
    let options = deep_diff_forge_patch::ParseOptions {
        byte_budget: 256 * 1024,
    };
    // Raw bytes, as the CLI and daemon read them: a non-UTF-8 line takes the lossy decode path,
    // which a `from_utf8` filter never reached (production readiness 2026-11-04).
    let decoded = deep_diff_forge_patch::decode(data);
    if let Ok(files) = deep_diff_forge_patch::parse_decoded(&decoded, options) {
        // Every accepted hunk changes something: Git refuses one that does not.
        for file in &files {
            for hunk in &file.patch_twin.hunks {
                assert!(
                    hunk.lines
                        .iter()
                        .any(|l| l.kind != deep_diff_forge_core::PatchLineKind::Context),
                    "an accepted hunk changes nothing"
                );
            }
        }
        let rendered = deep_diff_forge_patch::render_unified(&files);
        let reparsed = deep_diff_forge_patch::parse(&rendered)
            .expect("rendered model must remain parseable");
        // Canonical rendering must stabilize even for tolerated cosmetic input.
        // Application equivalence is checked independently by the Git oracle suites
        // (`tests/git_oracle.rs`, and `tests/git_differential.rs` over every seed).
        assert_eq!(rendered, deep_diff_forge_patch::render_unified(&reparsed));
    }
});
