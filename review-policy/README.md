# Review policy integration

**Status 2026-10-28 (0.3.0):** current reference. This directory holds the HDD provenance record (`provenance.json`), the fuzz seed for `council_report` (`fuzz-seeds/council_report.json`, used by the CI fuzz smoke) and the gate image definition (`sandbox/Containerfile`). The `source` paths in `provenance.json` are paths on the operator workstation and are not in the repository; the destination files and their SHA-256 hashes are. `obsidian://` links below open the owner's local vault.

[Review council architecture and CLI](../docs/REVIEW_COUNCIL.md) ↔ [Vault council hub](obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FReview%20Council). Canonical imported policy assets live in [the review crate](../crates/deep-diff-forge-review/policy/README.md) and enter every captured context. This directory owns the HDD provenance record and scoped fuzz seeds.

The parsed policy guard qualifies workspace lint inheritance and reference integrity. It does not claim to detect or eliminate all 50 imported semantic antipatterns. Each new detector needs an applicable source row, actual reader, trigger, budget and a negative control that fails for the intended reason.

[Exact reference module](provenance.json) ↔ [Module vault note](obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--review-policy--provenance).
