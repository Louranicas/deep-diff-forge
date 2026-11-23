# Command reference

`deep-diff-forge --help` is the authoritative list, and `deep-diff-forge <command> --help` prints one command's usage and what it does, with exit 0. A test runs it for every command `--help` lists.

## Review a change

```bash
deep-diff-forge --stdin-patch [MODE] [--require-files] [--require-hunks] [--allow-empty]
deep-diff-forge {--git-working-tree [--include-untracked] | --git-staged | --git-range A..B} [--repo PATH] [MODE | --acquisition-json]
deep-diff-forge [--] <old> <new> [MODE] [--require-files] [--require-hunks]
# MODE: --json | --jsonl | --rank | --cluster [--parallel N] | --layout inline|side-by-side
#       | --outline | --hunks [--address A]... [--path P]...   (both: [--max-bytes N] [--cursor C])
```

Choose one MODE: a second is refused with exit 2 before any input is read. `--json` also
selects the JSON form of `--rank` and `--cluster` (and is accepted, redundantly, with
`--outline` and `--hunks`, which always print JSON). An option that takes one value is
refused when given twice; only `--address` and `--path` repeat.

| Mode | Output |
| --- | --- |
| *(none)* | One line per file: status, path, `+adds -dels`, hunk count. |
| `--json` | One `deep-diff-forge.review.v0` document: files, hunks, line anchors, summary, seal. |
| `--jsonl` | One `{"event":"diff.file",…}` event per file, streamed. |
| `--rank` | Files by review risk, highest first, with their signals. Add `--json` for `deep-diff-forge.rank.v0`. |
| `--cluster [--parallel serial\|auto\|N]` | The same ranking through bounded parallel lanes with a deterministic join and a receipt. Add `--json` for `deep-diff-forge.cluster.v0`. |
| `--layout inline` / `--layout side-by-side` | Text projections with old and new line numbers, as wide as the file's largest. A `\` note row follows what a row cannot show: a non-UTF-8 line shown lossily, `\ No newline at end of file`, and a removed/added pair whose printed text looks the same (a tab against the spaces it advances to, a literal `\x1b` against a raw `ESC`, a trailing or no-break space, or in side-by-side a change past the column width). Inline prints tabs raw and judges them by 8-column tab stops; side-by-side shows a tab as four spaces. Letters that only look alike (Cyrillic `а` against Latin `a`) are not noted. |
| `--require-files` | Refuse a patch with 0 files: exit 7, empty stdout, `refused: 0 files in input (--require-files)`. |
| `--require-hunks` | Refuse a patch with 0 hunks or 0 changed lines (header-only, rename-only, context-only): exit 7, `refused: 0 hunks in input (--require-hunks)`. |
| `--allow-empty` | `--stdin-patch` only, Git's flag with Git's meaning: input in which Git finds no patch (zero bytes, whitespace, prose, a commit message alone) is an empty change, exit 0 with 0 files. Without it that input is refused as `git apply` refuses it: exit 4, empty stdout, stderr `error: no_patch_in_input: …`. The Git selections on a clean tree and a file pair of identical files still answer 0 files with exit 0. |
| `--outline` | `deep-diff-forge.review-outline.v0`: files in review-risk order, each hunk's address, ranges, `@@` heading and counts, no line text. |
| `--hunks [--address A]... [--path P]...` | `deep-diff-forge.review-hunks.v0`: the chosen hunks verbatim (all, by address or by path). A long hunk continues at a line (`complete: false`). |
| `--max-bytes N` · `--cursor C` | For `--outline` / `--hunks`: the most bytes the answer may take (1 KiB–16 MiB, default 64 KiB), and the `next_cursor` of the previous page. The page prints as one line of JSON, the daemon's page byte for byte. |
| `--acquisition-json` | Git input only: the `deep-diff-forge.git-acquisition.v0` identity of what Git returned. |

Git selections run Git with a fixed argv and stop it after 120 s (`TimedOut`, exit 3), so a
stalled filesystem or hook cannot hang the caller. A repository's own content filters (from its
local or worktree configuration and their includes) never run; the user's global and system
filters, such as Git LFS, stay on by decision, and nothing else they configure runs
([SECURITY.md](../../SECURITY.md#residual-risks-and-non-goals)). A filter name that is not UTF-8
refuses the acquisition (`InvalidSelection`, which exits 2 today, measured 2026-11-01).

The repository also decides which of its files Git calls binary: `-diff`, the `binary` macro or
a `diff=` driver with `binary = true`, in a `.gitattributes` (committed, staged or never added),
in `.git/info/attributes`, or in a file its own `core.attributesFile` names; or one NUL byte in a
text file. Up to 0.9.0 such a file's change was reviewed as a binary change, and no evidence rule
read it ([DDF-2026-006](../advisories/DDF-2026-006.md)). Since 0.10.0 every change Git gives as
binary is asked for once more with `--text`, the same selection and flags. When the answer is
the same change, as text, valid UTF-8 (NUL bytes included), within what is left of the 64 MiB
byte budget, and the parser reads it, it replaces the binary section: the two apply to the same
bytes. The file then carries the signal `diff_suppressed`, and `rank.v0` and
`git-acquisition.v0` say why (`"diff_suppressed": "attribute"` when a `diff` attribute is unset
or names a driver for the path, `"encoding"` for a UTF-16 or UTF-32 file, `"unread"` when it
could not be read again within the limits below, else `"content"`).
A file hidden by a `diff` attribute carries the label also when its text cannot be read back
(a byte that is not UTF-8, the byte budget): it then stays `binary_changed`. A UTF-16 or
UTF-32 file is labelled and not decoded, so its code evidence is not found; its byte order mark
is seen only when the change shows line 1, so wide text outside Latin-1 edited elsewhere, with
no mark in the change, is not labelled. No attribute is trusted or looked for first,
and attributes keep every other effect (line endings, the user's filters). Still read as binary,
with `binary_change` and no new label: a real binary (not UTF-8) that no attribute hides. A
second reading (or `git check-attr`) that the byte budget or the 120 s limit cuts off never
leaves a binary change silent and never fails the review: the file stays `binary_changed`, and
the attribute label stands, else it carries `"diff_suppressed": "unread"`. Paths are read again
in Git's order, each within the budget less every other section as it stands, so a large file
that does not fit is `unread` and the files after it are still read; once the time limit has
run out, every binary change not yet read again is `unread`. A binary by content whose text
form passes the 64 MiB budget is therefore labelled `unread`. The labels hold when the text
readings are dropped: if the whole patch with them does not parse (more than 1,000,000 lines,
say), the binary sections stand and a file whose text was read is `unread`, a known cause
stays (each binary change costs up to
two more Git calls, about 8 ms each: 300 changed images took 2.4 s instead of 0.25 s, measured
2026-11-22). `--stdin-patch` reviews the patch it is given: a binary section a
caller pipes in cannot be read again. A file pair (below) also shows a file with a NUL byte as
binary.

### Compare two files

`deep-diff-forge <old> <new>` reviews two regular files: Git writes the patch with
`git diff --no-index`, and DDF seals those exact bytes and prints any MODE above. Git runs from
`/` on the canonical paths, with no attributes file and no CRLF conversion, so neither a
repository around the files nor the user's attributes can run a filter or change the bytes
compared. File headers therefore name the canonical path without its leading `/`
(`a/home/me/old.rs`). Put `--` first when a path is also a command name
(`deep-diff-forge -- doctor notes.txt`).

| Refusal | Exit |
| --- | --- |
| A path that does not exist or cannot be inspected (`cannot read the old file: …`) | 3 |
| The path itself is a symbolic link, a directory, or a FIFO, socket or device (`old file refused: … is a directory, not a regular file`); a linked parent directory is resolved, not refused | 4 |
| Git cannot open a file it was given, as when it is removed after the checks (`` `git diff --no-index` exited with status 1: error: Could not access … ``) | 3 |
| Two files whose sizes together exceed the 64 MiB input budget, checked before Git reads them | 4 |
| Fewer than two paths, an unknown option, or conflicting modes | 2 |
| A first word that is neither a command nor an existing path, without `--` (`unknown command: …`) | 2 |

Directories are out of scope: a tree comparison brings nested repositories, links and special
files with it, which `--git-range` and the working-tree selections already handle for a
repository. The check and Git's read are separate steps, so a concurrent writer can still
change a file in between; compare files you control. Git's answer is checked too: exit 0 must
come with no patch and exit 1 with one, and anything on Git's stderr is a failure (exit 3), so a
file that vanishes in between is never reviewed as unchanged.

A gate should **always pass both guards**. When both fail, the files guard reports first.

## The review cockpit

```bash
git diff | deep-diff-forge review                    # interactive (needs a terminal)
git diff | deep-diff-forge review --side             # start side by side
git diff | deep-diff-forge review --probe            # one headless frame for CI and agents
git diff | deep-diff-forge review --probe --cols 120 --rows 40 --cmd rank
```

| Key | Action |
| --- | --- |
| `j` / `k` · `↓` / `↑` | next / previous file, in rank order |
| `g` / `G` · `Home` / `End` | first / last file |
| `h` / `l` · `←` / `→` | focus the file tree / the diff |
| `Enter` | open the selected file's diff (focus it), or run the selected palette command |
| `s` · `Tab` · `t` | toggle inline and side-by-side |
| `z` · `w` · `n` | fold context · wrap rows · show notes |
| `v` · `Space` | mark the file reviewed and move on |
| `T` | cycle theme (dark · midnight · mono) |
| `:` · `?` | command palette · key help |
| `Ctrl-d` / `Ctrl-u` · `PageDown` / `PageUp` | scroll the diff |
| `q` · `Esc` | quit · close the overlay |

The mouse works too: the wheel scrolls, and clicks select the file on the row clicked, focus
the diff when inside its pane and open menus; a click on a border or the status bar does
nothing. The palette (`:` in the app, or `--palette` / `--cmd NAME` headless) runs `rank`,
`outline`, `cluster`, `summary`, `notes`, `review`, `daemon`, `learning` and `maturity`
against the loaded review. An unknown name or a non-numeric size is a usage error (exit 2).

## Source files

```bash
deep-diff-forge semantic <path> [--json]       # top-level symbols with line ranges (semantic.v0)
deep-diff-forge highlight <path> [--color|--no-color]
deep-diff-forge structural <old> <new> [--json]   # token-level, reformat-aware (structural.v0)
```

Rust is supported today. Other extensions degrade explicitly to `fallback:UnsupportedLanguage`,
never to a guess. Parsing is byte- and node-budgeted, and a malformed file reports
`parsed_with_errors:N` instead of failing. Highlighting is **terminal-injection-safe**: source
text passes through a control-character sanitiser, so the only raw escapes are fixed colour codes.

## Machine surfaces

```bash
deep-diff-forge envelope -- <command…>          # any command as one ddf.result.v1 object
deep-diff-forge schema [result|exit-codes|signals]  # the envelope's JSON Schema, the registry, or the signal vocabulary
deep-diff-forge explain {<code> [--family engine|council] | signal <label>} [--json]
deep-diff-forge mcp                             # read-only MCP server over stdio
deep-diff-forge rpc                             # the daemon's JSON-RPC over stdio, no daemon
deep-diff-forge daemon {path|start [--foreground] [--log-format text|json]|health|status|metrics [--format json|prometheus] [--out PATH]|stop} [--socket PATH]
```

## Learning, analysis, deployment and diagnostics

```bash
deep-diff-forge learn status [--json]                 # local receipts, scores, trust verdict
echo '<receipt>' | deep-diff-forge learn record --stdin
deep-diff-forge analyze <patch-file> <base-dir> <candidate-dir>   # deep-diff-forge.analysis.v1
deep-diff-forge deploy status [--json]                # declared maturity and gate stack
deep-diff-forge deploy release [--json]               # per-target publication posture
deep-diff-forge deploy doctor [--json] [--socket PATH] # can this machine run the daemon and Git? exit 6 if not
deep-diff-forge doctor                                # runtime, cache, state and socket paths
deep-diff-forge --self-test
deep-diff-forge {claude-code-contract|chain-contract|cluster-contract|loom-contract}
```

`learn` is local-only. A receipt holds hashes, counts and timings, never a path or a source
line. Receipts live under `$XDG_STATE_HOME/deep-diff-forge/learning/` (an absolute path; a
relative one is ignored for `$HOME/.local/state`). An invalid receipt exits 4.

`analyze` checks that a patch turns the captured base directory into the candidate before it
attaches semantics.

`deploy release` is a declared snapshot. From 0.8.4 the source is published on GitLab, Gitea
and Forgejo, nothing goes to GitHub (`github` and `github-release` are `skipped`), the release
is the owner's signed tag and a local release, and crates.io is on hold.

DDF reads a patch as a plain `git apply` reads it in Git's default configuration; the
settings that change what Git writes are in
[SECURITY.md](../../SECURITY.md#git-settings-that-change-what-git-apply-writes).

## Review council

```bash
deep-diff-forge council {example|prepare|review|gate|status|import|export-tuicr|import-tuicr}
```

See [Review council and evidence gates](quality-gates.md#review-council-and-evidence-gates).

---

See also: [`CLAUDE_CODE_BASH_CLI`](../CLAUDE_CODE_BASH_CLI.md) · [`BASH_API_CONTRACTS`](../BASH_API_CONTRACTS.md).
