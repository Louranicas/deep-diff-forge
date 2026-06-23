# Making Skills & Extensions for Deep-Diff-Forge

> Wrap the deep-diff-forge engine as a **Claude Agent Skill** or a **Pi agent
> extension** so an AI agent can drive code review *deterministically* — the
> agent supplies judgment, the engine supplies ground truth, and patch truth is
> never corrupted by either.

Deep-Diff-Forge is **Bash-first and machine-readable by design**: every command
has a deterministic invocation, versioned JSON (`--json` / `--jsonl`), stable
exit codes, and a headless mode (`review --probe`) that needs no TTY or daemon.
That makes it close to an ideal *substrate* for an agent skill — a skill is then
a thin layer of "when to call what, and how to read the result."

This guide covers two targets:

- **Claude Agent Skills** — a `SKILL.md` folder that Claude Code / claude.ai /
  the Claude API load on demand.
- **Pi agent extensions** — a TypeScript module the Pi coding agent loads.

> **Terminology (verifiable-or-void).** Pi calls these **extensions**
> (TypeScript modules). There is **no "piece"** primitive in the current Pi docs
> — the unit types are *extensions, skills, prompts, themes*, which can be
> bundled as **packages** (npm/git). This guide documents the real
> `ExtensionAPI`; "piece" is not a Pi concept.

### Which should you build — Skill, extension, or slash command?

| You want… | Build | Why |
| --- | --- | --- |
| Claude (Code / claude.ai / API) to *autonomously* review when asked | **Claude Agent Skill** | triggers on the `description`; progressive-disclosure-cheap; no glue code |
| The **Pi** agent to expose review as a callable tool / `/command` | **Pi extension** | first-class `registerTool` the LLM can call mid-turn |
| A fixed, always-identical review command in Claude Code | a **slash command** (`.claude/commands/*.md`) | simpler than a skill when there is no "when to use" judgment |
| The same capability in *both* agents | a Skill **and** an extension over **one** `scripts/review.sh` | one substrate script, two thin wrappers — behavior can't drift |

Keep the real work in a small script both wrappers call; the skill/extension is
just "when to call it, and how to read the result."

## Quickstart — a working review Skill in 60 seconds

```bash
mkdir -p .claude/skills/deep-diff-review/scripts
cat > .claude/skills/deep-diff-review/scripts/review.sh <<'SH'
#!/usr/bin/env bash
set -uo pipefail
git diff "$@" | deep-diff-forge --stdin-patch --rank --json
SH
chmod +x .claude/skills/deep-diff-review/scripts/review.sh
```

Then save `.claude/skills/deep-diff-review/SKILL.md` with the frontmatter + body
from [§2.4](#24-worked-example--skillmd). Ask Claude to "review my diff" — it
discovers the skill from the `description`, runs the script, and reviews the
high-risk files first. The full anatomy (and the Pi equivalent) are below.

---

## 1. The substrate contract — what a skill actually calls

A skill should drive the **machine-readable** surface and branch on **exit
codes**, never scrape human prose. The agent-facing guarantees are emitted by
`deep-diff-forge claude-code-contract`; the commands a review skill leans on:

| Goal | Command | Output (schema) |
| --- | --- | --- |
| Risk-ranked review stream | `git diff \| deep-diff-forge --stdin-patch --rank --json` | `deep-diff-forge.rank.v0` |
| Full review document | `… --stdin-patch --json` | `deep-diff-forge.review.v0` |
| Streamed per-file events | `… --stdin-patch --jsonl` | `{"event":"diff.file",…}` lines |
| Parallel ranking + receipt | `… --stdin-patch --cluster --json` | `deep-diff-forge.cluster.v0` |
| Symbols of a file | `deep-diff-forge semantic <path> --json` | `deep-diff-forge.semantic.v0` |
| Reformat-aware structural diff | `deep-diff-forge structural <old> <new> --json` | `deep-diff-forge.structural.v0` |
| Headless review frame | `git diff \| deep-diff-forge review --probe` | rendered TUI frame (text) |
| Record a learning receipt | `echo '<receipt>' \| deep-diff-forge learn record --stdin` | confirmation |

**Exit codes** (branch on these): `0` success · `2` usage/argument · `3` input
read failure · `4` patch parse failure · `6` daemon / interactive-terminal
failure. Diagnostics go to **stderr**; primary records to **stdout**.

A typical "powerful review" loop is therefore:

```bash
git diff | deep-diff-forge --stdin-patch --rank --json   # 1. what to look at first
deep-diff-forge semantic <hot-file> --json               # 2. zoom into the riskiest
deep-diff-forge structural <old> <new> --json            # 3. is the change real or reformat?
# 4. agent writes the review; optionally records a learn receipt
```

See also [`CLAUDE_CODE_BASH_CLI.md`](CLAUDE_CODE_BASH_CLI.md) (agent invocation
patterns) and [`API_AND_IPC.md`](API_AND_IPC.md) (schemas + the daemon).

---

## 2. Part A — A Claude Agent Skill

A Skill is a **folder with a `SKILL.md`** plus optional bundled files. Claude
loads it through **progressive disclosure**, so a skill can be large on disk yet
cheap in context.

### 2.1 Frontmatter — the only required metadata

```yaml
---
name: deep-diff-review
description: Review a code change (a git diff or .patch) with deep-diff-forge — risk-rank the files, inspect the highest-impact ones, and produce a prioritized review. Use when the user asks to review a diff/PR/changeset, asks "what should I look at first", or pipes a patch for analysis.
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
├── REFERENCE.md        # the rank.v0 / review.v0 schema cheat-sheet (loaded on demand)
└── scripts/
    └── review.sh       # git diff | deep-diff-forge --stdin-patch --rank --json
```

Discovery (Claude Code): personal `~/.claude/skills/<name>/` or project
`.claude/skills/<name>/`; both are picked up automatically, no upload. (On
claude.ai you upload a zip; via the API you POST to `/v1/skills`.)

### 2.4 Worked example — `SKILL.md`

````markdown
---
name: deep-diff-review
description: Review a code change (a git diff or .patch) with deep-diff-forge — risk-rank the files, inspect the highest-impact ones, and produce a prioritized review. Use when the user asks to review a diff/PR/changeset, asks "what should I look at first", or pipes a patch for analysis.
---

# Deep-Diff Review

Drive `deep-diff-forge` to review a changeset in priority order. The engine is
the source of truth for *what changed* and *what is risky*; you supply the
judgment about whether it is correct.

## Workflow

1. **Rank.** Get the risk-ordered stream (or run `scripts/review.sh`):
   ```bash
   git diff | deep-diff-forge --stdin-patch --rank --json
   ```
   On exit `4` the patch is malformed — report that, do not invent a review.

2. **Triage.** Read the `ranked` array. `score` is review priority (higher =
   first); `signals` explain why (`public_api_surface`, `large_change`,
   `many_hunks`, `new_file`/`deleted_file`, `binary_change`,
   `config_or_lockfile`, `test_only`, `generated_or_vendored`).

3. **Zoom in** on the top files only:
   ```bash
   deep-diff-forge semantic <path> --json        # symbols touched
   deep-diff-forge structural <old> <new> --json # real change vs reformat-only
   ```

4. **Report** a prioritized review: per high-risk file, the symbols affected,
   the concrete concern, and a clear verdict. Never claim a file is fine that
   you did not actually open.

Schema field reference: [REFERENCE.md](REFERENCE.md).
````

And the bundled script `scripts/review.sh` (Level 3 — runs for its output, costs
no context):

```bash
#!/usr/bin/env bash
# Risk-ranked review JSON for the working tree (or a ref range passed as $@).
set -uo pipefail
git diff "$@" | deep-diff-forge --stdin-patch --rank --json
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

/** git diff <ref> | deep-diff-forge --stdin-patch --rank --json */
function rankedReview(ref: string): string {
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
      ["--stdin-patch", "--rank", "--json"],
      { input: patch, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 },
    );
  } catch (err: any) {
    // deep-diff-forge exits non-zero on bad input (4 = malformed patch). Surface
    // it as a structured result instead of throwing into the agent loop — this is
    // the "branch on exit codes" rule applied at the wrapper boundary.
    const code = err?.status ?? "unknown";
    const stderr = (err?.stderr ?? "").toString().trim();
    return JSON.stringify({ error: `deep-diff-forge exited ${code}`, stderr });
  }
}

export default function (pi: ExtensionAPI) {
  // A tool the model can call mid-conversation.
  pi.registerTool({
    name: "deep_diff_review",
    label: "Deep-Diff Review",
    description:
      "Risk-rank a changeset with deep-diff-forge and return the rank.v0 JSON. " +
      "Pass an optional git ref/range (e.g. 'HEAD~1 HEAD'); omit for the working tree.",
    parameters: Type.Object({
      ref: Type.Optional(
        Type.String({ description: "git ref or range; empty = working tree" }),
      ),
    }),
    async execute(_id, params, _signal, _onUpdate, _ctx) {
      const json = rankedReview(params.ref ?? "");
      return { content: [{ type: "text", text: json }] };
    },
  });

  // A slash command for the human operator.
  pi.registerCommand("ddf-review", {
    description: "Risk-ranked deep-diff-forge review of the current diff",
    handler: async (args, ctx) => {
      ctx.ui.notify(rankedReview(args.trim()), "info");
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

- **Read JSON, not prose.** `--rank --json` gives stable `path`/`score`/`signals`
  fields; branch on those, never on rendered text that may reflow.
- **Branch on exit codes.** Exit `4` means *malformed patch* — refuse rather than
  hallucinate a review. Exit `0` with empty `ranked` means *nothing to review*.
- **Spend the priority signal.** Review the top of the `ranked` list first;
  deep-dive only the high-`score` files with `semantic` / `structural`.
- **Stay headless.** `review --probe` renders one frame with no TTY — perfect for
  CI and agents that can't attach a terminal.
- **Keep patch truth sacred.** The engine never mutates the patch; your skill
  shouldn't either. Annotations and verdicts are *layers on top*.
- **Close the loop.** Feed outcomes back with `learn record --stdin` so the L9
  learning store can score review strategies over time.
- **Scale out.** For large or repeated reviews, `--cluster [--parallel N]` ranks
  via bounded parallel lanes with a deterministic join (identical results for any
  worker count), and the optional UDS daemon ([`API_AND_IPC.md`](API_AND_IPC.md))
  keeps a warm cache across calls — same answers, lower latency.

---

## 5. Testing your skill/extension

```bash
# the substrate is deterministic — assert on it directly
git diff HEAD~1 HEAD | deep-diff-forge --stdin-patch --rank --json | jq '.ranked[0]'
echo "garbage" | deep-diff-forge --stdin-patch --rank --json; echo "exit=$?"   # expect 4

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
| Engine returns `exit 4` | malformed / truncated patch | the patch is the problem — report it, don't synthesize a review |
| `review` returns `exit 6` | no TTY attached | use `review --probe` for headless / CI / agent contexts |
| Pi tool throws | engine exited non-zero and `execFileSync` raised | catch `err.status` / `err.stderr` and return a structured result (see §3.2) |
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
