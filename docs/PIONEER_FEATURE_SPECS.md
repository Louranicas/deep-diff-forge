# Pioneer Feature Specifications

This document converts the three above-baseline features into implementation-ready specs.

## 1. Semantic Patch Twin

### Purpose

Preserve apply-able patch truth while adding syntax-level understanding.

### Inputs

- canonical patch twin
- old/new file bytes
- language guess
- parser budget
- tree-sitter parse output when supported

### Outputs

- `PatchTwin`: apply-able hunk structure
- `SemanticTwin`: syntax spans and parse status
- `AnchorMap`: file, hunk, line, byte, and syntax-node links

### CLI

```bash
deep-diff-forge --git --semantic --json
deep-diff-forge old.rs new.rs --semantic --layout split
deep-diff-forge explain-span <span-id> --json
```

### Failure Modes

| Failure | Behavior |
| --- | --- |
| Unsupported language | Keep patch twin, semantic fallback reason. |
| Parser errors | Use parse error budget, then fallback. |
| Node budget exceeded | Stop semantic analysis, keep patch twin. |
| Anchor mismatch | Mark semantic span invalid, never alter patch truth. |

### Tests

- reformat-only change keeps patch lines and marks semantic reformat
- moved function links old and new spans
- malformed syntax keeps patch twin
- unsupported language exports fallback reason

## 2. Review Intelligence Graph

### Purpose

Rank review work by likely impact, not file order.

### Nodes

- file
- hunk
- symbol
- test
- owner
- risk signal
- command evidence
- agent annotation
- reviewer decision

### Edges

- file contains hunk
- hunk touches symbol
- symbol is covered by test
- hunk has risk signal
- annotation claims about hunk
- command produced evidence
- reviewer decision resolves annotation

### CLI

```bash
deep-diff-forge rank --git
deep-diff-forge rank --git --json
deep-diff-forge graph --git --json
deep-diff-forge why-first <hunk-id>
```

### Ranking Signals

- public API edit
- control-flow edit
- dependency fan-out
- security-sensitive path
- generated/vendor suppression
- test distance
- ownership
- prior fallback
- ungrounded agent claim

### Tests

- public API hunk outranks comment-only edit
- generated file suppresses below source edit
- ungrounded agent claim raises review priority
- ranking explanation is deterministic

## 3. Adaptive Diff Planner

### Purpose

Choose the cheapest truthful diff strategy per file and region.

### Strategies

- binary metadata
- generated summary
- line diff
- word diff
- syntax diff
- region syntax diff
- moved-block diff

### CLI

```bash
deep-diff-forge plan --git
deep-diff-forge plan --git --json
deep-diff-forge --git --budget fast
deep-diff-forge --git --budget deep
```

### Budget Profiles

| Profile | Use |
| --- | --- |
| `fast` | Claude Code quick review and CI preflight. |
| `balanced` | Default human review. |
| `deep` | Expensive semantic review. |
| `forensic` | Offline corpus analysis and benchmark runs. |

### Fallback Requirements

Every fallback records:

- attempted strategy
- fallback strategy
- reason
- elapsed time
- byte count
- node count if available
- whether patch truth remained complete

### Tests

- large file selects line or summary instead of syntax
- binary file selects binary metadata
- small Rust file selects syntax
- parser failure falls back to line diff with reason
- `fast` and `deep` profiles produce explainable differences

## Deployment Link

- Framework: [Codebase Deployment Framework](DEPLOYMENT_FRAMEWORK.md)
