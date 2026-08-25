# CLAUDE.md

## Project

Amiga Disk Engine (ADE) — a forensic-grade, cross-platform toolkit for reading, validating, cataloguing, and writing Amiga floppy and hard-disk images. Successor to the Atari Disk Engine, carrying its lessons (chiefly: no god-class, protected formats designed-for early, untrusted-input stance). Early implementation: documentation-first, with a Rust workspace that builds and lints but does not yet parse an image.

## Current state

- **Docs:** full scaffold present in `Docs/` (FEATURES, ROADMAP, ARCHITECTURE, DECISIONS, SPEC, ATTACK_VECTORS, BUILD stub, BUGS, IMPROVEMENTS, CHANGELOG, this file) plus an index `Docs/README.md`. Consistent, cross-referenced. The repository landing page is the root `README.md`.
- **Tree:** Cargo workspace, one crate per pipeline layer under `src/<layer>/`, plus `cli/` (`ade` binary). CI runs fmt, clippy, tests, docs, and the layering check.
- **Code:** scaffold only. Real: `ade-endian` (C-001, complete for the widths so far) and `ade-block` (geometry, `BlockSource` seam, `ValidBlock` bounds proof). Partial: `ade-filesystem::dostype`. The other layer crates are documented stubs. No parsing of real images yet.
- **Enforcement worth knowing about:** C-001 is a `clippy.toml` tripwire (raw `from_be_bytes` fails the build outside `ade-endian`); D-003 is `tools/check-layering.py` (a cross-layer dependency fails CI); AV-004 is type-level — `BlockSource::read_block` takes a `ValidBlock`, constructible only via `Geometry::validate`. Do not route around these; widen the policy deliberately or not at all.
- **Stack:** settled 2026-08-21. **D-001** = Rust core + C-ABI bridge + Qt6 GUI (Phase 5). **D-002** = reimplement OFS/FFS/RDB in Rust; ADFlib is a black-box differential oracle only — never linked, source never read (that would forfeit the licence freedom). Implementation is unblocked.
- **Fixtures:** **D-010** Accepted 2026-08-22 — **no disk image is ever committed**, in any form. Fixtures are generated in code at test time; `tests/fixtures/` holds a manifest and docs only. A 4288-image TOSEC corpus lives in `disks/`, gitignored, for differential testing against the D-002 oracle; tests must skip cleanly when it is absent. `.gitignore` ignores disk-image extensions repository-wide with no whitelist — committing one requires a DECISIONS entry, not a negation line.
- **Open decision, non-blocking:** **D-009** (xDMS — wrap/port/reimplement; Phase 2). Its blocking unknown is now **resolved**: xDMS is **Public Domain** ("you can spread it, modify it and use it in any way you like", `/usr/share/doc/xdms/copyright`), so the leaning option — port to safe Rust — is available. The entry can be Accepted whenever wanted.
- **Licence:** **Apache-2.0** (D-011), with `LICENSE` and `NOTICE` at the root; D-008 discharged. The repository is public. Keep `NOTICE` accurate if D-009 ever introduces third-party code, and remember D-010 still constrains what may be committed to `tests/fixtures/` — a `.gitignore` tripwire ignores disk-image extensions there until it lands.

## Active task / next milestone

Phase 0 is complete (2026-08-22). Phase 1 is unblocked and nothing is waiting on a decision.

1. ~~First slice `ade info`~~ — **done 2026-08-22**. Runs over all 4288 corpus images with zero crashes.
2. ~~Mount and traverse~~ — **done 2026-08-22**. `ade ls` and `ade extract` work; 11,087 files extracted from a 400-image sample with zero read errors. AV-001 is discharged: every chain carries a visited set.
3. ~~Fuzz harness~~ — **done 2026-08-23**. 900,000 cases, six targets, zero failures. Runs in CI; deep sweeps via `ADE_FUZZ_ITERS`.
4. **Phase 2 in progress.** Done: links (BUG-005), HD and extra-cylinder geometry, bitmap rebuild, unpartitioned hardfiles (BUG-006), **RDB partitioning** and **dircache** (both 2026-08-24). Left: LNFS (field-level pass done; **blocked on verifiability**, see below), 5.25" DD (no source), configurable block sizes (parsed and honoured, but nothing to test against — everything held is 512).
   **A cache block is a block something must reach.** Dircache blocks are marked used in the bitmap; not following the chain made the health report call them orphaned on all 21 `DOS\5` corpus disks. Any new structure that occupies blocks must be added to the health report's reachable set, or it will be reported as lost space.
   **Only real directories carry a cache.** `EntryKind::is_directory()` includes `HardLinkDir`, which has an `extension` field that is not a cache pointer.
   **LNFS has neither corpus material nor an oracle.** ADFlib decodes the dostype by bit pattern, so it calls `DOS\6`/`\7` dircache volumes and, asked to use that cache, prints an empty listing and exits 0. [AOS-LNFS] also declares no long-name *file* header, so those fields would be placed by inference. Implementing it means writing a parser checkable only against SPEC and itself — which is what D-002 gave up ADFlib's accumulated knowledge to avoid. Not a coding problem; do not "just implement it" without a decision entry.
   **ADE does not share ADFlib's bug**, and `dostype_lnfs.rs` is the only thing keeping it that way: `Dostype::mode()` must match `6 | 7` **before** testing the dircache bit.
   **`DOS\0`–`DOS\7` is the whole AmigaDOS set** — no source knows a `DOS\8` or `DOS\9` (SPEC §The wider dostype registry, surveyed 2026-08-24). Do not design for a ninth. Other filesystems are *separate 4-byte tags* (`PFS\*`, `SFS\*`, `muF\*`, `AFS\*`, `KICK`, `UNI\*` …), not further `DOS\` values, and none appear in the corpus — they matter on RDB partitions, where `claims_amigados` is the guard.
   **Phase 3 started 2026-08-25 with ADZ/HDZ.** DMS stays blocked on test data (D-009); gzip was the part that could be finished, and it is.
   **MFM: a track is a bit stream.** Sync must be searched at bit granularity — byte-aligned scanning finds nothing on most tracks. Two sync words is the norm, three occurs; the body starts after the last. **Only the `0xFF` format byte marks a sector**: gap decodes to zeros and zero satisfies its own checksum, so checksums cannot tell them apart. The decode is self-evidencing (two checksums per sector) — no oracle needed, which is unique in this project.
   **Phase 4 started 2026-08-25** with the extended-ADF container. In a raw-track container, **`space` is the allocation and `length` the extent** — never read `space` bytes as data. A track with both zero is empty, not broken. MFM decode is next and needs a SPEC pass first ([RKRM] Appendix C, never consulted).
   **Refused and not-implemented are different answers** (F-016). One is a decision that does not expire (IPF, C-003), the other a gap with a cause (DMS, D-009). Never collapse them: they invite opposite follow-up.
   **Never match virus names in bootblock text** (D-014). It is measurably backwards: every corpus disk naming a strain carries an *anti-virus* bootblock, because cracking groups installed virus killers. Report the text; draw no verdict. AV-002's real defence is that ADE has no execution path at all.
   **Bootblock text needs filtering or it is mostly opcodes.** `NqNqNq` is `NOP` repeated. The thresholds in `bootblock.rs` are measured against the corpus, not chosen — 91% of kept runs contain a space.
   **The gzip oracle is exact.** `gzip` compresses, ADE decompresses, byte-identical or wrong — no adjudication, unlike D-002's ADF oracle. Every corpus image is a test case.
   **A decompressor caps output *before* each write.** Checking afterwards has already allocated. Never size anything from a declared length — gzip's `ISIZE` is verified after the fact, never trusted before (BUG-003 with the attacker holding the pen).
   **Not every container has a bootblock at block 0.** An extended ADF opens `UAE-1ADF`, a device opens `RDSK`. Parsing either as a bootblock produced a confident report about a checksum that was never one — twice, once for each container. `Kind::has_bootblock` decides now. `Unknown` keeps its bootblock, because 7% of real images have a non-`DOS` one and some still mount (C-008).
   **A device has no volume of its own.** Its block 0 is an `RDSK`, not a bootblock; every volume is inside a partition. Anything that mounts an image must therefore ask which partition first, and a report that says "no volume" for a device is calling a sound disk broken — that is what `Health::examined` and `--partition=` exist for.
   **The RDB counts in units the filesystem does not.** `SizeBlock` is in **longs**; its lists terminate at **-1**, not 0. Both have their own tests because both read plausibly when wrong.
   **The RDB path has no corpus.** 0 of 4652 images carry an `RDSK` in their first 16 blocks, so the D-002 oracle over generated devices is its *only* external check — and ADFlib is stricter than the format there, refusing any device without an `LSEG` chain that FAQ §6.5 says is not needed (SPEC §The oracle is stricter than the format here).
   **A raw volume has no geometry** — only its block count, which fixes the rootblock. ADFlib invents a shape to match; so does ADE.
   The fixture generator now builds file extension blocks (IMP-004), so large-file and extension-chain paths are under the oracle.
   **F-012 undelete is blocked on material**, not effort: zero intact deleted file headers across 90 corpus disks, because mastered game disks have no editing history.
   **Bitmap rebuild is computed, never applied** — D-004 defers writes to Phase 4 and is never-reversible in v1.
5. ~~F-010 health report~~ — **done 2026-08-24**. `ade check`, severity-ranked findings, bitmap cross-check discharging AV-003's detection. Bad sectors and weak bits are deliberately absent: flux-level, Phase 4.
   **The bitmap block is the one exception to block layout** — checksum at offset 0, map from 4. Everything else keeps its checksum at 20. Getting this wrong is invisible to validation, because the normal checksum makes the whole block sum to zero (BUG-004).
   **Phase 1's acceptance criteria are met and IMP-001/002/003 are all applied.** Nothing is outstanding against the phase.
   **Beware allocation sized from disk fields.** BUG-003 was `Vec::with_capacity(byte_size)` with `byte_size` straight off the disk — 4 GB on an 880 KB floppy, and 900,000 fuzz cases never saw it, because the harness bounds output and a successful reservation produces none. Any length read from a disk that sizes an allocation needs an explicit bound *and* an explicit assertion.
   **Never run the oracle uncapped.** `unadf` OOM-killed a whole session once; the test wraps it in `ulimit -v` and `timeout`.
   **Be careful mutation-testing traversal.** Removing `walk`'s visited set makes ADE allocate 28.8 GB and take the host down — that is IMP-003, and it is why the harness cannot currently prove that invariant by breaking it.
4. Then the health report proper (F-010) — aggregate the faults `ade info` already finds.
5. ~~IMP-001~~ — **applied 2026-08-22**. `--format=json` on `info` and `ls`. The JSON field names and fault codes are now a **stability commitment** (F-015): rename them only with a decision entry. Text output is explicitly not parseable and may be reworded freely.
3. Fuzz at the **block** level, not whole images — a rootblock parser takes 512 bytes, so seeding with 880 KB images wastes the budget on bytes nothing reads.
4. Report bootblock and filesystem as **two independent facts** (C-008). A `DOS` prefix does not imply a mountable volume — 19% of real ones are not — and its absence does not imply an unmountable one.

Fixtures are ready: `ade-fixtures` builds any volume the tests need and `corrupt` supplies the malformed cases. Real images are in `disks/`, gitignored; corpus tests must skip cleanly without them.

## Testing posture

Two mechanisms, and the boundary is **specification versus reality**, not generated versus real (D-010, amended 2026-08-24):

- **The oracle** (`unadf`) validates conformance to the format, on anything structurally valid — including fixtures ADE generates. Runs in CI. Catches the generator and the parser sharing a misreading.
- **The corpus** (4652 real images in `disks/`, gitignored) validates conformance to reality: what SPEC omits. Local only; CI never sees a real disk. Running it is a habit, not an option.

Never run the oracle uncapped — `unadf` OOM-killed a session once. Every invocation goes through `sh` with `ulimit -v` and `timeout`.

## Architectural invariants

Do not violate without a DECISIONS entry:
- No god-class; no module spans more than one pipeline layer (D-003).
- All byte-order conversion through one module — data is big-endian, host is little-endian (C-001).
- Every image is untrusted; no parse path may crash/hang/allocate unboundedly (D-006).
- Internal model can hold a raw MFM track from day one (D-005).
- Read paths ship before their write counterparts (D-004).
- Format dispatch by content sniffing, not extension (F-003) — and it is a cascade, not a magic lookup, because plain ADF has no magic (SPEC §The sniffing problem).
- International hashing applies when the INTL **or** the dircache bit is set, and always for `DOS\6`/`DOS\7` (C-006). Use `Dostype::mode()`; do not test flag bits directly. `DOS\6`/`DOS\7` are dostypes, not bit patterns — they took the combinations the classic encoding left spare (BUG-001, fixed).
- Directory traversal carries a visited-set: hard links to directories make cycles legal, not merely hostile (AV-001).
- No FFI dependency on ADFlib in any shipping path; it is a test oracle only, and its GPL source is not read (D-002).

## Build & test

**The toolchain is pinned** in `rust-toolchain.toml` — an exact version, not `stable`. Do not float it: CI denies all warnings, clippy gains lints every release, and a floating channel breaks the build with no code change. Bump the pin deliberately; the non-blocking `toolchain-drift` job says when it is worth doing.

No build yet, but the toolchain is settled — Rust + Cargo for core/CLI/bridge, CMake + Qt6 for the GUI from Phase 5. See [BUILD.md](BUILD.md). Fuzzing the parsers (`cargo-fuzz`) is part of the Phase-1 acceptance bar, not an afterthought, and the ADFlib differential suite must **skip** rather than fail when the oracle binary is absent, so a fresh clone still builds and tests.

## Conventions

- British English; ISO 8601 dates.
- Project-scaffold standard: append-only IDs (F-/D-/C-/AV-/BUG-/IMP-), fixed status vocabularies, reversal conditions on every decision, README status blockquote header.
- Greek/Latin primordial-mythology naming for ecosystem projects (ADE keeps the descriptive "Amiga Disk Engine" name for parity with the Atari engine; a codename may be assigned later).
- **Maintenance Rule 8:** log discovered bugs / improvement candidates in BUGS.md / IMPROVEMENTS.md when found — do **not** silently fix or apply inline. (This is the rule AI partners most often break.)
- Cross-reference both directions when adding register entries.

## Known pitfalls

- See [ATTACK_VECTORS.md](ATTACK_VECTORS.md) for the canonical list (AV-001…AV-005), all currently `Detection: not implemented`.
- IPF cannot be created (C-003); the flux write path is SCP / extended-ADF.
- DMS does not always round-trip (`errdms`, C-004) — fail loudly, never silently.

## Out of scope

- System/CPU emulation — ADE is not an emulator.
- Executing any guest code (bootblocks, binaries).
- IPF authoring.
- Re-litigating D-003, D-004, D-005, D-006, D-007 — these are settled unless their reversal conditions fire.
