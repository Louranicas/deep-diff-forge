# Output formats, exit codes and the machine contract

## Output formats and schemas

Every machine document names its schema. `--json` prints one document and `--jsonl` one
event per line. Results go to **stdout**, diagnostics to **stderr**, and on error stdout is
empty.

| Command | Schema |
| --- | --- |
| `--stdin-patch --json`, `--git-* --json` | `deep-diff-forge.review.v0` |
| `--stdin-patch --jsonl` | line events `{"event":"diff.file",…}` |
| `--rank --json` · `--cluster --json` | `deep-diff-forge.rank.v0` · `deep-diff-forge.cluster.v0` |
| `--git-* --acquisition-json` | `deep-diff-forge.git-acquisition.v0` |
| `--outline` · `--hunks`, `review.outline` · `review.hunks` · `review.change`, MCP `review_outline` · `review_hunks` · `review_change` | `deep-diff-forge.review-outline.v0` · `deep-diff-forge.review-hunks.v0` |
| `semantic --json` · `structural --json` | `deep-diff-forge.semantic.v0` · `deep-diff-forge.structural.v0` |
| `analyze` | `deep-diff-forge.analysis.v1` |
| `learn status --json` | `deep-diff-forge.learning.v0` |
| `deploy status --json` · `deploy release --json` · `deploy doctor --json` | `deep-diff-forge.deployment-status.v0` · `deep-diff-forge.release.v0` · `deep-diff-forge.deploy-doctor.v0` |
| MCP `review_patch` / `rank_patch` over 256 KiB | `deep-diff-forge.review-summary.v0` · `deep-diff-forge.rank-summary.v0` |
| `envelope` | `ddf.result.v1` |
| `agent setup --json` · `agent doctor --json` | `deep-diff-forge.agent-setup.v0` · `deep-diff-forge.agent-doctor.v0` |
| `findings verify`, `findings.verify`, MCP `verify_findings` | `deep-diff-forge.findings.v0` (input: one `deep-diff-forge.finding.v0` object per finding) |
| `daemon metrics`, `daemon.metrics` | `deep-diff-forge.daemon-metrics.v0` (`--format prometheus`: Prometheus text 0.0.4) |
| `bench --json` · `eval --json` | `deep-diff-forge.bench.v0` · `deep-diff-forge.eval.v0` (corpus: `deep-diff-forge.eval-manifest.v0`) |
| `daemon`, `rpc` | JSON-RPC 2.0, protocol 2 |

**Input sealing.** `review.v0`, `rank.v0`, `cluster.v0`, `semantic.v0` and
`git-acquisition.v0` carry two members right after `schema`:
- `input_sha256`: lowercase hex SHA-256 of the exact bytes read, before any parsing.
  `sha256sum < input` reproduces it.
- `tool`: `{"name": "deep-diff-forge", "version": "<CARGO_PKG_VERSION>"}`. In `rank.v0` (since
  0.10.0) `tool` also carries `scorer_sha256`: the SHA-256, taken when the binary is built, of
  every source of the ranking crate but its `lib.rs` (the rule tables, the secret prefixes and
  their byte classes, the readers, the path rules and weights, the order and the vocabulary). It
  moves on any edit of those files, a comment included, so two scorers never share it, which
  `version` alone cannot promise: a build from a tree between releases keeps the last version.
  It does not identify the patch parser or the other crates.

The outline and hunk views carry the seal as well.

Output is deterministic: the same bytes give byte-identical stdout on every run.

**Hunk addresses.** Every hunk in `review.v0` and in the views carries these fields:
- `address`: SHA-256 over `ddf-hunk-v1`, the file path, the header's ranges and the body. It names
  that hunk's content and place in that patch: changing the path, the position or one byte of the
  body changes it. A patch that repeats a section of one path with a byte-identical hunk at the
  same ranges gives both copies one address; `--address` then selects every copy, and a finding
  anchored to it resolves to the first in review order.
- `body_sha256`: the body alone. It survives a move, so use it to recognise the same change
  across revisions, never as proof that two reviews saw the same patch.
- `section`: the `@@` heading Git wrote. It is display-only and never re-rendered into the patch.
- `old_lines` and `new_lines`: the header's counts.

The new fields are additive, so the schema stays `review.v0`.

**Hazards.** A file whose path traverses, names `.git` or hides characters, and a line that
carries a bidi control, an invisible character or a control character, gets a `hazards` list:
`path_traversal`, `git_internal`, `bidi_control`, `invisible` or `control`. `invisible` covers
zero-width and joiner characters, invisible operators, fillers, the soft hyphen, variation
selectors (the Mongolian free variation selectors included) and tag characters: with
`bidi_control`, every Unicode `Default_Ignorable_Code_Point`, plus the interlinear annotation
controls; `control` covers C0/C1 controls other than tab and the Unicode line and paragraph
separators. The carriage return of a CRLF body line does not count; one in a path does. A
`---`/`+++` path ends at a CR, as Git's does, so a CRLF patch names the paths Git writes and a
CR reaches such a path only quoted (`"x\r"`). The `diff --git` line keeps its CR: a section
named only by that line, such as a mode change, reports a path ending in CR with `control`,
although Git refuses that patch. Hazards are facts about the bytes, reported only where found.
An empty list never means the text is safe. They also raise the file's rank
(`hazardous_text`).

```json
{
  "schema": "deep-diff-forge.rank.v0",
  "input_sha256": "9317219840467a00fbb7bc3ff905b0ae26a999582c12c8d02f49d380b398bbb3",
  "tool": {"name": "deep-diff-forge", "version": "0.10.0", "scorer_sha256": "<64 hex>"},
  "ranked": [
    {"path": "src/lib.rs", "status": "modified", "score": 7, "signals": ["public_api_surface"]},
    {"path": "tests/it.rs", "status": "modified", "score": 1, "signals": ["test_only"]}
  ]
}
```

**One byte form for `rank.v0`.** The bytes `--rank --json` prints, schema first as above with
the final newline, are the document's canonical form on every surface: the daemon's `diff.rank`
(and so `rpc` and the Python SDK's `rank_patch`) answers with the document's value plus the member
`canonical_json`, a string holding exactly those bytes, and MCP's `rank_patch` gives them as its
text, with the value as `structuredContent`. Seal or compare the bytes, not a re-serialisation of
the value. Before 0.10.0 the daemon, `rpc` and MCP gave only a compact form with sorted keys,
equal in value but not in bytes. `diff.plan` and `session.snapshot` still answer with the bare
`ranked` array, unsealed; `cluster.v0`'s `ranked` list uses the same entries without
`evidence`, `evidence_truncated` and `diff_suppressed`.

Strings are RFC 8259-escaped, and valid UTF-8 passes through. A hunk line decoded lossily is
marked `"lossy": true`; a header line that is not UTF-8 (a path, a mode, a hunk header) is
refused with exit 3, because the name it gives would be a substitute; from a Git selection such a
file name is the change's own input, refused with exit 4, and the daemon's `review.change` answers
it with code 4; a file pair refuses such a name with exit 4 too, and so does a second reading
of a binary change that the Git acquisition's byte budget cuts off. File statuses use one snake-case spelling everywhere: `added`, `modified`,
`deleted`, `renamed`, `copied`, `type_changed` (a file, a symlink or a submodule became
another), `mode_changed` (only the mode changed, such as the executable bit), `binary_changed`,
`unknown`.

## Exit codes

| Code | Meaning | Next |
| --- | --- | --- |
| 0 | Success. | `proceed` |
| 2 | CLI usage / argument error. | `fix_invocation` |
| 3 | I/O or encoding failure: reading input (stdin, a file, Git, the learning store), non-UTF-8 input, or writing output. | `check_environment` |
| 4 | Invalid input: a patch or analysis input that fails to parse or validate, `--stdin-patch` input in which Git finds no patch (`no_patch_in_input`), input over the byte budget, an invalid receipt, a named path that is not a regular file, or a refused Git acquisition or file pair. | `change_input` |
| 6 | Daemon / interactive-terminal failure. | `check_environment` |
| 7 | Input contract refused. `--require-files`: the (well-formed) patch describes 0 files — stderr `refused: 0 files in input (--require-files)`. `--require-hunks`: the patch has 0 hunks or 0 added+removed lines (header-only, rename-only, context-only) — stderr `refused: 0 hunks in input (--require-hunks)`. Gates should pass both. | `change_input` |
| 101 | Internal error (a panic). Report it as a bug. | `report_bug` |

On `--stdin-patch` the refusals come in this order: a parse error (4), then `--require-files` / `--require-hunks` (7), then input in which Git finds no patch (4, stderr `error: no_patch_in_input: …`, unless `--allow-empty`).

A reader that closes stdout ends a command with exit 0 (`… | head`), except while a
`--require-*` refusal is pending: each is decided before the output is written and exits 7
with its reason on stderr even when the reader has gone (since 0.10.0;
[DDF-2026-010](../advisories/DDF-2026-010.md)). A diagnostic that cannot be written
(`2>/dev/full`) leaves the exit code as it is; before 0.10.0 it exited 101. A daemon socket path of
108 bytes or more is a usage error (exit 2) of `daemon path` and `daemon start` since 0.10.0.

The `council` subcommands use their own family ([review council](../REVIEW_COUNCIL.md)):

| Council code | Meaning | Next |
| --- | --- | --- |
| 0 | The requested council operation completed under its contract; `status` exit 0 means ready for human review. | `proceed` |
| 3 | I/O failure after the decision was recorded: the journal holds it; run the same import again to complete it. | `check_environment` |
| 4 | Malformed input, stale admission, an existing artifact, or a protocol or I/O failure. | `change_input` |
| 7 | HOLD: failed, missing or inconclusive evidence, or an import conflict. | `review_evidence` |

**Next** is what a caller, person or agent, should do. `schema exit-codes`, `explain --json`
and the MCP tools report it as `action`, with `may_be_transient`, which is true only for
`check_environment`:

- `proceed`: use the result.
- `fix_invocation`: correct the command line; the same arguments fail the same way.
- `check_environment`: check what the message names (a file or stream, the terminal, the daemon);
  the same command can succeed once that is fixed.
- `change_input`: change the input or the state the message names (a stale admission, an
  existing artifact), or drop the contract that refused it; the same input is refused again.
- `review_evidence`: read the evidence the council reported and decide; rerunning does not change it.
- `report_bug`: report the command and its input; do not retry blindly.

Both tables come from one registry in the binary, and a test fails if this document drifts from it.

### Machine contract (`ddf.result.v1`)

```bash
deep-diff-forge envelope -- --stdin-patch --json < change.patch   # one ddf.result.v1 object
deep-diff-forge schema result        # JSON Schema (2020-12) of the envelope
deep-diff-forge schema exit-codes    # the registry: family, code, name, meaning, action
deep-diff-forge explain 7 --family council
```

`envelope` runs the command as a child of the same binary, with stdin passed through. It reports:

- `exit_code`, and `outcome`, the registry name for that code (`undocumented` for an unlisted code, `signal` and the `signal` number if the command was killed);
- `ok`, which is true only for exit 0 with nothing truncated;
- `stdout`, parsed when it is exactly one JSON value (otherwise text, so `--jsonl` stays text);
- `stderr`.

The command's own output is unchanged, and `envelope` exits with the command's code; a command ended by signal N gives 128 + N, the status a shell reports for it (not a registry code). Everything after `--` is the command's, `--help` included: `envelope -- council --help` wraps council's help. Captured stdout is capped at 16 MiB and stderr at 1 MiB; anything cut is listed in `truncated` and never parsed.

## See also

See also: [`BASH_API_CONTRACTS`](../BASH_API_CONTRACTS.md) · [`CLAUDE_CODE_BASH_CLI`](../CLAUDE_CODE_BASH_CLI.md).
