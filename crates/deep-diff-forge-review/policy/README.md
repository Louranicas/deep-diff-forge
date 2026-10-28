# Pinned review policy assets

**Status 2026-10-28 (0.3.0):** current reference. The three pstack files and `antipatterns.json` (50 entries) are compiled into the binary with `include_str!` and written into every captured context (`src/storage.rs`). The two Python checkers, `review_policy_check.py` and `exemption_oracle.py`, are also embedded (`src/execution.rs`) and run inside the gate sandbox. `obsidian://` links below open the owner's local vault.

These assets are consumed by the review crate into every captured manifest. Pstack text remains verbatim with Lauren Tan’s MIT license. The antipattern crosswalk retains all 50 advisory/unverified source labels.

[Architecture and module index](../../../docs/REVIEW_COUNCIL.md) ↔ [Vault council hub](obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FReview%20Council). [Source hashes and HDD provenance](../../../review-policy/provenance.json).

[Exact reference module](antipatterns.json) ↔ [Module vault note](obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-review--policy--antipatterns).
