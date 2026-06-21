# Deep-Diff-Forge

Deep-Diff-Forge is a next-generation review engine for code changes. It combines:

- Hunk-style review-first interaction, multi-file sidebar flow, worktree awareness, and inline AI annotations.
- Lumen-style interactive viewer ergonomics, mouse support, runtime toggles, and reviewer-native navigation.
- Difftastic-style structural understanding through syntax trees.
- Delta/diff-so-fancy/diff compatibility for pager workflows and terminal adoption.
- Classic diff interoperability for patch generation, patch application, and toolchain trust.

The first principle is simple: a diff engine should preserve patch truth while exposing semantic intent.

## Baseline Commitment

Deep-Diff-Forge must provide every capability in the comparison matrix:

| Capability | Baseline |
| --- | --- |
| Review-first interactive UI | Required |
| Multi-file review stream + sidebar | Required |
| Inline agent / AI annotations | Required |
| Responsive auto split/stack layout | Required |
| Mouse support inside the viewer | Required |
| Runtime view toggles | Required |
| Syntax highlighting | Required |
| Structural diffing | Required |
| Pager-compatible mode | Required |

## Three Pioneer Features

1. **Semantic Patch Twin**
   Every change has two synchronized representations: an apply-able patch twin and a semantic syntax twin. Reviewers can switch views without losing line anchors, comments, approvals, or patch applicability.

2. **Review Intelligence Graph**
   The engine builds a typed graph of files, hunks, symbols, tests, ownership, risk, and agent notes. It ranks the review stream by risk and dependency impact rather than raw file order.

3. **Adaptive Diff Planner**
   The engine dynamically chooses the best diff strategy per file and region: line, word, syntax, moved-block, generated-file suppression, or binary metadata. It exposes why a strategy was selected and falls back conservatively.

## Project Shape

- `crates/deep-diff-forge-core`: model, diff planning, patch/semantic twins, and review graph primitives.
- `crates/deep-diff-forge-cli`: pager-compatible CLI and early smoke surface.
- `docs/`: vision, architecture, baseline matrix, roadmap, and design constraints.

## Deployment Spine

- `docs/CLAUDE_CODE_BASH_CLI.md`: Claude Code, Bash, and CLI-first command contracts.
- `docs/FEATURE_DEPLOYABILITY_MATRIX.md`: deployability gates for every baseline and pioneer feature.
- `docs/PIONEER_FEATURE_SPECS.md`: implementation specs for semantic twins, review graph, and adaptive planner.
- `docs/BASH_API_CONTRACTS.md`: strict shell, JSON, JSONL, and exit-code contracts.
- `docs/END_TO_END_DEPLOYMENT.md`: gates from local build to release receipts.
- `docs/OPERATIONS_AND_DAEMON.md`: optional UDS daemon, health, systemd-user posture, and observability.
- `docs/LEARNING_LOOP.md`: feedback loop for planner, ranking, annotations, and corpus regression.
- `docs/STORAGE_AND_10TB_CORPUS.md`: policy for using `/mnt/storage-10tb` as optional corpus/archive storage.
- `docs/RELEASE_AND_PUBLICATION.md`: GitHub/GitLab/crates/binary release plan.

## Status

This repo is in vision and architecture bootstrap. The initial code intentionally defines stable vocabulary before algorithmic work begins.
