# Vesper V3 round 16 review — `4391c1e`

## Verdict

**NO-GO.** IR8-M-1 and IR8-M-2 are closed, IR8-M-3 and IR8-M-4 are acceptably declared, and the original bare-`Command` reproduction of VD3R15-M-1 is fixed. However, the same edge-selection claim is still false: a valid Git-produced hunk can end on `Command::` without showing a process spawn. That is `VD3R16-M-1` and breaks claim 4. I also found one low third spelling of the released network paths, `VD3R16-L-1`.

Reviewed exact commit `4391c1edf6c663e51b24eb7efd14ee8e9cf0aaac`. Frozen release SHA-256: `155700c0518cbadc80e0763735ec29997472bb2deaedf7c54a25292db6b57dfb`.

## Findings

### VD3R16-M-1 — an honest line can end in `Command::` without spawning a process

The round removes the false edge on bare `Command`, but still treats a side ending in `Command::` as `process_spawn` (`crates/deep-diff-forge-graph/src/rust_tokens.rs:899-907`). Rust permits a path to continue on the next line, and the following unchanged line need not be `new` or any spawning operation.

I compiled this valid source after changing only `Other::` to `Command::`:

```rust
struct Other;
impl Other { const VERSION: u8 = 0; }

struct Command;
impl Command { const VERSION: u8 = 1; }

const X: u8 = Command::
VERSION;

fn main() { println!("{X}"); }
```

The program compiled with Rust 1.99 and printed `1`. Git itself produced this zero-context patch:

```diff
@@ -7 +7 @@ impl Command { const VERSION: u8 = 1; }
-const X: u8 = Other::
+const X: u8 = Command::
```

Frozen results:

- v0.8.3: score 2, no evidence.
- v3r14: score 2, no evidence.
- v3r15: score 6, `process_spawn` at new line 7, basis `hunk`.
- v3r16: score 6, `process_spawn` at new line 7, basis `hunk`.

There is no process spawn in the file. The same grammatical issue applies to the retained `process::`, `TcpStream::`, and `UdpSocket::` edge forms: a line break after a path separator is valid, and a later unchanged segment can name an unrelated item. The 8.1-million-line measurement does not establish that no honest line can end so; it establishes only that the sampled lines did not.

The exact bare-name case from VD3R15-M-1 is closed, but the substantive edge false-positive class and claim 4 are not. Remove these path-prefix edge forms, or require enough tokens on the shown side to establish the released construct. Pairing with a later hunk can recover a construct when that hunk is present, but no inference can distinguish the reproduction above when the continuation is unchanged and absent from the patch.

Reproduction: `/var/home/louranicas/.cache/ddf-t/vesper-scratch/v3r16-command-colon/command-colon.patch` and its scratch Git repository.

### VD3R16-L-1 — fully qualified angle-bracket network paths remain unread

The two added spellings are found, but the same standard paths with their full owner inside angle brackets are not:

```rust
let _ = <std::net::TcpStream>::connect("127.0.0.1:9");
let _ = <std::net::UdpSocket>::bind("127.0.0.1:0");
```

Both compile and run. Frozen v3r16 reports no `network_fetch` for either line. In the same patch it reports `<std::process::Command>::new(...)` only because the separate `process::Command` owner rule fires. `released_path` accepts a bracketed target only when the target name stands alone inside `<...>` (`rust_tokens.rs:696-713`), so it misses the qualified network owners.

This is low, matching the severity of IR8-L-1: v0.8.3 misses it too, and it is another spelling of the same path rather than a regression below the release. Reproduction: `/var/home/louranicas/.cache/ddf-t/vesper-scratch/v3r16-paths/{q.rs,q.patch}`.

## Requested closures

- **VD3R15-M-1: exact reproduction closed; substantive class not closed.** Bare `Command` no longer fires, and the original patch returns to score 2. `VD3R16-M-1` shows that the retained `Command::` edge still violates the same “honest line” selection rule.
- **IR8-M-1: closed.** `decoy2.patch` is back at r14's score 5 with `trivial_assert`; changing the decoy's unread number no longer changes r16's answer. `triple.patch`, which r14/r15 missed, now gains the assertion. Source keeps the round-14 `Lined` family independently and adds `LinesFirst` and `Positions` beside it. Plain r14-to-r16 comparison over all 6,002 listed patches found zero loss.
- **IR8-M-2: closed.** On the independent review's 50,000-hunk extension-less path view, r15 took 41.98 s and r16 took 0.56 s (v0.8.3: 0.57 s). On the 333,000-hunk file, r16 took 0.29 s for a 64 KiB page and 0.33 s for a 1 MiB page. `Scans` caches `FileKinds` per file and passes it to `hunk_signals_alone`; the exact-commit counter test confirms two whole-file passes at both 30 and 600 hunks.
- **IR8-M-3: acceptably declared.** The documentation now explicitly names the word-alone three-hunk cut and the function-value cut before its name, and the 265/201 boundary is pinned. I do not require those writer-only two-false-number forms to be read in this round.
- **IR8-M-4: acceptably declared.** The documentation explicitly names the `--unidiff-zero` insertion inside an earlier hunk's shown lines, gives the reproduction shape, and states that no reading interleaves a hunk into another hunk's lines.

## Claims

1. **Held in the tested full-answer scope.** Every hunk-alone reading remains, the six joined families are additive, and r14-to-r16 over 6,002 patches had zero loss and 132 gains. Released items retain bounded-list priority.
2. **Held for single-section inputs and under the stated parser dependency; not literally held by this branch alone.** Strict v0.8.3-to-r16 comparison over 6,002 patches printed only the known two-section case: one duplicate `src/evil.rs` entry loses `new_file` when the two entries change places. There were 2,518 gains, 275 mutual refusals, and no refusal difference. I continue to accept this only with the prerequisite that the branch is rebased onto the release-line parser refusal before merge.
3. **Held in the tested scope.** `decoy2` closes the known withdrawal, the 40,000-case property test carries round 14's rule independently, and I found no r14 loss in 6,002 patches. At one shared position, 50,000/100,000/200,000 one-line hunks took 0.21/0.35/0.63 s and 29,024/54,340/104,788 KiB, consistent with linear storage and no position hot spot. Source has six joined families at most, hence seven texts including the hunk alone. The documented lines-bearing false-number predecessor remains a declared exception.
4. **Broken by VD3R16-M-1.** A Git-produced honest line ends in a still-read form and is not the claimed construct.
5. **Held in the modes and shapes tested.** IR8-M-2 is closed; the same-position stress is linear; the 499,990-hunk creation measured 263,240 KiB against v0.8.3's 176,696 KiB; the 333,000-hunk joined-hints shape measured 317,736 KiB against 159,788 KiB. The disclosed 2.0 peak-memory factor is accurate for these shapes. The hidden thread-local `file_walks()` counter is acceptable as a test door: it has no answer semantics and directly holds the regression's whole-file-pass count.
6. **Held literally, but incomplete as a quality claim.** The exact-commit eval tests pass, including manifest drift, declared misses and corpus floors; the corpus describes its entries as the binary answers them. It does not contain VD3R16-M-1 or VD3R16-L-1, so passing it does not establish claims 4 or D.

## Answers to F and G

### F. Pairing option and the two r15-lower patches

I do **not** require the `.unwrap`/`(` pairing option. The two-false-number cut is now accurately declared, while pairing independently selected hunk edges would add another heuristic and its own false associations.

I also do **not** require retaining r15's defective join family merely to keep its two generated gains. Round 14 is the stated monotonicity anchor. Keeping the r15 family would raise the per-hunk text bound from seven to eleven to preserve behavior that IR8-M-1 showed was header-order-sensitive and could withdraw a round-14 join. The correction is preferable.

### G. Two-section dependency and memory

Both remain acceptable as stated. The two-section behavior is acceptable only as a merge-order dependency: rebase onto and retain the release-line parser refusal before this branch merges. It is not an independently true v0.8.3 floor on the current branch. The three memory shapes and 2.0 joined-shape factor are now stated accurately and agree with my two repeated measurements.

## Branch decision

`d4dbc91..4391c1e` is **not yet a GO** because `VD3R16-M-1` is a medium false positive in an ordinary Git-produced patch. After that edge rule is corrected, `VD3R16-L-1` should also be addressed or explicitly carried as a remaining spelling, and the release-line parser dependency must be present before merge.

## Verification performed

- Read the complete request, round-15 report, eighth independent review, `ceb2e6f..4391c1e`, affected graph/token/view code, tests, fuzz changes, corpus changes, README and CHANGELOG claims.
- Verified exact clean HEAD and both frozen hashes. `git diff --check` reported only trailing spaces inside adversarial patch fixture payloads.
- Reproduced the original VD3R15-M-1 repair, `decoy2`, its other-number control, `triple`, the two new released-path spellings, IR8-M-2 at 50,000 and 333,000 hunks, and the two disclosed memory shapes.
- Built and compiled the two new Rust reproductions and obtained the medium's patch from Git with `git diff -U0`.
- Compared all 6,002 supplied patches: v0.8.3 strict to r16 had only the known two-section loss; r14 to r16 full answer had zero loss.
- Checked the hive slot immediately before each Cargo invocation. From an exact `git archive 4391c1e`, with a private target directory and toolbox launched under the real HOME: graph 233 unit plus 6 integration tests passed; daemon 229 passed with one child-helper ignored; graph/daemon doc tests passed; CLI eval 9/9 passed.

## Not independently repeated

I did not run the complete 2,078-test workspace, gate, clippy, the four fuzz campaigns, mutation plants, the implementer's 55,000/300,000/12,000-case campaigns, repository-history comparisons, or all 7.9 million honest-line probes. I did not reproduce the 9,000 long-context memory shape, run an actual daemon socket or MCP request, prove Git's placement algorithm from Git source, exhaust every hint collision/order, or inspect unrelated branch changes.

STATUS: COMPLETE
