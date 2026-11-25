# Command reference

`deep-diff-forge --help` is the authoritative list, and `deep-diff-forge <command> --help` prints one command's usage and what it does, with exit 0. A test runs it for every command `--help` lists.

## Review a change

```bash
deep-diff-forge --stdin-patch [MODE] [--require-files] [--require-hunks] [--allow-empty]
deep-diff-forge {--git-working-tree [--include-untracked] | --git-staged | --git-range A..B} [--repo PATH] [MODE | --acquisition-json]
deep-diff-forge [--] <old> <new> [MODE] [--require-files] [--require-hunks]
# MODE: --json | --jsonl | --rank [--fail-on LIST] | --diagnostics [--fail-on LIST]
#       | --cluster [--parallel serial|auto|N] | --layout inline|side-by-side
#       | --outline | --hunks [--address A]... [--path P]...   (both: [--max-bytes N] [--cursor C])
```

Choose one MODE: a second is refused with exit 2 before any input is read. `--json` also
selects the JSON form of `--rank`, `--diagnostics` and `--cluster` (and is accepted, redundantly, with
`--outline` and `--hunks`, which always print JSON). An option that takes one value is
refused when given twice; only `--address` and `--path` repeat.

| Mode | Output |
| --- | --- |
| *(none)* | One line per file: status, path, `+adds -dels`, hunk count. |
| `--json` | One `deep-diff-forge.review.v0` document: files, hunks, line anchors, summary, seal. |
| `--jsonl` | One `{"event":"diff.file",…}` event per file, streamed. |
| `--rank` | Files by review risk, highest first, with their signals. Add `--json` for `deep-diff-forge.rank.v0`. |
| `--diagnostics` | Every label of the ranking at the evidence row that locates it or at its file: a text form for agents (each path quoted and escaped, the line, the code, the vocabulary ids and their sentences), or with `--json` `deep-diff-forge.diagnostics.v0` ([Diagnostics](diagnostics.md#on-the-command-line)). The ranking is the one `--rank` prints and the input is refused as `--rank` refuses it. |
| `--cluster [--parallel serial\|auto\|N]` | The same ranking through bounded parallel lanes with a deterministic join and a receipt. Add `--json` for `deep-diff-forge.cluster.v0`. |
| `--layout inline` / `--layout side-by-side` | Text projections with old and new line numbers, as wide as the file's largest. A `\` note row follows what a row cannot show: a non-UTF-8 line shown lossily, `\ No newline at end of file`, and a removed/added pair whose printed text looks the same (a tab against the spaces it advances to, a literal `\x1b` against a raw `ESC`, a trailing or no-break space, or in side-by-side a change past the column width). Inline prints tabs raw and judges them by 8-column tab stops; side-by-side shows a tab as four spaces. Letters that only look alike (Cyrillic `а` against Latin `a`) are not noted. |
| `--require-files` | Refuse a patch with 0 files: exit 7, empty stdout, `refused: 0 files in input (--require-files)`. |
| `--require-hunks` | Refuse a patch with 0 hunks or 0 changed lines (header-only, rename-only, context-only): exit 7, `refused: 0 hunks in input (--require-hunks)`. |
| `--fail-on LIST` | With `--rank` or `--diagnostics` only (any other mode: exit 2, `error: --fail-on needs --rank or --diagnostics`), on `--stdin-patch`, the Git selections and `<old> <new>`. A gate on the ranking, with one verdict under both modes: `LIST` is comma-separated, no spaces, each token a class (`security`, `integrity`, `shape`) or a label `deep-diff-forge schema signals` lists; any other token, an empty one included, is exit 2 before the input is read. The ranking or the diagnostics are printed as without the flag; then the command exits 7 when any file's `signals[]` holds a selected label (`refused: N files carry a label of LIST (--fail-on)`) or when any file's evidence list was cut at 32 rows, whatever the list (`refused: the evidence of N files was cut at 32 rows (--fail-on evidence_truncated)`), each reason line followed by the files it names, and exits 7 even when the reader closed stdout. It reads `signals[]`, not the evidence rows, so a label with no row (`hazardous_text`) or one whose rows were cut still refuses. On a patch read from stdin, a file given as binary is judged by its shape only; use `--git-*`, which reads Git's attributes and raises `diff_suppressed`, or add `binary_change` to `--fail-on`. |
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
with `binary_change` and no new label: a change no attribute hides whose second reading is
not UTF-8, on a path of no language the readers read, and either holds more than 128 odd bytes
per KiB of what it shows (bytes UTF-8 decoding replaces, and controls other than tab, carriage
return and form feed), a real binary most often, or gives no code evidence (since 0.11.0,
S3.27.h: a reading under that bound, text with a few stray bytes, is decoded lossily and read
for code evidence beside the binary change, and a file it gives evidence for carries
`"diff_suppressed": "content"`, as does every such change on a path of a language the readers
read, whatever its share of odd bytes; nothing of the decoding reaches a document but positions
and labels). A
second reading (or `git check-attr`) that the byte budget or the 120 s limit cuts off never
leaves a binary change silent and never fails the review: the file stays `binary_changed`, and
the attribute label stands, else it carries `"diff_suppressed": "unread"`. A `git check-attr`
that fails in another way (its 64 KiB output cap included), or a path that cannot be passed to
it, counts as an attribute (`"diff_suppressed": "attribute"`). Paths are read again
in Git's order, each within the budget less every other section as it stands, so a large file
that does not fit is `unread` and the files after it are still read; once the time limit has
run out, every binary change not yet read again is `unread`. A binary by content whose text
form passes the 64 MiB budget is therefore labelled `unread`. A binary section stays
unlabelled only when no attribute covers it and Git's second reading of it succeeded, is
not valid UTF-8 (nor UTF-16 or UTF-32), and is either over the bound above or gives no code
evidence when read lossily, and its path names no language the readers read. A change Git
pairs with another as a copy (two changes in one reading) is not read lossily, and a text
drowned in stray bytes (more than one in eight of what the change shows) is read as bytes: its
lines stay unread, a stated limit; on a source path it is labelled `"content"`. Every other
outcome that does not end in a used text section keeps a known cause or is `unread`: a reading
Git failed or cut, and a valid UTF-8 reading not used for any reason (the parser refused it
alone, more than 1,000,000 lines say; Git answered it as more than one change, a copy under
the repository's `diff.renames=copies` say; or the whole patch with the readings did not
parse) (each binary change costs up to
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
deep-diff-forge schema [result|exit-codes|signals|diagnostics]  # the envelope's JSON Schema, the registry, the signal vocabulary, or the JSON Schema of diagnostics.v0
deep-diff-forge explain {<code> [--family engine|council] | signal <label> [--audience agent|person]} [--json]
deep-diff-forge mcp [--allow-repo PATH]...      # read-only MCP server over stdio; --allow-repo lets review_change read those repositories
deep-diff-forge rpc [--allow-repo PATH]...      # the daemon's JSON-RPC over stdio, no daemon; --allow-repo as for mcp
deep-diff-forge agent setup --client claude|codex [--allow-repo PATH]... [--hooks [--judge PATH] [--policy NAME] [--ranker-dir DIR]] [--json]  # print the MCP registration and the hooks, never written (agent-hooks.md)
deep-diff-forge agent doctor [--gate [--judge PATH] [--policy NAME] [--ranker-dir DIR]] [--json]  # the agent surfaces agree; --gate: every link a judged stop needs
deep-diff-forge agent hook session-start|post-tool|stop --client claude [--judge PATH] [--policy NAME] [--ranker-dir DIR]  # the hook commands agent setup --hooks prints
deep-diff-forge agent check [--session ID | --base REV] [--range A..B] [--judge PATH] [--policy NAME] [--ranker-dir DIR] [--require-pass] [--json]  # the units a stop judges, judged with its judge and policy (agent-hooks.md)
deep-diff-forge agent verify-branch BASE..HEAD [--json]  # every commit of the range passed by a stop in the project's ledger, else exit 7 (agent-hooks.md)
deep-diff-forge agent policy-accept --commit C [--policy NAME] [--judge PATH] [--yes-from-pipe]  # a person's acceptance of the policy at C, a local convention, not authentication (agent-hooks.md)
deep-diff-forge daemon {path|start [--foreground] [--allow-repo PATH]... [--log-format text|json]|health|status|metrics [--format json|prometheus] [--out PATH]|stop} [--socket PATH]
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

`deploy release` is a declared snapshot. From 0.10.0 every target is `published`: the owner's
signed tag and a local release on GitLab (the place of release), Gitea and Forgejo; GitHub as a
mirror of the source (`github`) and of the release page (`github-release`); the crates on
crates.io from the tag. 0.8.4 and 0.9.0 declared `github` and `github-release` `skipped` and
crates.io `blocked`.

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
