# Bugs

Catalogue of bugs discovered during development. Per Maintenance Rule 8, bugs are logged here when found, not silently fixed. The author decides whether to fix immediately, defer, or leave alone.

Status vocabulary: open | fixed | wontfix | deferred.
Severity vocabulary: low | medium | high.

> First entries logged 2026-08-22, from the SPEC research pass rather than from testing. Use `BUG-001`, `BUG-002`, … sequentially; reference from commits, CHANGELOG `### Fixed`, and ATTACK_VECTORS where a bug pattern warrants a new vector.

## Open

_None._

## Fixed

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
