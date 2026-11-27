# Making Skills & Extensions for Deep-Diff-Forge

> Wrap the deep-diff-forge engine as a **Claude Agent Skill** or a **Pi agent
> extension** so an AI agent can drive code review *deterministically* — the
> agent supplies judgment, the engine supplies ground truth, and patch truth is
> never corrupted by either.

Deep-Diff-Forge is **Bash-first and machine-readable by design**: every command
has a deterministic invocation, versioned JSON (`--json` / `--jsonl`), stable
exit codes with registered exit-7 reason classes, and a headless mode
(`review --probe`) that needs no TTY or daemon. That makes it close to an ideal
*substrate* for an agent skill — a skill is then a thin layer of "when to call
what, and how to read the result."

This guide covers two targets:

- **Claude Agent Skills** — a `SKILL.md` folder that Claude Code / claude.ai /
  the Claude API load on demand.
- **Pi agent extensions** — a TypeScript module the Pi coding agent loads.

> **Terminology (verifiable-or-void).** Pi calls these **extensions**
> (TypeScript modules). There is **no "piece"** primitive in the current Pi docs
> — the unit types are *extensions, skills, prompts, themes*, which can be
> bundled as **packages** (npm/git). This guide documents the real
> `ExtensionAPI`; "piece" is not a Pi concept.

> **Status (v0.11.0).** Every Deep-Diff-Forge command, schema, MCP tool and exit
> code on this page was verified against `v0.11.0` source
> (`crates/deep-diff-forge-cli/src/{main,args,mcp,fail_on,refusal,agent}.rs` and
> `crates/deep-diff-forge-core/src/exit.rs`). Key facts for skill authors:
> - `--stdin-patch` refuses empty or prose input (such as `echo garbage`) with
>   **exit `4`** (`no_patch_in_input`) unless `--allow-empty` is passed; pass
>   `--require-files --require-hunks` so empty or hunkless patches refuse with
>   **exit `7`** (`require_files` / `require_hunks`) instead.
> - `--diagnostics --json` emits `deep-diff-forge.diagnostics.v0` (every ranking
>   label located with repair IDs and `texts`); `--rank --fail-on LIST` and
>   `--diagnostics --fail-on LIST` turn selected classes/labels into exit `7`
>   gates (`fail_on_label` / `fail_on_truncated`).
> - `deep-diff-forge mcp` ships **9 read-only tools** (including `review_outline`,
>   `review_hunks`, `diagnose_patch`, `review_change`, `verify_findings`), and
>   `deep-diff-forge agent setup --client claude --hooks` prints ready-to-paste
>   MCP and Claude Code hook settings.
> - Everything on this page uses shipped commands; the Claude Skills and Pi
>   `ExtensionAPI` specs describe third-party hosts — verify host specifics
>   against [§7 Sources](#7-sources).

### Which should you build — Skill, extension, MCP server, hooks, or slash command?

| You want… | Build / Use | Why |
| --- | --- | --- |
| Claude (Code / claude.ai / API) to *autonomously* review when asked | **Claude Agent Skill** | triggers on `description`; progressive-disclosure-cheap; no glue code |
| Claude Code to get *automatic* post-tool advice and stop-time policy gating | built-in **Claude Code hooks** (`deep-diff-forge agent setup --client claude --hooks`) | ships in the binary: `SessionStart`, `PostToolUse` (`diagnostics.v0` advice + guard-file notices), `Stop` (`forge-warden` judging + `.git/deep-diff-forge/ledger.jsonl`) |
| The **Pi** agent to expose review as a callable tool / `/command` | **Pi extension** | first-class `registerTool` the LLM can call mid-turn |
| A fixed, always-identical review command in Claude Code | a **slash command** (`.claude/commands/*.md`) | simpler than a skill when there is no "when to use" judgment |
| Any MCP client to call review as a tool, with no glue code | the built-in **MCP server** (`deep-diff-forge mcp`) | ships in the binary: **9 read-only tools** (`review_outline`, `review_hunks`, `diagnose_patch`, `review_change`, `review_patch`, `rank_patch`, `verify_findings`, `explain_exit_code`, `exit_code_registry`); `deep-diff-forge agent setup --client claude\|codex` prints the registration |
| The same capability in *both* agents | a Skill **and** an extension over **one** `scripts/review.sh` | one substrate script, two thin wrappers — behavior can't drift |

Keep the real work in a small script both wrappers call; the skill/extension is
just "when to call it, and how to read the result."

## Quickstart — a working review Skill in 60 seconds

```bash
mkdir -p .claude/skills/deep-diff-review/scripts
cat > .claude/skills/deep-diff-review/scripts/review.sh <<'SH'
#!/usr/bin/env bash
set -uo pipefail
git diff "$@" | deep-diff-forge --stdin-patch --diagnostics --json --require-files --require-hunks
SH
chmod +x .claude/skills/deep-diff-review/scripts/review.sh
```

Then save `.claude/skills/deep-diff-review/SKILL.md` with the frontmatter + body
from [§2.4](#24-worked-example--skillmd). Ask Claude to "review my diff" — it
discovers the skill from the `description`, runs the script, and inspects the
highest-risk files and located diagnostics first. The full anatomy (and the Pi
equivalent) are below.

---

## 1. The substrate contract — what a skill actually calls

A skill should drive the **machine-readable** surface and branch on **exit
codes** (and exit-7 reason tags), never scrape human prose. Agent-facing
guarantees are printed by `deep-diff-forge claude-code-contract`; the commands
a review skill leans on:

| Goal | Command | Output (schema) |
| --- | --- | --- |
| Located diagnostics & repair IDs | `git diff \| deep-diff-forge --stdin-patch --diagnostics --json` | `deep-diff-forge.diagnostics.v0` |
| Risk-ranked review | `git diff \| deep-diff-forge --stdin-patch --rank --json` | `deep-diff-forge.rank.v0` |
| Read directly from Git (untracked files included) | `deep-diff-forge --git-working-tree --include-untracked --diagnostics --json` | `deep-diff-forge.diagnostics.v0` |
| Compare two regular files directly | `deep-diff-forge <old> <new> --rank --json` | `deep-diff-forge.rank.v0` |
| Gate on non-empty/non-hunkless diff | `… --rank --json --require-files --require-hunks` | exit `7` (`require_files` / `require_hunks`, empty stdout) |
| Gate on signal classes or labels | `… --diagnostics --json --fail-on security,integrity` | `diagnostics.v0` on stdout, then exit `7` (`fail_on_label` / `fail_on_truncated`) |
| Budgeted outline & targeted hunks | `… --outline --max-bytes 16384` · `… --hunks --address A` | `deep-diff-forge.review-outline.v0` · `deep-diff-forge.review-hunks.v0` |
| Full review document | `… --stdin-patch --json` | `deep-diff-forge.review.v0` |
| Streamed per-file events | `… --stdin-patch --jsonl` | `{"event":"diff.file",…}` lines |
| Parallel ranking + receipt | `… --stdin-patch --cluster --json` | `deep-diff-forge.cluster.v0` |
| Explain a signal for an agent | `deep-diff-forge explain signal <label> --audience agent --json` | `deep-diff-forge.signal-explain.v0` |
| Symbols of a file | `deep-diff-forge semantic <path> --json` | `deep-diff-forge.semantic.v0` |
| Reformat-aware structural diff | `deep-diff-forge structural <old> <new> --json` | `deep-diff-forge.structural.v0` |
| Headless review frame | `git diff \| deep-diff-forge review --probe` | rendered TUI frame (text) |
| Check agent findings against the patch | `deep-diff-forge findings verify --findings found.jsonl --require-findings --require-all < change.patch` | `deep-diff-forge.findings.v0` (exit `7` on refusal) |
| On-demand stop-judge check & branch ledger check | `deep-diff-forge agent check --require-pass --json` · `agent verify-branch BASE..HEAD --json` | `deep-diff-forge.agent-check.v0` · `deep-diff-forge.agent-verify-branch.v0` (exit `7` on refusal) |
| Record a learning receipt | `echo '<receipt>' \| deep-diff-forge learn record --stdin` | `recorded: …` line (exit `4` for an invalid receipt) |
| Any command as one JSON object | `deep-diff-forge envelope -- <command…>` | `ddf.result.v1` |
| 9 read-only tools for an MCP client | `deep-diff-forge mcp [--allow-repo PATH]...` | MCP over stdio |

**Exit codes** (branch on these; `deep-diff-forge schema exit-codes` prints the
full registry with `action` and `refusals`):

- `0` (`success`) — proceed.
- `2` (`usage`) — CLI usage/argument error or withdrawn subcommand (`chain`, `cluster`, `loom`).
- `3` (`io`) — I/O or encoding failure (unreadable file, Git failure, non-UTF-8 input).
- `4` (`invalid_input`) — malformed/truncated patch, `--stdin-patch` input with no patch (`no_patch_in_input`, unless `--allow-empty`), input over the 64 MiB byte budget, invalid receipt, non-regular file path, or refused Git acquisition / file pair.
- `6` (`daemon_or_terminal`) — daemon or interactive-terminal failure (`review` without TTY).
- `7` (`input_contract_refused`) — contract or gate refusal; `stderr` prints `refused: <text> (<tag>)` naming one of the **11 registered reason classes**:
  - Input refusals (empty `stdout`): `require_files` (`--require-files`), `require_hunks` (`--require-hunks`).
  - Verdict refusals (document printed to `stdout` first, exit `7` preserved even if stdout closes): `require_findings` (`--require-findings`), `require_all` (`--require-all`), `eval_failed` (`eval`), `chain_not_ready` (`agent doctor`), `fail_on_label` (`--fail-on`), `fail_on_truncated` (`--fail-on evidence_truncated`), `require_pass_no_unit` (`--require-pass no_unit`), `require_pass_not_pass` (`--require-pass not_pass`), `branch_unverified` (`verify-branch`).
- `101` (`panic`) — internal error.

Diagnostics go to **stderr**; primary records go to **stdout**.

A typical "powerful review" loop is therefore:

```bash
git diff | deep-diff-forge --stdin-patch --outline --max-bytes 16384 --require-files --require-hunks  # 1. risk-ordered map
git diff | deep-diff-forge --stdin-patch --diagnostics --json --require-files --require-hunks         # 2. located signals & repair IDs
git diff | deep-diff-forge --stdin-patch --hunks --address <HUNK_ADDR>                                # 3. zoom into hot hunks
deep-diff-forge semantic <hot-file> --json                                                            # 4. symbols touched
deep-diff-forge structural <old> <new> --json                                                         # 5. real change vs reformat?
# 6. agent writes findings JSONL and verifies anchors with `findings verify --require-all`
```

See also [`CLAUDE_CODE_BASH_CLI.md`](CLAUDE_CODE_BASH_CLI.md) (agent invocation
patterns & hooks) and [`API_AND_IPC.md`](API_AND_IPC.md) (schemas, MCP tools &
daemon protocol 3).

---

## 2. Part A — A Claude Agent Skill

A Skill is a **folder with a `SKILL.md`** plus optional bundled files. Claude
loads it through **progressive disclosure**, so a skill can be large on disk yet
cheap in context.

### 2.1 Frontmatter — the only required metadata

```yaml
---
name: deep-diff-review
description: Review a code change (a git diff or .patch) with deep-diff-forge — risk-rank the files, locate security/integrity diagnostics, inspect the highest-impact hunks, and produce a prioritized review. Use when the user asks to review a diff/PR/changeset, asks "what should I look at first", or pipes a patch for analysis.
---
```

**Field rules (authoritative):**

| Field | Required | Constraints |
| --- | --- | --- |
| `name` | yes | ≤ 64 chars; lowercase letters, numbers, hyphens only; no XML tags; cannot contain the reserved words `anthropic` / `claude`. |
| `description` | yes | non-empty; ≤ 1024 chars; no XML tags. **Must state both what the skill does and when to use it** — this is the only thing always in context, so it is what makes the skill trigger. |

### 2.2 Progressive disclosure (why a skill stays cheap)

| Level | Loaded | Cost | Holds |
| --- | --- | --- | --- |
| 1 — Metadata | always (startup) | ~100 tokens | `name` + `description` |
| 2 — Instructions | when triggered | < 5k tokens | the `SKILL.md` body |
| 3 — Resources/code | on demand, via bash | ~unlimited | extra `.md` files, scripts run for their *output only* |

So put the decision logic in the body, and push bulky reference material and
**deterministic operations into scripts** — Claude runs `scripts/review.sh` and
sees only its stdout, never the script's source.

### 2.3 Directory layout

```text
deep-diff-review/
├── SKILL.md            # frontmatter + the review workflow
├── REFERENCE.md        # diagnostics.v0 / rank.v0 / review.v0 schema cheat-sheet (loaded on demand)
└── scripts/
    └── review.sh       # git diff | deep-diff-forge --stdin-patch --diagnostics --json --require-files --require-hunks
```

Discovery (Claude Code): personal `~/.claude/skills/<name>/` or project
`.claude/skills/<name>/`; both are picked up automatically, no upload. (On
claude.ai you upload a zip; via the API you POST to `/v1/skills`.)

### 2.4 Worked example — `SKILL.md`

````markdown
---
name: deep-diff-review
description: Review a code change (a git diff or .patch) with deep-diff-forge — risk-rank the files, locate security/integrity diagnostics, inspect the highest-impact hunks, and produce a prioritized review. Use when the user asks to review a diff/PR/changeset, asks "what should I look at first", or pipes a patch for analysis.
---

# Deep-Diff Review

Drive `deep-diff-forge` to review a changeset in priority order. The engine is
the source of truth for *what changed*, *what is risky*, and *where signals
stand*; you supply the judgment about whether it is correct.

## Workflow

1. **Rank & diagnose.** Get the risk-ordered outline and located diagnostics (or
   run `scripts/review.sh`):
   ```bash
   git diff | deep-diff-forge --stdin-patch --rank --json --require-files --require-hunks
   git diff | deep-diff-forge --stdin-patch --diagnostics --json --require-files --require-hunks
   ```
   - Exit `4`: malformed patch or no valid patch in input (`no_patch_in_input`) —
     report that, do not invent a review.
   - Exit `7`: input contract refused (`--require-files` or `--require-hunks`) —
     there are no changed hunks to review; state that plainly.

2. **Triage.** Inspect `ranked` (`rank.v0`: `score`, `signals`, `evidence`) and
   `diagnostics` (`diagnostics.v0`: `code`, `class` in `security`/`integrity`/`shape`,
   `path`, `side`, `line`, `repairs`, `not_accepted`, `texts`). For any signal
   you want details on:
   ```bash
   deep-diff-forge explain signal <label> --audience agent --json
   ```

3. **Zoom in** on the top files/hunks only:
   ```bash
   git diff | deep-diff-forge --stdin-patch --hunks --path <path> --max-bytes 16384
   deep-diff-forge semantic <path> --json        # symbols touched
   deep-diff-forge structural <old> <new> --json # real change vs reformat-only
   ```

4. **Verify & report** a prioritized review: per high-risk file, the symbols
   affected, the concrete concern, and a clear verdict. When emitting structured
   findings (`finding.v0` JSONL), check anchors against the patch with
   `deep-diff-forge findings verify --findings found.jsonl --require-all`.
   Never claim a file is fine that you did not actually open.

Schema field reference: [REFERENCE.md](REFERENCE.md).
````

And the bundled script `scripts/review.sh` (Level 3 — runs for its output, costs
no context):

```bash
#!/usr/bin/env bash
# Located diagnostics JSON for the working tree (or a ref range passed as $@).
set -uo pipefail
git diff "$@" | deep-diff-forge --stdin-patch --diagnostics --json --require-files --require-hunks
```

### 2.5 Make the `description` trigger well

The description is the whole game for triggering. Name the **artifacts**
("diff", "patch", "PR", "changeset"), the **verbs** ("review", "what to look at
first"), and keep it specific so it fires for review requests and stays quiet
otherwise.

---

## 3. Part B — A Pi agent extension

Pi extensions are **TypeScript modules** auto-discovered from:

- `~/.pi/agent/extensions/*.ts` or `~/.pi/agent/extensions/*/index.ts` (global)
- `.pi/extensions/*.ts` or `.pi/extensions/*/index.ts` (project)

Multi-file / npm extensions point an entry in `package.json`:

```json
{ "pi": { "extensions": ["./src/index.ts"] } }
```

### 3.1 The `ExtensionAPI`

The module default-exports a factory receiving `pi`:

```typescript
export default function (pi: ExtensionAPI): void | Promise<void>
```

| Method | Use |
| --- | --- |
| `pi.registerTool({ name, label, description, parameters, execute })` | a tool the LLM can call |
| `pi.registerCommand(name, { description, handler })` | a slash command the user runs |
| `pi.on(event, handler)` | lifecycle hook (`tool_call`, `tool_result`, `before_agent_start`, `session_start`, …) |
| `pi.registerShortcut(key, { description, handler })` | a keybinding |
| `pi.registerFlag(name, { type, default, description })` + `pi.getFlag(name)` | a CLI flag |
| `pi.appendEntry(type, data)` | persist session-local state (not sent to the LLM) |

### 3.2 Worked example — `.pi/extensions/deep-diff-forge.ts`

```typescript
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { execFileSync } from "node:child_process";

/** git diff <ref> | deep-diff-forge --stdin-patch --diagnostics --json --require-files --require-hunks */
function diagnoseReview(ref: string): string {
  // Split a ref/range like "HEAD~1 HEAD" into separate argv entries (passing it
  // as one arg would make git look for a single ref literally named with a space).
  const refArgs = ref.trim() ? ref.trim().split(/\s+/) : [];
  const patch = execFileSync("git", ["diff", ...refArgs], {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  try {
    return execFileSync(
      "deep-diff-forge",
      ["--stdin-patch", "--diagnostics", "--json", "--require-files", "--require-hunks"],
      { input: patch, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 },
    );
  } catch (err: any) {
    // deep-diff-forge exits non-zero on invalid input (4 = malformed patch or no
    // patch in input) or gate/contract refusal (7 = --require-files/--require-hunks/--fail-on).
    // Surface exit code, stdout (present on verdict refusals), and stderr cleanly.
    const code = err?.status ?? "unknown";
    const stdout = (err?.stdout ?? "").toString().trim();
    const stderr = (err?.stderr ?? "").toString().trim();
    return JSON.stringify({ error: `deep-diff-forge exited ${code}`, stderr, stdout: stdout || undefined });
  }
}

export default function (pi: ExtensionAPI) {
  // A tool the model can call mid-conversation.
  pi.registerTool({
    name: "deep_diff_diagnose",
    label: "Deep-Diff Diagnose",
    description:
      "Locate every risk-ranking label with deep-diff-forge and return deep-diff-forge.diagnostics.v0 JSON. " +
      "Pass an optional git ref/range (e.g. 'HEAD~1 HEAD'); omit for the working tree.",
    parameters: Type.Object({
      ref: Type.Optional(
        Type.String({ description: "git ref or range; empty = working tree" }),
      ),
    }),
    async execute(_id, params, _signal, _onUpdate, _ctx) {
      const json = diagnoseReview(params.ref ?? "");
      return { content: [{ type: "text", text: json }] };
    },
  });

  // A slash command for the human operator.
  pi.registerCommand("ddf-review", {
    description: "Diagnostics review of the current diff with deep-diff-forge",
    handler: async (args, ctx) => {
      ctx.ui.notify(diagnoseReview(args.trim()), "info");
    },
  });
}
```

Run ad-hoc with `pi -e ./.pi/extensions/deep-diff-forge.ts`, or drop the file in
a discovery path.

### 3.3 A safety hook (optional but powerful)

Use `pi.on("tool_call", …)` to keep the engine honest — e.g. surface a warning
before the agent acts on a high-risk patch:

```typescript
pi.on("tool_call", async (event, ctx) => {
  // `tool_call` fires *before* a tool runs; its payload carries the tool name and
  // arguments (confirm the exact field shape against the live ExtensionAPI types).
  // Return `{ block: true }` to veto a call entirely; here we just surface a note.
  ctx.ui.notify("A review tool is about to run — patch truth stays read-only.", "info");
});
```

---

## 4. What makes a skill/extension *powerful*

Powerful skills aren't bigger — they exploit the engine's guarantees:

- **Read JSON, not prose.** `--diagnostics --json` (`diagnostics.v0`) and
  `--rank --json` (`rank.v0`) give stable fields (`code`, `class`, `path`,
  `side`, `line`, `score`, `signals`, `repairs`); branch on those, never on
  rendered text that may reflow.
- **Branch on exit codes and exit-7 reason tags.** Exit `4` means *malformed
  patch* or *no patch in input* (`no_patch_in_input` on `--stdin-patch` without
  `--allow-empty`) — refuse rather than hallucinate a review. Pass
  `--require-files --require-hunks` so empty or hunkless changes refuse with
  exit `7` (`require_files` / `require_hunks`), and add `--fail-on security,integrity`
  when you want a signal gate (exit `7` `fail_on_label` / `fail_on_truncated`,
  with the JSON document still printed on `stdout`).
- **Spend the priority signal.** Start with `--outline` or `--diagnostics --json`,
  then deep-dive only the highest-risk hunks/files with `--hunks`, `semantic`
  and `structural`.
- **Stay headless.** `review --probe` renders one frame with no TTY — perfect for
  CI and agents that can't attach a terminal.
- **Keep patch truth sacred.** The engine never mutates the patch; your skill
  shouldn't either. Annotations and findings (`findings verify`) are *layers on top*.
- **Close the loop.** Feed outcomes back with `learn record --stdin` so the L9
  learning store can score review strategies over time.
- **Scale out.** For large reviews, `--cluster [--parallel N]` ranks via bounded
  parallel lanes with a deterministic join (identical results for any worker
  count). The optional UDS daemon and `deep-diff-forge rpc`
  ([`API_AND_IPC.md`](API_AND_IPC.md), protocol 3) give the same answers over
  JSON-RPC; the daemon keeps review sessions in memory but has no response cache.
- **Or skip the wrapper.** `deep-diff-forge mcp` already exposes **9 read-only
  MCP tools**, `deep-diff-forge agent setup --client claude --hooks` wires
  automatic hooks directly into Claude Code, and the Python SDK
  (`sdk/python/deep_diff_forge.py`) wraps `rpc` and the daemon socket for scripts.

---

## 5. Testing your skill/extension

```bash
# the substrate is deterministic — assert on it directly
git diff HEAD~1 HEAD | deep-diff-forge --stdin-patch --rank --json | jq '.ranked[0]'
git diff HEAD~1 HEAD | deep-diff-forge --stdin-patch --diagnostics --json | jq '.counts'

# truncated hunk -> exit 4
printf -- '--- a/x\n+++ b/x\n@@ -1,5 +1,1 @@\n-a\n' \
  | deep-diff-forge --stdin-patch --rank --json; echo "exit=$?"

# prose / empty stdin without guards -> exit 4 (no_patch_in_input)
echo "garbage" | deep-diff-forge --stdin-patch --rank --json; echo "exit=$?"

# prose / empty stdin with --require-files -> exit 7 (require_files)
echo "garbage" | deep-diff-forge --stdin-patch --rank --json --require-files; echo "exit=$?"

# intentional empty change -> exit 0 with --allow-empty
printf '' | deep-diff-forge --stdin-patch --rank --json --allow-empty; echo "exit=$?"

# headless render (skills/CI, no TTY)
git diff | deep-diff-forge review --probe --cols 120 --rows 40

# Pi extension smoke test
pi -e ./.pi/extensions/deep-diff-forge.ts
```

For a Claude Skill, the cheapest check is to install it under
`.claude/skills/`, ask a review question, and confirm it triggers (the
`description` is doing its job) and runs the workflow.

### Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| Skill never triggers | `description` too vague / missing the "when" | name the artifacts (diff/patch/PR) **and** verbs (review) in `description` |
| Engine returns `exit 4` | malformed/truncated patch, or empty/prose stdin without `--allow-empty` (`no_patch_in_input`) | report malformed input or empty diff; pass `--allow-empty` only when empty stdin is intentional |
| Engine returns `exit 7` | input contract (`--require-files`/`--require-hunks`) or verdict gate (`--fail-on`, `findings verify`, `agent check`, `agent verify-branch`) refused | read `refused: <text> (<tag>)` on stderr (and the JSON document on stdout for verdict refusals) |
| `review` returns `exit 6` | no TTY attached | use `review --probe` for headless / CI / agent contexts |
| Pi tool throws | engine exited non-zero and `execFileSync` raised | catch `err.status`, `err.stdout` and `err.stderr` and return a structured result (see §3.2) |
| `SKILL.md` body too large | exceeds the Level-2 budget | move bulky reference material to a Level-3 file (`REFERENCE.md`) loaded on demand |
| Skill works in Claude Code but not claude.ai/API | Skills don't sync across surfaces | upload separately per surface (Claude Code is filesystem-only) |

---

## 6. Security & trust

A skill/extension can direct an agent to run code, so treat one like installing
software: **only use skills from trusted sources, and audit every bundled file**
(`SKILL.md`, scripts, the extension `.ts`). Deep-Diff-Forge itself is built to
run on **untrusted input** (the diff is attacker-controlled) and is adversarially
hardened — see [`../SECURITY.md`](../SECURITY.md) — but your skill's own scripts
are your responsibility: avoid unsanitized shell interpolation of refs/paths, and
prefer `execFileSync(cmd, [args])` over a shell string.

---

## 7. Sources

- Claude — *What are Skills?* — https://support.claude.com/en/articles/12512176-what-are-skills
- Claude — *Agent Skills (overview + authoring spec)* — https://platform.claude.com/docs/en/agents-and-tools/agent-skills/overview
- Pi — *Extensions* — https://pi.dev/docs/latest/extensions

> Back to: [README](../README.md) · [`DEPLOYMENT_FRAMEWORK.md`](DEPLOYMENT_FRAMEWORK.md)
