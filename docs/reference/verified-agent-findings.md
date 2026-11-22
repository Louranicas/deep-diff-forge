# Verified agent findings

Agents report findings, and DDF checks each one against patch truth before anyone reads it. A
finding is one JSON object per line (`deep-diff-forge.finding.v0`):

```json
{"agent": "codex", "severity": "high", "title": "spawns curl", "body": "…", "address": "<hunk address>", "line": 2}
```

```bash
deep-diff-forge findings verify --findings found.jsonl < change.patch     # the sealed verdict
deep-diff-forge findings verify --findings found.jsonl --require-all < change.patch   # exit 7 if any is refused
deep-diff-forge review --findings found.jsonl < change.patch              # the accepted ones as notes
```

- **Anchors.** A finding is anchored by a hunk `address` (from the outline or `rank.v0`
  evidence), or by a `path` with an optional `line` and `side` (`new` or `old`). A path and a
  line resolve to the hunk that holds the line, in any section with the path (a concatenated
  patch can name one file twice), so a finding survives an agent that never saw addresses.
- **Accepted findings** come back normalised from the review: the path, the address, and the
  line's own kind and text. The title and body are sanitised: no control, bidi or invisible
  characters.
- **Refused findings** carry one reason: `malformed`, `stale_review` (the finding's `review` is
  not this patch's `input_sha256`), `unknown_path`, `unknown_address`, `line_outside_hunk`,
  `duplicate` (same agent, title and anchor) or `too_long`.
- **Limits.** At most 10,000 findings and 8 MiB per call; the rest are counted as `over_limit`.
- **Claims are not checked.** DDF checks *where* a finding points, never *what* it says.
  `claim_verified` is always `false`, every accepted finding says `source: "agent"`, and
  findings never enter `review.v0`. In the cockpit they are agent notes, never grounded, and an
  agent cannot pass for the engine whatever it calls itself.
- **The same verdict everywhere.** The daemon's `findings.verify` (protocol 2), `rpc`, MCP
  `verify_findings` and the Python SDK's `verify_findings` take the findings as an array and give
  the same document as the CLI. `findings_sha256` covers the CLI's file bytes, or for an array
  its JSONL (one compact object per line, keys sorted).

See also: [`PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY`](../PLAN_AGENT_EVIDENCE_AND_OBSERVABILITY.md).
