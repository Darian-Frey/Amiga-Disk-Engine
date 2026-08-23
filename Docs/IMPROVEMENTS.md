# Improvements

Catalogue of code-quality improvements, refactors, and architectural changes proposed during development. Per Maintenance Rule 8, improvements are logged here when noticed, not silently applied. The author decides whether to apply, defer, or decline.

This is the dual of [BUGS.md](BUGS.md): bugs are broken; improvements work but could be better.

Status vocabulary: suggested | applied | declined | deferred.
Effort vocabulary: trivial | small | medium | large.

> First entry logged 2026-08-22. Use `IMP-001`, `IMP-002`, … sequentially. Every entry needs a **Trade-offs** field, or it is a feature request, not an improvement candidate.

## Suggested

_None._

## Applied

### IMP-003 `walk` and `read_file` are bounded only by their visited sets
**Status:** applied
**Effort:** small
**Found:** 2026-08-23, by mutation-testing the fuzz harness — deliberately removing the visited set from `Volume::walk` to confirm the fuzzer would notice.
**Where:** [src/filesystem/src/volume.rs](../src/filesystem/src/volume.rs), `Volume::walk` and `Volume::read_file`.

**What works today.** Both terminate correctly on every input tried: 900,000 fuzz cases and a 4652-image corpus, with no runaway. The visited sets are right, and D-006 is satisfied.

**Why it could be better.** `walk` accumulates into an unbounded `Vec`, and its boundedness depends *entirely* on one `HashSet::insert` returning false. There is no second line of defence on the invariant AV-001 exists to protect. Removing that single call makes ADE allocate **28.8 GB** and take the host down with it — the same failure shape as ADFlib's 29 GB blow-up recorded in SPEC §Corpus observations, from ADE's own code.

`read_file` is bounded more loosely than it looks: extension-chain length × 72 pointers × block size, which on a floppy permits roughly 64 MB for a file that cannot exceed 880 KB.

Two consequences beyond tidiness. A genuine regression in the cycle detection would not fail CI, it would **exhaust the runner's memory** — the fuzz harness asserts on `walked.len()` after the call returns, which is exactly what would never happen. And the invariant cannot be mutation-tested without risking the developer's machine, which is how this was found.

**Suggested fix.** Give both a hard structural cap that does not depend on the visited set being right:

- `walk` stops at the volume's block count. A volume cannot contain more entries than it has blocks.
- `read_file` stops at the volume's total bytes. A file cannot exceed the volume that holds it.

Hitting either cap is a **fault to report**, not a silent truncation — it means the visited set failed, which is a finding in itself. `FileContents` already has somewhere to say so; `walk` would need a result type rather than a bare `Vec`.

**Trade-offs.** A redundant check is dead weight while the primary defence holds, and two mechanisms enforcing one invariant can rot apart — the cap could drift wrong and nobody would notice, because the visited set gets there first. Against that: the cap is two comparisons, it converts a host-killing failure into a test failure, and it is the difference between an invariant that is *tested* and one that is *asserted*. D-006 forbids unbounded allocation on a parse path; resting that on a single line, for the specific vector rated Critical, is thinner than the decision intends.

**Related.** AV-001, D-006, F-001, and SPEC §Corpus observations for the reference implementation doing exactly this.

**Applied 2026-08-24.** `walk` returns a `Walk` struct rather than a bare `Vec`, carrying `hit_limit`, and stops at three structural bounds that do not consult the visited set: entry count, pending-directory count, and **depth**. `read_file` stops at the volume's total bytes.

Depth was not in the original plan and turned out to be the one that mattered. Bounding the entry count is not enough: each path is built from its parent's, so a cycle makes the *strings* grow without bound — `a/b/a/b/a/b/…` — while the count stays comfortably inside its cap. The first version of this fix still reached 4 GB.

**It found a real vulnerability.** Chasing that remaining 4 GB showed it was not in `walk` at all: `read_file` reserved `Vec::with_capacity(byte_size)` from a `u32` read off the disk, so a crafted header made ADE allocate 4 GB before reading anything. Logged and fixed as **BUG-003**. The improvement was hardening; the bug it uncovered was live.

**Verified by mutation.** Removing the visited set now produces a clean test failure in one second, naming the seed:

```
seed 1: the structural cap fired on a deliberately hostile image
        — the visited set should have stopped it first (IMP-003)
```

Previously the same mutation allocated 28.8 GB and the kernel killed the session. That is the whole point: the invariant is now *tested* rather than *asserted*.

The fuzz harness asserts `!hit_limit` on mutated and hostile images, so the cap firing is itself a failure — it is a backstop, not a mechanism, and a walk stopped by the backstop means cycle detection broke.


### IMP-002 OFS data blocks are read without validating their headers
**Status:** applied
**Effort:** small
**Found:** 2026-08-23, from an oracle disagreement on a corpus of 4652 images.
**Where:** [src/filesystem/src/volume.rs](../src/filesystem/src/volume.rs), `Volume::read_file`.

**What was wrong.** For OFS, `read_file` took each data block's `data_size`, clamped it to the 488-byte payload, and copied from offset 24 — never checking that the block *was* a data block. An OFS data block carries `type == T_DATA (8)`, a `header_key` pointing back at its file header, and a `seq_num` counting from 1; none was examined.

On `A500+A2000 Systest v9.1 …[cr A-Ha].adf`, the file `instruments/thriller2.ss` has table entries past index 38 pointing at blocks holding raw audio: `type`, `seq_num` and `data_size` all read `0x6db66db6`. ADE clamped the absurd length to 488 and copied sample data as payload, silently, producing plausible output with no sign the structure had stopped making sense a third of the way in.

**Applied 2026-08-24.** `read_file` now checks each OFS data block against what the table claimed of it, and records what it finds in `FileContents::faults`.

Nothing is refused. The bytes are read regardless, because refusing to recover data is the one thing a forensic tool must not do (D-012); an oversized `data_size` is still clamped rather than trusted. What changes is that the doubt is visible.

Faults are **summarised by kind**, not enumerated — one real file has 18 zeroed blocks in a row, and eighteen identical lines would bury the finding. Coalescing is on the fault's discriminant rather than on equality, so twenty blocks each naming a *different* wrong owner still report as one cross-linking finding.

`is_complete()` now means full-length **and** structurally sound; `is_full_length()` preserves the old meaning, because the two are genuinely different questions.

**What it caught immediately**, on the two disks that prompted the entry:

```
taterm1: recovered 29768 of 38320 declared bytes — 8552 short
taterm1: data block is entirely zero (18 blocks, first at 1749)

instruments/thriller2.ss: not a data block: type 0x6db66db6, expected 8 (22 blocks, first at 33)
```

The first now explains *why* it came up short. The second was previously **silent** — it returned exactly its declared length and looked like a clean read.

**The noise worry was unfounded.** Across a 388-disk sample and 4128 extracted files: 20 files carry a structural fault (0.48%), on 10 disks. All four kinds occur in the wild, including cross-linked blocks and out-of-sequence numbering. Rare enough to be a finding rather than a flood.

FFS reports nothing here, having no data-block header to check — C-005's forensic asymmetry showing up in the tooling.

**Trade-offs (as recorded before applying).** Strictness risks rejecting data a lenient reader would recover, so these are reports and never refusals. The noise cost was the other worry, and measurement settled it.

**Related.** F-010, D-006, C-005, D-012.


### IMP-001 `ade ls` output is not machine-parseable
**Status:** applied
**Effort:** small
**Found:** 2026-08-22, while validating extraction across the corpus.
**Where:** [cli/src/main.rs](../cli/src/main.rs), `list`.

**What works today.** The listing is readable and correct: size, protection bits, datestamp, name, optional comment, aligned in columns.

**Why it could be better.** The columns are whitespace-separated and two fields contain spaces — the datestamp (`1990-09-20 17:10:20`) and, more awkwardly, Amiga filenames, which routinely contain spaces. Splitting on whitespace therefore cannot recover the name. I hit this writing a validation script and misread 4283 files as extraction failures before realising the parser, not the tool, was wrong.

F-015 commits to "a documented, versioned CLI and library binding suitable for automation, with stable exit codes and **structured output**". The exit codes are done; the structured output is not.

**Suggested fix.** A `--format` flag offering at least a machine-readable mode. Tab-separated with the name last is the cheap option; JSON Lines is the more useful one for F-014's batch reporting, and would suit `ade info` equally, whose evidence list and fault list are currently prose.

**Trade-offs.** A second output path is a second thing to keep correct, and committing to a schema early risks having to break it — which for a stability promise is worse than not having made it. Against that: the schema is easier to design now, while the only consumers are ADE's own tests, than after F-014 has scripts depending on the human format. Doing nothing means the scriptable surface F-015 promises does not exist, and anyone automating ADE will parse the human output and be broken by the first column change.

**Related.** F-014 (batch reporting needs machine-readable summaries), F-015 (the promise itself).

**Applied 2026-08-22.** A `--format=json` flag on `ade info` and `ade ls`, with `--format=text` remaining the default and explicitly documented as unstable.

`ls` emits JSON Lines — one object per entry, so a large directory streams rather than buffering. `info` emits a single object carrying its evidence and fault lists as real arrays instead of prose. Names are written as `\uXXXX` escapes, so output is **pure ASCII** and a Latin-1 filename round-trips losslessly: a name is a sequence of bytes on the disk, not text, and a consumer must be able to get the original bytes back.

Written with an internal `ade_core::json` writer rather than `serde`, keeping the workspace at zero dependencies. The scope is narrow — ADE emits JSON and never parses it — so the whole escaping surface is one function.

**Two things came out of doing it.**

Fault computation moved from the CLI into `Inspection::faults()`, with a typed `Fault` carrying a **stable code** alongside its message. The message is for people and may be reworded; the code is the contract. That also means the human and JSON outputs cannot drift apart about what is wrong with an image, and a future GUI inherits the same list.

It exposed a genuine bug: `ade ls --format=json disk.adf | head` **panicked**, because `println!` panics on a closed pipe. For a tool designed to be piped that is unacceptable. Restoring the default `SIGPIPE` disposition needs a `libc` call behind `unsafe`, which the workspace forbids, so all output now goes through an `emit` helper that treats a closed pipe as the ordinary end of a command.

**Verified** across all 4288 corpus images: 4288 `info` objects and 68,961 JSON Lines, every one parsing, zero non-ASCII bytes, zero malformed output.


## Declined

_None._

## Deferred

_None._
