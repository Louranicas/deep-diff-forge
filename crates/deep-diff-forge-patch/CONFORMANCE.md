# Conformance of the patch parser to Git

The rule of this crate is that the patch `git apply` applies is the truth, and nothing may
show it otherwise. Five independent assessments in a row (v0.8.0 to v0.8.3) each found one
more header shape that DDF read one way and `git apply` wrote another way. From the release after
0.8.4 on, the parser does not try to read every shape as Git reads it. It reads the shapes Git's own diff
writes, and it refuses the others (`PatchParseError::HeaderNotWrittenByGit`).

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
- nothing at all: an empty file, a pure rename or copy, or a mode change.

A change of type, such as a file that becomes a symlink, is written as a deletion followed by
a creation (`run_diff` 5017).

These are 37 header shapes in all:

- 4 shapes create a file;
- 4 shapes delete one;
- 5 shapes have no `index` line and no body;
- 24 shapes have an `index` line and a body.

They are the `GIT_WRITES` list in `src/parser.rs`. A section is read only if its headers are one
of these shapes. A `similarity index`, `dissimilarity index` or `index` line may be left out of
the shape, except that a binary line may not come straight after `diff --git`: Git reads no
patch from that, and leaves the line's name for the next section (F3 of the v0.8.2 rescore).
None of these lines changes what `git apply` writes:

- a score only sets the percentage `--summary` prints;
- without `index`, Git applies a text section alike, and it refuses a binary one ("cannot apply
  binary patch … without full index line", `apply_binary`, `apply.c` 3315).

DDF's own rendering of a traditional diff has no `index` line. Its rendering of a rename that
it found from the names has no `similarity index` line.

Measured on 2026-11-16 with Git 2.56.1. The patches came from `git diff` (plain, `-M -C -B`,
`--binary`, and more for the third repository), `git format-patch --stdout` and `git log -p
--first-parent`, run for every commit of each repository:

| Repository | Commits | Patches | `diff --git` sections | Shapes |
|---|---|---|---|---|
| DDF, all refs | 892 | 4,457 | 57,038 | 12 |
| forge-warden `hub.git`, all refs | 772 | 3,857 | 29,301 | 9 |
| A scratch repository built to give every row (symlinks, submodules, executables, empty and binary files, renames and copies with edits and mode changes, type changes, rewrites, quoted names) | 55 | 866 | 1,071 | 37 |

Every shape the first two repositories gave is among the scratch repository's 37. The 37 are
exactly the shapes the reading of `diff.c` above predicts. No line differed from the forms in
the table below.

The traditional form (`---`/`+++` with no `diff --git`) is not written by Git, but by
`diff -u`. It is read as before, under the rules of its row.

## The table

| Row | Form | Git sets (apply.c) | Repeated | Git refuses | Git writes it (diff.c) | DDF 0.8.4 | DDF now |
|---|---|---|---|---|---|---|---|
| `diff-git` | `diff --git a/P b/P` | Opens a section (`find_header` 1646). The default name is the path both halves give (`git_header_name` 1197). With no header line after it, the line is no patch (`git_hdr_len <= len`, 1654). | Opens the next section. | No name from it or from any other line: "lacks filename information" (1462). | Always, first (`builtin_diff` 3879). | Read; refused with nothing after it. With halves that name two paths and no `---`/`+++`, rename or copy header, the rightmost ` b/` named the file, so a creation or a deletion was reviewed that Git refuses (found by the generated differential). | Same for a file created or deleted under such halves, which no Git writes: refused, as Git refuses it. **Limit:** `git diff --no-index` of two binary files, or of two files that differ only in mode, writes such halves with nothing else naming the files. Git's own `git apply` refuses that output, and DDF reads it as the pair it compares (as 0.8.4 did). Git writes nothing from it. |
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
| `old-name` | `--- P` (in a Git section) | Checks the old name against the one already known, or sets it (`gitdiff_oldname` 988, `gitdiff_verify_name` 941). | Checked again; a different name: "inconsistent old filename". | A name that differs, or `/dev/null` beside a known name. | Before the first hunk (`emit_rewrite_diff` 1874, and the first hunk of a text diff). | Read and checked as Git checks it. A second pair was refused. | Same, and only as the last header before the hunks. |
| `new-name` | `+++ P` (in a Git section) | As `---`, for the new name (`gitdiff_newname` 997). | As `---`. | As `---`. | Right after `---`. | As `---`. | As `---`. |
| `hunk-header` | `@@ -a,b +c,d @@` | Ends the header (`gitdiff_hdrend` 922). The hunks are read from here (`parse_single_patch` 1924). | Each hunk follows the last. | A hunk that changes nothing (`parse_fragment` 1772). A hunk with no header before it: "patch fragment without header" (1633). | Only after `---`/`+++`. | Read. A first hunk with no `---`/`+++` before it was read. | Read only after `---`/`+++`. |
| `other-line` | Any other line | Ends the header (`gitdiff_unrecognized` 1159). The patch is whatever the headers so far say. | Not applicable. | "patch with only garbage" if no hunk, no binary line and no change in the headers (2344). | Not inside a header (a blank line or the next commit comes after a section). | The header ended there; a later header or hunk was refused. | Same. |
| `git-binary-patch` | `GIT binary patch` | A binary patch. `literal`/`delta` hunks follow (`parse_chunk` 2302, `parse_binary` 2154). | Not applicable. | Not exactly this line: no binary patch. No `literal`/`delta` line after it: "unrecognized binary patch" (2181). | With `--binary` (1580). | Read, as a marker ending the header. With a carriage return it was read too. | Only as the last line of a shape Git writes. With a carriage return it is refused. |
| `binary-files` | `Binary files A and B differ` | A binary section; Git writes the blob of the `index` line, or empties the file for a null id (`parse_chunk` 2319, `apply_binary` 3299). | Not applicable. | A line ending in a carriage return is not the marker. | For a binary file without `--binary` (3945, 3969). | Read. With a carriage return it was read as the marker too, so `old mode`/`new mode`, `index`, then the line was reviewed as a binary change while Git changed only the mode. | Only as the last line of a shape Git writes. With a carriage return it is refused. |
| `files-differ` | `Files A and B differ` | As `Binary files … differ` (`parse_chunk` 2322). | Not applicable. | As `Binary files … differ`. | No. | Read as `Binary files … differ` (v0.8.3 ignored it: G1). | Refused. |
| `traditional` | `--- P` then `+++ Q`, with no `diff --git`, then a hunk | A traditional patch (`find_header` 1661, `parse_traditional_patch` 868): the names, `/dev/null` and epoch timestamps give a creation or a deletion. | Each pair opens a file. | With no hunk after it: not a patch. | No: `diff -u` writes it. | Read. | Read, as before. |
| `no-line-feed` | A header line with no line feed, at the end of the input | Not read: the header loop stops (`parse_git_diff_header` 1417). | Not applicable. | The section as the earlier lines say. | No. | Refused. | Refused, as before. |
