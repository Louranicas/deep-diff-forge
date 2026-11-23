# Diagnostics

`deep-diff-forge.diagnostics.v0` is new in 0.11.0. This page is the contract that the
builder and every surface that prints it (`--diagnostics`, the daemon method `diff.diagnostics`,
MCP `diagnose_patch`, the SDKs) are held to. `--diagnostics` prints it
([On the command line](#on-the-command-line)); the daemon method, MCP and the SDKs are still to
come, and until they land no other surface prints this schema.

## What a diagnostics document is

A diagnostics document locates every label that [risk ranking](risk-ranking.md#risk-ranking)
raises on one input: one diagnostic per evidence row, and one per label that no row locates. It
holds no text of the change except each file's path. It is built from the same sealed review as
`deep-diff-forge.rank.v0`, so it says nothing the ranking does not.

## The document

Members, in this order:

| Member | Meaning |
| --- | --- |
| `schema` | `deep-diff-forge.diagnostics.v0` |
| `input_sha256` | SHA-256 of the exact input bytes, as in `rank.v0` |
| `tool` | `{name, version, scorer_sha256}`: the engine that wrote this document |
| `judge` | `{state, ranker_sha256}`: `unknown`, `same`, `different` or `none_admitted`, and the SHA-256 of the ranker the policy admits, or null |
| `counts` | `{files, diagnostics, agent, person, cut_files}` |
| `cut` | `{path, rows_kept}` for each file whose located diagnostics were cut at 32, in file order |
| `diagnostics` | the diagnostics, in the order below |
| `texts` | `{id: sentence}` for every repair or substitute id the diagnostics name, sorted by id |

One diagnostic, members in this order:

| Member | Meaning |
| --- | --- |
| `key` | see Keys |
| `code` | `<label>@<anchor>`, from the table of codes |
| `label`, `class` | the label and its class: `security`, `integrity` or `shape` |
| `path` | the file's path: the only text of the change in the document |
| `anchor` | `line` when a row locates it; `file` when the label is raised and no row of this document locates it |
| `side`, `line` | `new` or `old` and the line number on that side; null for `file` |
| `occurrence` | 0, or the count of earlier diagnostics with the same path, label, anchor, side and line (a patch can repeat a section of one path) |
| `reading` | `current` when a current reading of this version finds it, or `floor_only` when only a reading kept for the floor finds it (see Readings) |
| `audience` | `agent`, or `person` for a `floor_only` reading |
| `repairs` | ids of the changes that answer the label |
| `not_accepted` | ids of substitutes that do not answer it and that this engine raises a label for |
| `test_likely` | only on `panic_path_added@line`: the hunk shows the panic inside an item marked as a test |
| `judge_sees` | only when `judge.state` is `different`: whether the admitted ranker raises this label in this file |

## Completeness and order

The pairs (path, label) of the diagnostics are exactly the pairs of `signals[]` in `rank.v0` for
the same input and scorer: none is added, none is left out. Every evidence row is a `@line`
diagnostic. A label raised from the path, the status or the modes, or one whose rows were cut, is a
`@file` diagnostic. Files come in `rank.v0` order. Inside a file come the rows in `rank.v0` order,
then `hazardous_text@line` diagnostics, one per changed line whose text carries the hazard (its
place only, never its text), then one `@file` diagnostic per remaining label in `signals[]`
order. A file keeps at most 32 located diagnostics, rows first; a file over that is listed in
`cut`. `hazardous_text` raised by a path alone, or whose lines found no place, is `@file`.

## Readings

To keep every release finding at least what the one before found, the engine reads Rust files
also as release 0.8.3 read them, and every file also as release 0.8.4 read it. A diagnostic's
`reading` says which kind of reader found it:

| Reader | Files | `reading` |
| --- | --- | --- |
| the readers of this version | every language | `current` |
| the reading of release 0.8.3 | Rust | `floor_only`, unless a current reader also finds it |
| the reading of release 0.8.4 | every language | `floor_only`, unless a current reader also finds it |

In a file that is not Rust, the readers of 0.8.3 are the current ones (their rules have not
changed), so what they find is `current`. A Rust `unsafe` in code is `current`, and so is
`#[allow(` named inside a Rust comment, which the current readers match on the raw line; an
`unsafe` inside a raw string over lines, which the scanner of release 0.8.3 took for code, is
found by the 0.8.3 reading only and is `floor_only`.

## Codes

A code names a label and how the diagnostic is anchored. A code's label and anchor never change,
and a code is never removed or reused while the schema is `v0`.

| Label | Class | Codes |
| --- | --- | --- |
| `public_api_surface` | shape | `@file` |
| `new_file` | shape | `@file` |
| `deleted_file` | shape | `@file` |
| `binary_change` | shape | `@file` |
| `diff_suppressed` | security | `@file` |
| `large_change` | shape | `@file` |
| `many_hunks` | shape | `@file` |
| `config_or_lockfile` | shape | `@file` |
| `test_only` | shape | `@file` |
| `generated_or_vendored` | shape | `@file` |
| `hazardous_text` | security | `@line`, `@file` |
| `symlink_change` | security | `@file` |
| `executable_bit` | security | `@file` |
| `unsafe_added` | security | `@line`, `@file` |
| `process_spawn` | security | `@line`, `@file` |
| `network_fetch` | security | `@line`, `@file` |
| `dynamic_eval` | security | `@line`, `@file` |
| `ci_privilege` | security | `@line`, `@file` |
| `build_hook` | security | `@line`, `@file` |
| `secret_like` | security | `@line`, `@file` |
| `check_suppressed` | integrity | `@line`, `@file` |
| `test_removed` | integrity | `@line`, `@file` |
| `test_skipped` | integrity | `@line`, `@file` |
| `trivial_assert` | integrity | `@line`, `@file` |
| `stub_added` | integrity | `@line`, `@file` |
| `error_swallowed` | integrity | `@line`, `@file` |
| `panic_path_added` | integrity | `@line`, `@file` |

A diagnostic's code is always its label and its own `anchor`; whether the reading is current or
kept for the floor is said by `reading` only. The list of codes is also kept in
`fixtures/diagnostics/codes.v0.txt`, which only grows.

## Keys

`key` is the lowercase hex SHA-256 of these UTF-8 fields, each followed by one NUL byte except the
last: `ddf-diagnostic-v0`, `input_sha256`, `path`, `label`, `anchor`, `side`, `line`, `occurrence`.
A null `side` or `line` is the empty string; numbers are written in decimal without leading zeros.
A key holds no line text and no hunk address: anyone can recompute it from the document, and it
reveals nothing the document does not already state. It is unique within one document and equal
across engines for the same input and location. It does not follow a finding across edits: the
next input has another `input_sha256`.

## Byte form

The document is written in the canonical form of `rank.v0`: `schema` first, members in the
orders above, the same bytes on every surface. Strings are escaped as `rank.v0` escapes them and,
in addition, every bidi control and invisible character is written as `\uXXXX`, so a path cannot
reorder or hide text where it is shown. The value of `path` equals the value in `rank.v0`.
`address` and `body_sha256` are not carried: both are hashes of the change's text.

## Two engines: who advises and who judges

`tool` names the engine that wrote the document. When the agent hooks judge a stop, a policy
decides which ranker may rank it, by SHA-256, and it can admit an older one
([agent hooks](agent-hooks.md)). `judge` says which, when the writer knows: `same`, `different`
(with `ranker_sha256`) or `none_admitted`. The command line, the daemon, MCP and the SDKs do not
run the judge and write `unknown`. With `different`, `judge_sees` says per diagnostic whether the
admitted ranker raises that label in that file on the same input; a diagnostic with `false` is
advice that the stop will not enforce. Text written for agents does not print `judge_sees`.

## For agents and for people

Text that reaches an agent (the post-tool advice, the text form of `--diagnostics`, MCP, `agent
check`, and `texts` here) holds the accepted repairs and only those substitutes for which this
engine raises a label, each in a sentence that names only a spelling a committed test patch
proves the engine raises. Substitutes that no label raises are listed for people:
`deep-diff-forge explain signal <label>` (its `audience` is `person`), this repository's reference
pages and `SECURITY.md`. `explain signal <label> --audience agent` prints what an agent is shown.
A `floor_only` reading has `audience: person`, no repairs and no substitutes; text for agents
counts these readings and asks that the text carrying them be left in place.

## On the command line

`--diagnostics` is an output mode of `--stdin-patch`, the Git selections and `<old> <new>`. With
`--json` it prints the document as the builder writes it; `schema diagnostics` prints its JSON
Schema. The input is refused before any document exactly as `--rank` refuses it: a patch that
does not parse, or, on stdin, input in which Git finds no patch, exits 4 (`--allow-empty` makes
that an empty document); `--require-files` and `--require-hunks` exit 7. The command runs no
judge, so `judge` is `{"state": "unknown", "ranker_sha256": null}`. On Git input a file Git gave
as binary and read again as text carries `diff_suppressed@file` whatever the cause; the cause
(`attribute`, `content`, `encoding` or `unread`) is in `rank.v0`, not here. The three inputs give
one document for one change, except `input_sha256` and the keys, which hash the patch bytes
each input makes, and, for `<old> <new>`, the paths, which are the two paths as given.

Without `--json` it prints a text form for agents. First one line per diagnostic whose
`audience` is `agent`, in document order, then for each file the count of its `floor_only`
readings and each file in `cut`, then each named id with its sentence, and the counts:

    "src/b.rs":2: unsafe_added@line (security, new side); repairs: unsafe_added:a1, unsafe_added:a2
    "src/a.rs": unsafe_added@file (security); repairs: unsafe_added:a1, unsafe_added:a2
    "src/app.rs": 1 readings kept for compatibility with 0.8.3 are for a person; leave the text that carries them
    "src/a.rs": 32 places listed; the rest were cut at 32
    unsafe_added:a1: <the sentence of the repair>
    counts: files 3, diagnostics 35, agent 34, person 1, cut_files 1

The path is quoted, with `\`, `"`, control, bidi and invisible characters escaped; it is the only
text of the change the text form prints. `judge_sees` is not printed.

## Paging

The daemon method `diff.diagnostics` (engine protocol 3, [the optional daemon](mcp-sdk-daemon.md#the-optional-daemon))
returns pages of at most `max_bytes` (1 024 to 16 777 216, default 65 536). A page holds whole
diagnostics. Each page repeats the header, the members from `schema` to `counts`, which holds
no path and no list and is at most 960 bytes with its `page` member. It holds its part of
`diagnostics`, the `cut` entries of the files whose first diagnostic is on that page, the
`texts` its diagnostics name, and `page`: `{first, count, total, next_cursor,
document_sha256}`. `document_sha256` is the SHA-256 of the whole document as `--diagnostics
--json` prints it: the first page's header without `page`, every page's `cut` entries in order,
every page's diagnostics in order and the union of `texts`, written in the canonical form, hash
to it. A cursor belongs to one input and one scorer; a cursor from another patch, another engine
or outside the document is refused. A `max_bytes` below the header and one diagnostic (with its
file's `cut` entry when it is that file's first) is refused and names that diagnostic; no page is
ever over `max_bytes` and no diagnostic is ever split. A client that asks for protocol 3 from a
daemon that speaks 2 gets error 2 and no document; the SDKs report that as "protocol too old" and
never build a document from an older daemon's ranking.

## Gates: `--fail-on`

`--rank --fail-on LIST` and `--diagnostics --fail-on LIST` take a comma-separated list of classes
(`security`, `integrity`, `shape`) and labels. The verdict is decided before the document is
printed, and the document is printed as without the flag. Then the command exits 7 when any
file's `signals[]` holds a selected label, or when the ranking cut any file's evidence list at
32 rows (the same bit under `--rank` and `--diagnostics`), with one `refused:` line per reason on
stderr (see [Exit codes](output-and-exit-codes.md#exit-codes)). A reader that closes stdout early
(`| head`) does not turn the refusal into a pass: the command still exits 7. It reads
`signals[]`, not the rows, so a label with no row, such as `hazardous_text`, or one whose rows
were pushed out of the list, still refuses.

On a patch read from stdin, a file given as binary is judged by its shape only, even when the
same patch adds the `.gitattributes` line that made Git print it so: it ranks `binary_change` and
no code label, and `--fail-on security` passes it. Use `--git-*`, which reads Git's attributes,
raises `diff_suppressed` (class security) and reads the file as text, or add `binary_change` to
the list.

## Stability

While the schema is `v0`: members are only added, and only as optional; readers ignore members
they do not know; codes and ids are only added. A version of the engine that finds more on the
same input changes `scorer_sha256`, not the schema. Removing, renaming or retyping a member, or
changing the key, the order, the completeness rule or a code's meaning, makes `v1`.

## What it does not say

- A label no reading raises is not in the document; an empty document never means the change is safe.
- Outside the hooks and `agent check`, it does not say which ranker will judge a stop.
- `key` does not follow a finding across edits.
- A substitute that no label raises passes every gate on this page; it is listed for people.
- A `floor_only` reading still refuses under `--fail-on`, because the ranking keeps it.
- On stdin, a file given as binary has no code labels (see Gates).
