# Conformance of the patch parser to Git

The rule of this crate is that the patch `git apply` applies is the truth, and nothing may
show it otherwise. Five independent assessments in a row (v0.8.0 to v0.8.3) each found one
more header shape that DDF read one way and `git apply` wrote another way. From the release
after 0.8.4 on, the parser does not try to read every shape as Git reads it. It reads the
header shapes Git's own diff writes, and it refuses the others
(`PatchParseError::HeaderNotWrittenByGit`).

## What is promised

What DDF promises is what a patch alone can settle. For every patch DDF accepts, a plain
`git apply` in Git's default configuration does one of two things:

- it refuses the whole patch while checking it, and writes nothing; or
- it writes exactly the paths, types and modes the review shows and, for each hunk, removes and
  adds exactly the lines (as Git stores them) the review shows, at the place Git's own anchor
  rule gives:
  - a hunk with no trailing context is placed at the end of the file;
  - a hunk at old line 0 or 1 is placed at the start of the file (with both, it is the whole
    file);
  - any other hunk is placed by Git's search: from the new-side line its header names (`+c`)
    outward (that line, one after, one before, two after, …), over lines no earlier hunk of the
    patch wrote, at the first place its context and removed lines are found. The review shows
    the hunk's lines and the header's numbers as the patch's claim; it does not say which of
    several equal places Git takes (row `hunk-anchor`).

The line numbers in a hunk header and the text after `@@` are the patch's own claims. On the
tree Git made the patch from they are true. A patch written by hand can name one occurrence of
some lines while Git changes another: on a file whose end repeats them, `@@ -2,2 +2,2 @@` with
leading context only changes the last copy (row `hunk-anchor`). So the review a person reads
states the anchor: the inline and side-by-side layouts and the TUI's hunk header say "Git places
this hunk at the end of the file" (or at the start, or on the whole file). The machine surfaces
(`review.v0`, `review-hunks.v0`) carry the lines the anchor follows from, and the outline
(`review-outline.v0`) carries ranges documented as the patch's claim (`docs/API_AND_IPC.md`;
the JSON carries no mark).
Checking the place against the tree, when DDF has one, is not built.

A review never shows one change while Git writes another. It may show a change that never
lands, because DDF reads a diff without the tree it applies to: a hunk that fits no place Git's
rule allows, or a file that already exists, makes Git refuse the whole patch.

To keep that, DDF refuses what Git would place or write otherwise than the review shows:

- **A hunk Git places by an anchor its header does not set** (`HunkPlacedElsewhere`). Plain
  `git apply` must place a hunk with no trailing context at the end of the file, and one at old
  line 0 or 1 at its start, or refuse the patch (`apply_one_fragment`, `apply.c` 3146–3173:
  without `-C` it tries no other place). A hunk with no context past line 1 then lands at the
  end of the file, and a hunk after one with no trailing context cannot be where it says.
  `git diff -U0` writes such hunks: Git-written, and refused (`git apply --unidiff-zero` is
  their reader). DDF's own `git diff` passes `-U3`, whatever `diff.context` says. A hunk with
  no context at line 1 is read: Git writes it only when it is the whole file, and else refuses
  the patch (`git diff -U0` writes one for an insertion after line 1; Git refuses it whole).
- **Read though no Git writes it: a creation whose hunks all start at line 0.** The accept-list
  is about header shapes; this is a layout of hunks. Git writes the first hunk into the empty
  file, then cannot anchor the second at line 0 (`match_beginning` with `match_end` wants an
  empty file) and refuses the whole patch while checking it, writing nothing. Nothing is placed
  elsewhere, so C1 holds through its second outcome, and the reading 0.8.4 gave a created file's
  later hunks stays reachable. A creation's later hunk at any other line is refused as above.
- **What `git apply` fails to write once it has begun** (`GitCannotWrite`). It removes what the
  patch deletes, then writes (`write_out_results`). It fails on a symlink whose target is
  empty, holds a NUL or is longer than 4,095 bytes, and on one path that is both a file and the
  directory of another: two paths the patch writes, or a copy's source, which the patch keeps
  as a file unless a section removes it, and a path written under it or at its directory. A
  symlink in a binary body has a target DDF cannot read. These are refused.

"Git writes it" is not the same as "`git apply` accepts it". C1 allows any refusal of the whole
patch. Among those, the generated differential counts three presentations Git's own diff
writes, which DDF reads as 0.8.4 did:

- **A binary marker with abbreviated ids.** A plain `git diff` of a binary file writes
  `Binary files … differ` after an `index` line with abbreviated ids. `apply_binary` wants full
  ids ("cannot apply binary patch … without full index line", `apply.c` 3315). With
  `--full-index` or `--binary`, the same change applies.
- **The inner section of a `--submodule=diff` envelope.** `git diff --submodule=diff` writes a
  `Submodule sm …` line, then the submodule's own section under the superproject path. In the
  repository it describes, Git refuses that section.
- **`diff --git` halves Git cannot split.** These are halves that name two paths, or that Git
  cannot split into one name (a trailing space or tab, mixed quoting, a repeated ` b/`), with no
  `---`/`+++`, rename or copy header. `git diff --no-index` of two binary files, or of two that
  differ only in mode, writes such halves. Git names no file ("lacks filename information").

The list is not exhaustive. Options that change what is shown also write patches Git refuses
whole: `-D`/`--irreversible-delete` ("removal patch leaves file contents"), textconv output, and
`-w`, `-b` or `-I` output whose context no longer fits. `tests/conformance.rs` asserts each of
the three against real Git output: Git refuses the whole patch and the tree is unchanged. The
generated differential counts each one apart and fails on anything else Git refuses to read.

Limits of the promise:

- `git apply --reject`, which applies a patch in part, is outside it.
- **A failure while writing that the patch does not determine** (permissions, a full disk, a
  file-system limit, or a file, or a directory with other files, already in the tree where the
  patch writes: a patch that deletes `y` and creates `x/b` on a tree holding a file `x`) can
  leave the tree half-written: Git has removed what the patch deletes before it writes. That is
  Git's behaviour, not a reading of the patch; the tree decides it
  (`a_partial_write_the_tree_decides_is_outside_the_promise`).
- **Another tree.** The review states Git's rule, not a line of the tree Git is given. On a
  tree other than the patch's base, the place Git's search finds ("offset"), or the copy at the
  end of the file, can be another occurrence of the same lines than the one the header names.
  The `@@` section text is the patch's own claim.
- **Attributes in the target tree.** `.gitattributes` there (`eol`, `ident`,
  `working-tree-encoding`) makes `git apply` write the work-tree form, not the bytes Git stores
  and the review shows.
- **A non-default configuration.** With `core.fileMode=false`, and without `--index`, Git
  writes the mode an `index` line names: `check_preimage`, `apply.c` 3919–3937, takes the old
  mode from the patch when it does not trust the executable bit. Leaving the `index` line out
  then changes the mode written (independent review of S3a, S3A-L-2). The patch does not carry
  the repository's configuration, so no reading of it can close this.

This file is the table of what Git reads. It holds one row for each header line `git apply`
recognises in a `diff --git` section, and one for the traditional `---`/`+++` form. Each row
gives:

- the form;
- what Git sets from the line;
- what Git does when the line is repeated;
- what Git refuses;
- whether Git's diff writes the line;
- what DDF 0.8.4 did;
- what DDF does now.

Sources: Git 2.56.1 (`03b41523c576`), `apply.c` and `diff.c`, cited by function and line. The
toolbox that runs the tests has Git 2.56.0. Between the two versions, `apply.c` differs only in
`parse_binary_hunk`, which has a size limit and frees memory on error. `diff.c` differs only in
how the base85 lines are emitted. The header machinery is the same.

Each row has a test of its name in `tests/conformance.rs`. Each test runs the row's forms
through DDF, through `git apply --numstat --summary` and through a real `git apply` into a
scratch repository with a seeded object store, and asserts what the row states. A further test
fails if a row has no test or a test names no row.

## The shapes Git writes (the accept-list)

`builtin_diff` (`diff.c` 3805) writes `diff --git`, then one of these:

- `new file mode` (3882) or `deleted file mode` (3889);
- `old mode` and `new mode` (3897), when the mode changes.

Then it writes `fill_metainfo`'s lines (`diff.c` 4823):

- `similarity index`, `rename from`, `rename to` (4852);
- `similarity index`, `copy from`, `copy to` (4842);
- `dissimilarity index` for a rewrite under `-B` (4864);
- then, when the blob changed, `index <old>..<new>`. The ` <mode>` follows only when the mode
  did not change (4893).

The body comes last, from `builtin_diff`:

- `---`/`+++` and hunks;
- `Binary files … differ` (3945, 3969);
- `GIT binary patch` (1580, with `--binary`);
- nothing at all: an empty file, a pure rename or copy, or a mode change;
- `index` and nothing after it, when `-w`, `-b`, `-I`, `--ignore-blank-lines` or a textconv
  driver hides every hunk of a rename, copy or mode change: the header is still written
  (`must_show_header`).

A change of type, such as a file that becomes a symlink, is written as a deletion followed by
a creation (`run_diff` 5017).

These are 42 header shapes in all:

- 4 shapes create a file;
- 4 shapes delete one;
- 5 shapes have no `index` line and no body;
- 5 shapes have an `index` line and no body (a rename, copy or mode change under `-w`);
- 24 shapes have an `index` line and a body.

They are the `GIT_WRITES` list in `src/parser.rs`. A section is read only if its headers are one
of these shapes. A `similarity index`, `dissimilarity index` or `index` line may be left out of
the shape, except that a binary line may not come straight after `diff --git`: Git reads no
patch from that, and leaves the line's name for the next section (F3 of the v0.8.2 rescore).
In Git's default configuration, none of these lines changes what `git apply` writes:

- a score only sets the percentage `--summary` prints;
- without `index`, Git applies a text section alike, and it refuses a binary one ("cannot apply
  binary patch … without full index line", `apply_binary`, `apply.c` 3315).

DDF's own rendering of a traditional diff has no `index` line. Its rendering of a rename that
it found from the names has no `similarity index` line.

Measured on 2026-11-16 with Git 2.56.1, and counted again on 2026-11-18 from the same patch
files (the same numbers). The patches came from `git diff` (plain, `-M -C -B`,
`--binary`, and more for the third repository), `git format-patch --stdout` and `git log -p
--first-parent`, run for every commit of each repository:

| Repository | Commits | Patches | `diff --git` sections | Shapes |
|---|---|---|---|---|
| DDF, all refs | 892 | 4,457 | 57,038 | 12 |
| forge-warden `hub.git`, all refs | 772 | 3,857 | 29,301 | 9 |
| A scratch repository built to give every row (symlinks, submodules, executables, empty and binary files, renames and copies with edits and mode changes, type changes, rewrites, quoted names) | 55 | 866 | 1,071 | 37 |

Every shape the first two repositories gave is among the scratch repository's 37. The 37 are
exactly the shapes the reading of `diff.c` above predicts for these option sets. The five with
`index` and no body come from `-w` and the like; an independent sweep found them (227 patches,
S3B-M-2), and a test reads them from real `git diff -w` output. No line differed from the forms
in the table below.

The traditional form (`---`/`+++` with no `diff --git`) is not written by Git, but by
`diff -u`. It is read as before, under the rules of its row.

## The table

| Row | Form | Git sets (apply.c) | Repeated | Git refuses | Git writes it (diff.c) | DDF 0.8.4 | DDF now |
|---|---|---|---|---|---|---|---|
| `diff-git` | `diff --git a/P b/P` | Opens a section (`find_header` 1646). The default name is the path both halves give (`git_header_name` 1197). With no header line after it, the line is no patch (`git_hdr_len <= len`, 1654). | Opens the next section. | No name from it or from any other line: "lacks filename information" (1462). | Always, first (`builtin_diff` 3879). | Read; refused with nothing after it. With halves that name two paths and no `---`/`+++`, rename or copy header, the rightmost ` b/` named the file, so a creation or a deletion was reviewed that Git refuses (found by the generated differential). | Same for a file created or deleted under such halves, which no Git writes: refused, as Git refuses it. **Limit:** any other section whose halves Git cannot split, with nothing else naming the files, is read as 0.8.4 read it (the rightmost ` b/`), and Git refuses it whole. Such halves name two paths, or have a trailing space or tab, mixed quoting or a repeated ` b/`. `git diff --no-index` of two binary files, or of two that differ only in mode, writes them. See "What is promised". |
| `old-mode` | `old mode M` | The old mode, canonicalised (`gitdiff_oldmode` 1023, `parse_mode_line` 1006). | The last wins. | Not octal: "invalid mode" (1017). A section whose only change is an unchanged mode: "patch with only garbage" (2344). | Before `new mode`, when the mode changes (3897). | Read. A repeat was refused. Any octal mode with a type Git writes was taken, such as `100664`. | Only right after `diff --git` and before `new mode`, and only with `100644`, `100755`, `120000` or `160000`. |
| `new-mode` | `new mode M` | The new mode (`gitdiff_newmode` 1031). | The last wins. Also overrides `new file mode`. | As `old mode`. | After `old mode` (3898). | Read. A repeat, or one beside `new file mode`, was refused. | Only right after `old mode`, with a mode Git writes. |
| `deleted-file-mode` | `deleted file mode M` | A deletion with that old mode. The old name is the default name (`gitdiff_delete` 1039). | The last mode wins. | With `new file mode`, `rename` or `copy`: "inconsistent header lines" (`check_header_line` 1347). A deletion with content left: "still has contents" (1982). | For a deleted file (3889). | Read. A repeat was refused. | Only right after `diff --git`, with a mode Git writes. |
| `new-file-mode` | `new file mode M` | A creation with that new mode. The new name is the default name (`gitdiff_newfile` 1049). | The last mode wins. | As `deleted file mode`. A creation with old lines: "depends on old contents" (1980). | For a created file (3882). | Read. A repeat, or one beside `new mode`, was refused. A mode Git reads as a submodule (`644`) was refused. | Only right after `diff --git`, with a mode Git writes. |
| `copy-from` | `copy from P` | A copy; its source (`gitdiff_copysrc` 1059). | The last wins. | With a rename, a creation or a deletion (1347). A path such as `..` or `.git` (`check_unsafe_path` 4059). | After `similarity index` (4844). | Read. A repeat naming another path was refused. | Only after `similarity index` (which may be left out), before `copy to`. |
| `copy-to` | `copy to P` | The path written (`gitdiff_copydst` 1069). | The last wins. | As `copy from`. | After `copy from` (4847). | Read; the last one named the path. | Only once, right after `copy from`. |
| `rename-from` | `rename from P` | A rename; its source (`gitdiff_renamesrc` 1079). | The last wins. | As `copy from`. | After `similarity index` (4854). | Read. A repeat naming another path was refused. | Only after `similarity index` (which may be left out), before `rename to`. |
| `rename-to` | `rename to P` | The path written (`gitdiff_renamedst` 1089). | The last wins. | As `copy from`. | After `rename from` (4857). | Read; the last one named the path. A `rename to` before its `rename from` was read. | Only once, right after `rename from`. |
| `rename-old` | `rename old P` | As `rename from` (the same handler, 1405). | The last wins. | As `rename from`. | No (an old Git wrote it). | Refused. | Refused, as before. |
| `rename-new` | `rename new P` | As `rename to` (1406). | The last wins. | As `rename to`. | No. | Refused. | Refused, as before. |
| `similarity-index` | `similarity index N%` | The score, if N is at most 100. Any other number is ignored (`gitdiff_similarity` 1099). Nothing that is applied. | The last wins. | Nothing. | First of a rename's or a copy's lines, N from 0 to 100 (4842, 4852). | Read anywhere among the extended headers, any text after it. | Only before `rename from` or `copy from`, N a whole number from 0 to 100. |
| `dissimilarity-index` | `dissimilarity index N%` | As `similarity index` (1109). | The last wins. | Nothing. | For a rewrite (`-B`), before `index` (4864). | Read anywhere, beside a rename too. | Only right after the mode lines (or `diff --git`), before `index`. |
| `index` | `index A..B` or `index A..B M` | The two ids, and M as the old mode (`gitdiff_index` 1119). A line with no `..` sets nothing. For a binary section the ids must be full (3315). | The last wins, the mode included. | Nothing at this line. A binary section without full ids is refused when it is applied (3315). | When the blob changed. ` M` only when the mode did not (4893). | Read anywhere among the extended headers, any text. Its mode was held to the object types Git writes (a file, a symlink, a submodule), and refused beside a mode line. | Only where Git writes it: last of the extended headers, with the mode exactly when there is no mode line. The ids are lowercase hex digits, 1 to 64 of them (Git writes 4 or more; a shorter id changes nothing Git writes). |
| `old-name` | `--- P` (in a Git section) | Checks the old name against the one already known, or sets it (`gitdiff_oldname` 988, `gitdiff_verify_name` 941). | Checked again; a different name: "inconsistent old filename". | A name that differs, or `/dev/null` beside a known name. | Before the first hunk (`emit_rewrite_diff` 1874, and the first hunk of a text diff). | Read and checked as Git checks it. A second pair was refused. A pair with no hunk after it was read in a new or deleted file. | Same, and only as the last header before the hunks, with a hunk after it (Git writes an empty file as `new file mode` and `index`). |
| `new-name` | `+++ P` (in a Git section) | As `---`, for the new name (`gitdiff_newname` 997). | As `---`. | As `---`. | Right after `---`. | As `---`. | As `---`. |
| `hunk-header` | `@@ -a,b +c,d @@` | Ends the header (`gitdiff_hdrend` 922). The hunks are read from here (`parse_single_patch` 1924). | Each hunk follows the last. | A hunk that changes nothing (`parse_fragment` 1772). A hunk with no header before it: "patch fragment without header" (1633). | Only after `---`/`+++`. | Read. A first hunk with no `---`/`+++` before it was read. | Read only after `---`/`+++`. |
| `hunk-placement` | A hunk's place: `@@ -a,b +c,d @@` and its context lines | With no trailing context the hunk's old lines must end the file (`match_end`); at old line 0 or 1 they must start it (`match_beginning`); with neither, Git searches outward from line `c` (`c`, `c`+1, `c`−1, …) over lines no earlier hunk wrote (`LINE_PATCHED`) and takes the first place they are found (`find_pos`). An anchored hunk that does not match there fails: Git tries elsewhere only under `-C` (`apply_one_fragment` 3146–3173). | Each hunk in turn, on the file as the hunks before it left it. | A hunk whose lines are not found at its anchor, or nowhere. | At the default (`-U3`), a hunk lacks trailing context only at the end of the file, and leading context only at its start. `git diff -U0` writes hunks with no context. | Read. A `-U0` hunk was shown at its header's line, and Git placed it at the end of the file (S3B-H-1). | A hunk with no context past line 1, and a hunk after one with no trailing context (except a creation's hunks at line 0, after which Git refuses the patch), are refused (`HunkPlacedElsewhere`). A hunk at line 1 with no context is read: Git writes it only when it is the whole file, and else refuses the patch. |
| `hunk-anchor` | A last hunk with leading context and no trailing context, whose header names an earlier copy of its lines | `match_end`: the hunk is placed at the end of the file whatever line its header names (`find_pos`, `apply.c` 2865–2868). | As `hunk-placement`. | A file whose end does not hold the hunk's old lines: the whole patch. At line 1 with no trailing context, any file longer than the hunk. | At the end of its base, always; the header is then true. A hand-written header can name an earlier copy. | Read; the header's line was shown as the place. | Read, and the review says "Git places this hunk at the end of the file" (or "at the start of the file", "on the whole file"); line numbers are the patch's claim. Hunks with neither anchor are read and placed by Git's search: one that skips a copy an earlier hunk wrote, the later of two equally near copies, and a `+c` the `-a` and earlier counts do not give (Git searches from `+c`). |
| `other-line` | Any other line | Ends the header (`gitdiff_unrecognized` 1159). The patch is whatever the headers so far say. | Not applicable. | "patch with only garbage" if no hunk, no binary line and no change in the headers (2344). | Not inside a header (a blank line or the next commit comes after a section). | The header ended there; a later header or hunk was refused. | Same. |
| `git-binary-patch` | `GIT binary patch` | A binary patch. `literal`/`delta` hunks follow (`parse_chunk` 2302, `parse_binary` 2154). | Not applicable. | Not exactly this line: no binary patch. No `literal`/`delta` line after it: "unrecognized binary patch" (2181). | With `--binary` (1580). | Read, as a marker ending the header. With a carriage return it was read too. | Only as the last line of a shape Git writes. With a carriage return it is refused. |
| `binary-files` | `Binary files A and B differ` | A binary section; Git writes the blob of the `index` line, or empties the file for a null id (`parse_chunk` 2319, `apply_binary` 3299). | Not applicable. | A line ending in a carriage return is not the marker. | For a binary file without `--binary` (3945, 3969). | Read. With a carriage return it was read as the marker too, so `old mode`/`new mode`, `index`, then the line was reviewed as a binary change while Git changed only the mode. | Only as the last line of a shape Git writes. With a carriage return it is refused. |
| `files-differ` | `Files A and B differ` | As `Binary files … differ` (`parse_chunk` 2322). | Not applicable. | As `Binary files … differ`. | No. | Read as `Binary files … differ` (v0.8.3 ignored it: G1). | Refused. |
| `traditional` | `--- P` then `+++ Q`, with no `diff --git`, then a hunk | A traditional patch (`find_header` 1661, `parse_traditional_patch` 868): the names, `/dev/null` and epoch timestamps give a creation or a deletion. | Each pair opens a file. | With no hunk after it: not a patch. | No: `diff -u` writes it. | Read. | Read, as before. |
| `no-line-feed` | A header line with no line feed, at the end of the input | Not read: the header loop stops (`parse_git_diff_header` 1417). | Not applicable. | The section as the earlier lines say. | No. | Refused. | Refused, as before. |
