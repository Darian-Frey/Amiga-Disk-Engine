# Bugs

Catalogue of bugs discovered during development. Per Maintenance Rule 8, bugs are logged here when found, not silently fixed. The author decides whether to fix immediately, defer, or leave alone.

Status vocabulary: open | fixed | wontfix | deferred.
Severity vocabulary: low | medium | high.

> First entries logged 2026-08-22, from the SPEC research pass rather than from testing. Use `BUG-001`, `BUG-002`, … sequentially; reference from commits, CHANGELOG `### Fixed`, and ATTACK_VECTORS where a bug pattern warrants a new vector.

## Open

_None._

## Fixed

### BUG-007 `--format=json` is accepted and silently ignored by four commands
**Severity:** medium
**Status:** fixed
**Found:** 2026-08-28, auditing F-015's status against what the binary does.
**Where:** [cli/src/main.rs](../cli/src/main.rs), the `diff`, `consolidate`, `identify` and `formats` arms.

**What is wrong.** `--format=json` is parsed globally, and `info`, `ls`, `check` and `batch` honour it. The other four accept the flag, print text, and exit 0. `ade --format=json identify --datfiles=… disk.adf` produces a human report with no indication that the flag did nothing.

**Why it matters more than the missing output.** F-015 makes the JSON surface a stability commitment, and a script's normal way to ask "is this supported?" is to pass the flag and check the exit code. Silently ignoring it means a caller cannot tell an unsupported command from a successful one, and the failure surfaces downstream as a parse error against text that was never meant to be parsed. Refusing the flag would be a worse feature but a better contract.

**Correct behaviour.** Either emit JSON from all four — `diff` and `consolidate` have obvious record shapes, and `identify` already has one internally — or reject the flag with exit code 2 (usage) naming the commands that support it. Emitting is preferable; rejecting is the honest interim.

**Not a regression.** The flag has been global since IMP-001 on 2026-08-22, and each command added since simply did not wire it up.

**Fixed 2026-08-28 by emitting, not by rejecting.** All four now honour the flag: `formats` produces the whole conversion matrix, `diff` and `consolidate` a document each, and `identify` one JSON object per image as JSON Lines — the shape `batch` already uses, so a run over thousands of images is readable as it goes rather than only once it ends.

**The new shapes were the part worth thinking about**, because F-015 makes them a commitment the moment they ship, and a field is unconstrained exactly once:

- **The conversion matrix is keyed on codes, not on prose.** `Kind` gained `code()` — `adf`, `extended-adf`, `scp` — because `Display` produces `ADF (DD, 80 cylinders)`, a sentence carrying a geometry that varies between images of one kind. Matching on that is parsing prose. The display strings are alongside as `from_label` / `to_label` for anything rendering the matrix.
- **A conversion separates what it is from why.** `kind` is `lossless` / `lossy` / `not implemented` / `refused`; `reason` is the sentence. F-016 turns on the difference between the last two — refused is a decision that does not expire, not-implemented is a gap with a cause — and they invite opposite follow-up, so a caller must not have to read English to tell them apart.
- **`consolidate` reports `can_vote`.** With two dumps every disagreement ties by definition, so `unresolved_sectors` is arithmetic rather than damage; a caller ranking by it without this field would call every two-dump run the most broken thing it had seen.
- **`diff` lists which sectors moved, not merely how many.** The whole reason to compare two dumps is to act on the answer, and a disk holds 1760 sectors, so the list is bounded by the format rather than by a cap.
- **`identify` returns every match with `ambiguous`.** A caller taking `matches[0]` without checking would be choosing on ADE's behalf. *(The "71 collisions" originally cited here were re-measured on 2026-08-29 and are duplicate content under different names, not collisions; `identify` now says which kind of several a match is.)*

**One inconsistency found and left alone**: `diff`'s sector numbers are absolute while `consolidate`'s are within their track — sector 2 of track 1 is absolute sector 13, and the two commands report it differently. Both are right where they sit, since `consolidate`'s numbers live inside the track object that owns them. It is now stated in the doc comment rather than left for someone to discover by comparing two outputs of the same disk.

**`--output`'s confirmation moved to stderr under JSON only.** Announcing the merged image on stdout would put a line of prose in the middle of a JSON document, which is then not a JSON document. The first attempt moved it for *both* formats and an existing test caught it — fixing the JSON surface is no reason to move a line a text-mode script may already be reading, and "the text output is unchanged" had been asserted a few minutes earlier on the case that happened not to use `--output`.

**Verified**: 400 corpus images through `identify --format=json` gave 400 valid JSON Lines, zero malformed, zero non-ASCII; 50 documents from `diff` and `consolidate` over corpus pairs, zero malformed. Text output for all four commands is byte-identical to before. 12 new tests — 7 on the shapes, 5 on the flag being honoured, the latter checking that the flag *changes what comes out*, since a test asserting only "exit code 0" would have passed throughout the bug's life.

### BUG-008 `ade formats` panics on a closed pipe
**Severity:** low
**Status:** fixed
**Found:** 2026-08-28, running `ade formats | head` while checking what SCP conversion reported.
**Where:** [cli/src/main.rs](../cli/src/main.rs), the `formats` arm.

**What is wrong.** `ade formats | head` panics with `failed printing to stdout: Broken pipe (os error 32)`. The matrix is 70-odd lines, so piping it to `head` or `grep -m1` is the obvious way to read it.

**This is a known defect returning.** IMP-001 fixed exactly this for `info` and `ls` on 2026-08-22: `println!` panics on a closed pipe, `SIGPIPE` cannot be restored without `unsafe`, and all output was routed through an `emit` helper that treats a closed pipe as the ordinary end of a command. `formats` was added afterwards and writes with `println!` directly, so it never got the fix — the helper exists and is simply not used here.

**Correct behaviour.** Route the matrix through `emit` like every other command. Worth checking the other later arms at the same time, since the same omission is available to each of them.

**Fixed 2026-08-28, and it was six commands rather than one.** Checking the other arms found `--help`, `--version`, `convert`, `diff` and `consolidate` doing the same thing — every command written after IMP-001, and none of the ones written before it. The defect is not really "`formats` used `println!`"; it is that the rule lived only in a doc comment, where each new command had to rediscover it.

So three things changed rather than one. The direct writes are gone. `emit_lines` now exists for the common case — a command with a block of output — so the pattern to copy is one call rather than a loop somebody might not write. And `cli/tests/pipes.rs` runs commands into a reader that stops, asserting none exits 101, which is what a panic looks like from outside.

**The test was checked against the bug**, not just against the fix: restoring `println!` in `formats` makes it fail with `` `ade formats` panicked when its output was not read ``. A regression test that has never seen the regression is a guess.

**One consequence worth knowing.** `println!` flushes on every newline; a locked writer flushed once at the end does not. So output now reaches the pipe in one block, which is why `--help` never panicked in practice even before the fix — 1.5 KB fits the 64 KB pipe buffer, and the writer never learns the reader is gone. `formats` panicked because `println!` flushed line by line and met the closed pipe on line two. That also means `cli/tests/pipes.rs` is a net rather than a proof: a small command may pass whether or not it is correct, which is why the deterministic check on `emit_lines` lives in `main.rs`'s own tests against a writer that fails on demand.

**Verified identical.** `formats`, `--help`, `--version`, `diff` and `consolidate` produce byte-identical output before and after.


### BUG-006 The fixture generator panicked on any volume larger than ~2 MB
**Severity:** medium
**Status:** fixed
**Found:** 2026-08-24, on the first attempt to generate a hardfile for Phase 2's RDB work.
**Where:** [tools/fixtures/src/lib.rs](../tools/fixtures/src/lib.rs), `Volume::write_bitmap`.

**What was wrong.** The generator wrote exactly one bitmap block. One 512-byte bitmap block covers `(512/4 - 1) × 32 = 4064` blocks, so any volume above that — anything past about 2 MB — overran it:

```
index out of bounds: the len is 512 but the index is 1024
```

An 8 MB hardfile, the smallest thing worth testing RDB against, needs five bitmap blocks. A real volume of that size stores their pointers in the rootblock's 25 `bm_pages` slots, and past 25 of them in a `bm_ext` chain.

**Why it went unnoticed.** Every fixture until now was a floppy: 1760 blocks for DD, 3520 for HD, both comfortably inside one bitmap block. The limit only appears when a volume needs a second one, and nothing before Phase 2 asked for a volume that large.

Loud rather than silent, at least — a panic, not a wrong answer. But it made the whole hardfile and RDB branch of Phase 2 unreachable.

**Fixed 2026-08-24.** `write_bitmap` allocates as many bitmap blocks as the geometry needs, fills the rootblock's `bm_pages`, and chains beyond 25 through `bm_ext` — which `Bitmap::read` already followed, so the reader was ahead of the generator here.

### BUG-005 Reading a hard link returned an empty file, silently
**Severity:** high
**Status:** fixed
**Found:** 2026-08-24, on the first Phase 2 task — before writing link support, checking what the existing code already did with one.
**Where:** [src/filesystem/src/volume.rs](../src/filesystem/src/volume.rs), `Volume::read_file`.

**What was wrong.** `EntryKind::HardLinkFile.is_file()` returns true, and `read_file` accepted anything for which it did. But a hard-link block holds **no data of its own**: its `real_entry` field names the block it stands for (ADF FAQ §4.6). `read_file` therefore read the link block's own — empty — data-block table and returned `Ok("")`.

`real_entry` was parsed into `Entry` and never read by anything.

```
real.txt: kind=file     -> Ok("the actual contents")
link.txt: kind=linkfile -> Ok("")
```

No error, no shortfall, no fault. A caller extracting a hard link got a zero-byte file and nothing to suggest it was wrong — the silent wrong answer this project treats as the worst failure shape, and worse here than a refusal would have been.

**Why nothing caught it.** The corpus contains no links at all — none in 8865 entries sampled — because they are FFS-only and rare on floppies. The fixture generator could not build one either, so neither mechanism had material. It surfaced only because the Phase 2 task was "links" and the first step was to look at what already happened.

**Fixed 2026-08-24** as part of link support: `Volume::resolve` follows `real_entry` with a visited set and a bounds check, `read_file` resolves before reading, and `ade ls` shows link targets. See the Phase 2 entry in CHANGELOG.

### BUG-004 The fixture generator wrote the bitmap checksum into the map
**Severity:** medium
**Status:** fixed
**Found:** 2026-08-24, while building the F-010 health report — generated volumes reported 27 orphaned blocks where real disks reported none.
**Where:** [tools/fixtures/src/lib.rs](../tools/fixtures/src/lib.rs), `Volume::write_bitmap`.

**What was wrong.** The bitmap block is the **one exception** to the usual block layout: its checksum sits at offset 0 and the map runs from offset 4 (ADF FAQ §4.3, SPEC §Bitmap). Every other block type reserves 0 for the primary type and keeps the checksum at 20.

`write_bitmap` wrote it at 20, where the map's fifth word lives, silently overwriting the bits covering blocks 130–161. Fixtures therefore claimed around thirty blocks were allocated that nothing referenced.

**Why it hid.** The normal checksum is defined so the whole block sums to zero, which makes *validation* insensitive to where the field sits: checking against offset 20 succeeds on a block whose checksum is at 0. So the malformed fixtures passed their own checksum test, and ADE's reader — which also validated at offset 20 — agreed with them. The offset only matters for writing, and for not losing the map words it displaces.

**How it surfaced.** Not from a test, but from the disagreement between fixtures and reality: the health report showed 0 orphaned blocks on real disks and 27 on generated ones. That is D-010's two-mechanism design working in the direction I had not anticipated — the argument for the corpus was always that it would catch the *parser* being wrong, and here it caught the *generator*.

**Fixed 2026-08-24.** `normal_checksum_at(block, field)` in the fixtures crate and `checksum::normal_at` in `ade-block`; the bitmap writer uses field 0. `ade-block::checksum` gained `sums_to_zero`, which states the invariant directly and is what `Bitmap::read` now uses, since it is true regardless of layout.

### BUG-003 `read_file` allocated an attacker-controlled amount before reading anything
**Severity:** high
**Status:** fixed
**Found:** 2026-08-24, while applying IMP-003 — by mutation-testing `walk` and finding the 4 GB allocation was not in `walk` at all.
**Where:** [src/filesystem/src/volume.rs](../src/filesystem/src/volume.rs), `Volume::read_file`.

**What was wrong.** `Vec::with_capacity(entry.byte_size as usize)`. `byte_size` is a `u32` read straight off the disk, so a crafted or corrupt file header could claim up to 4,294,967,295 bytes on an 880 KB floppy — and ADE would try to allocate exactly that, before reading a single data block.

```
memory allocation of 4294967295 bytes failed
```

**Why it matters.** This is AV-005 — resource exhaustion from untrusted input — in one line, on the *plain ADF* path rather than in decompression where the vector was originally expected. It is the same failure class as the reference implementation's 29 GB blow-up that SPEC §Corpus observations holds up as the bar ADE clears, which ADE did not in fact clear.

It also passed 900,000 fuzz cases unnoticed. The harness asserts on *output* size, and a `with_capacity` that merely succeeds produces no output and no failure — so a several-hundred-megabyte allocation looked exactly like a clean run. Reserving is invisible to an output-bounds check.

**Fixed 2026-08-24.** The reservation is clamped to the volume's own size: `byte_size` is a hint, and the volume is the bound. A file cannot exceed the medium holding it.

Test `a_file_header_claiming_four_gigabytes_must_not_allocate_it` pins it, and runs under the ordinary suite.

**Follow-up worth noting.** The fuzz harness cannot see allocation that is never used. Catching this class properly needs either an allocator hook — which requires the `unsafe` the workspace forbids (D-001) — or explicit assertions at each point where a length from disk sizes an allocation. The second is tractable and there are few such points; it is not yet done.

### BUG-001 `Dostype::is_international()` is wrong for `DOS\4` and `DOS\5`
**Severity:** high
**Status:** fixed
**Found:** 2026-08-22, during the SPEC.md research pass (not by testing — no fixture exercises it yet).
**Where:** [src/filesystem/src/dostype.rs](../src/filesystem/src/dostype.rs), `Dostype::is_international`.

**What is wrong.** The method returns `flags & FLAG_INTL != 0`. Per [FAQ §4.1], when the dircache bit (2) is set the volume **is** international but the INTL bit (1) is left **clear**. So `DOS\4` and `DOS\5` report `is_international() == false` when they are international. [AOS-LNFS] adds that `DOS\6` and `DOS\7` are always international too.

**Why it matters.** `toupper` is the only difference between the two directory hash functions (SPEC §Directory hashing). Getting it wrong does not produce an error — it produces a hash that misses, so entries are reported "not found" on a structurally perfect disk. A silent wrong answer in the lookup path is the worst failure shape for a forensic tool.

**Correct behaviour.** International hashing applies when bit 1 **or** bit 2 is set, or when the dostype is `DOS\6`/`DOS\7`. See **C-006**.

**Notes.** The existing unit test `decodes_the_documented_bits` asserts the wrong behaviour (`assert!(dostype(2).is_international())` passes, but nothing asserts the `DOS\4` case), so the fix needs a test change, not just a code change. Worth keeping a distinct accessor for the raw flag bit, since a health report may want to say "dircache set, INTL bit clear" as an observation.

**Fixed 2026-08-22.** The fix was larger than "add an OR". `DOS\6` and `DOS\7` are `0b110` and `0b111` — the two combinations the classic encoding never used, *because* dircache implies international, which is exactly why LNFS was able to claim them. They are therefore dostypes rather than bit patterns and cannot be bit-decoded at all: the old `has_dircache()` reported them as dircache volumes, a second wrong answer the original report had not spotted.

Replaced the bit predicates with a `Mode` enum (`Classic` / `DirCache` / `LongNames`) resolved from the whole flags byte, plus `intl_flag_set()` to expose the raw bit separately so a health report can state what the disk says beside what it means. `mode()` matches `6 | 7` on the full byte, so a value that merely ends in `110` is treated as classic-with-unrecognised-bits rather than mistaken for LNFS.

**Verified against the corpus.** Surveying 4288 TOSEC images: 20 are `DOS\5`, every one of which reports `is_international() == true` with the stored INTL bit clear. All 20 would have hashed wrongly before the fix. The survey also found three images with a flags byte of `0x32`, now rendered as `DOS\50 (OFS, INTL, unknown bits 0x30)` rather than silently truncated.

Tests: `matches_the_full_dostype_table` walks all eight dostypes; `dircache_is_international_with_the_intl_bit_clear` and `lnfs_is_not_decoded_as_intl_plus_dircache` pin the two traps; `a_high_bit_does_not_turn_a_byte_into_lnfs` pins the whole-byte match.

### BUG-002 `Geometry::midpoint()` approximates the documented rootblock formula
**Severity:** low
**Status:** fixed
**Found:** 2026-08-22, during the SPEC.md research pass.
**Where:** [src/block/src/lib.rs](../src/block/src/lib.rs), `Geometry::midpoint`.

**What is wrong.** `midpoint()` computes `total_blocks / 2`. The documented location is `rootKey = (numReserved + highKey) / 2` where `highKey = numCyls * numSurfaces * numBlocksPerTrack - 1` [FAQ §4.2]. The two agree for DD and HD floppies with the usual two reserved blocks, and disagree once `numReserved` rises — e.g. four reserved blocks over 1000 total gives 501, not 500.

**Why it is deferred.** Not reachable today: `Geometry` has no `reserved` field and the only constructible geometries are the floppy constants, for which the answer is correct. It becomes live with RDB partitions in Phase 2, where `Reserved` comes from the partition's DOSEnvVec.

**Correct behaviour.** Carry `reserved` in `Geometry` and implement the documented formula. The doc comment should cite [FAQ §4.2] rather than describing the value as "the volume midpoint", which is the coincidence rather than the rule.

**Fixed 2026-08-22**, ahead of its deferral, because the API change was cheapest with almost nothing depending on it. `Geometry` gained a `reserved` field (`FLOPPY_RESERVED = 2` for the floppy constants, a fifth `new()` parameter otherwise), and `midpoint()` was renamed to `root_block()` implementing `(numReserved + highKey) / 2`. The rename matters: "midpoint" named the coincidence, not the rule.

`new()` also now rejects `reserved >= total_blocks` with a new `GeometryError::ReservedExceedsVolume`, since a volume that reserves itself entirely has no addressable blocks.

Test `root_block_follows_the_documented_formula_not_the_midpoint` uses a 1000-block geometry with four reserved, where the formula gives 501 and the old code gave 500, and separately asserts the two still agree for DD and HD.


## Won't Fix

_None._

## Deferred

_None._
