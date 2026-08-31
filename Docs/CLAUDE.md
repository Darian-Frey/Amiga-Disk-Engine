# CLAUDE.md

## Project

Amiga Disk Engine (ADE) — a forensic-grade, cross-platform toolkit for reading, validating, cataloguing, and writing Amiga floppy and hard-disk images. Successor to the Atari Disk Engine, carrying its lessons (chiefly: no god-class, protected formats designed-for early, untrusted-input stance). Documentation-first, with a Rust workspace, a C ABI and a Qt6 GUI that read, verify, catalogue and convert real images.

## Current state

- **Docs:** full scaffold present in `Docs/` (FEATURES, ROADMAP, ARCHITECTURE, DECISIONS, SPEC, ATTACK_VECTORS, BUILD stub, BUGS, IMPROVEMENTS, CHANGELOG, this file) plus an index `Docs/README.md`. Consistent, cross-referenced. The repository landing page is the root `README.md`.
- **Tree:** Cargo workspace, one crate per pipeline layer under `src/<layer>/`, plus `cli/` (`ade` binary). CI runs fmt, clippy, tests, docs, and the layering check.
- **Code:** 12 crates, an `ade` CLI, a C ABI (`bridge/`) and a Qt6 GUI (`gui/`). `ade` does `info`, `check`, `ls`, `extract` (one file or `--all`), `convert`, `create`, `scan`, `find`, `layout`, `formats`, `batch`, `identify`, `diff` and `consolidate`; the GUI browses several images at once, previews, extracts by drag, shows a whole disk in hex with its regions coloured, and searches every open image by name or by content. `ade-flux` reads SCP captures as of 2026-08-28. Still unread: DMS (D-009) and IPF (C-003), both sniffed and honestly refused.
- **Enforcement worth knowing about:** C-001 is a `clippy.toml` tripwire (raw `from_be_bytes` fails the build outside `ade-endian`); D-003 is `tools/check-layering.py` (a cross-layer dependency fails CI); AV-004 is type-level — `BlockSource::read_block` takes a `ValidBlock`, constructible only via `Geometry::validate`. Do not route around these; widen the policy deliberately or not at all.
- **Stack:** settled 2026-08-21. **D-001** = Rust core + C-ABI bridge + Qt6 GUI (Phase 5). **D-002** = reimplement OFS/FFS/RDB in Rust; ADFlib is a black-box differential oracle only — never linked, source never read (that would forfeit the licence freedom). Implementation is unblocked.
- **Fixtures:** **D-010** Accepted 2026-08-22 — **no disk image is ever committed**, in any form. Fixtures are generated in code at test time; `tests/fixtures/` holds a manifest and docs only. A 4288-image TOSEC corpus lives in `disks/`, gitignored, for differential testing against the D-002 oracle; tests must skip cleanly when it is absent. `.gitignore` ignores disk-image extensions repository-wide with no whitelist — committing one requires a DECISIONS entry, not a negation line.
- **Open decision, non-blocking:** **D-009** (xDMS — wrap/port/reimplement; Phase 2). Its blocking unknown is now **resolved**: xDMS is **Public Domain** ("you can spread it, modify it and use it in any way you like", `/usr/share/doc/xdms/copyright`), so the leaning option — port to safe Rust — is available. The entry can be Accepted whenever wanted.
- **Licence:** **Apache-2.0** (D-011), with `LICENSE` and `NOTICE` at the root; D-008 discharged. The repository is public. Keep `NOTICE` accurate if D-009 ever introduces third-party code, and remember D-010 still constrains what may be committed to `tests/fixtures/` — a `.gitignore` tripwire ignores disk-image extensions there until it lands.

## Active task / next milestone

**Registers audited 2026-08-28.** Statuses in FEATURES and ATTACK_VECTORS are now checked against what the binaries do; ROADMAP's per-phase list was renamed **"Features in scope"**, which is what it always held — Phase 2's names F-012 and F-017, neither begun. Do not read a scope line as a delivery record.

Phases 0 and 1 are complete. Phases 2–5 are each partly delivered, and **as of 2026-08-29 nothing unblocked remains**: what is left is blocked on material or hardware, or settled by a decision — DMS (D-009), LNFS (D-013), virus signatures (D-014), F-012 undelete (no deleted headers in 90 corpus disks), F-006 and F-005 (no Greaseweazle board), F-017 (cut, D-017), WHDLoad and OpenRetro (D-016). The register has no undecided entries. SCP reading and F-018's GUI half both landed 2026-08-28. Nothing unblocked remains in Phases 2–5; what is left waits on material or hardware.

   **The C ABI's reading calls take a partition selector**, `ADE_WHOLE_IMAGE` for an image holding its own volume. A device is not a special case of an image — it is what an image is when it has an RDB. Never compute a partition's rootblock from `first_block`: its own block size and reserved count decide where it sits (C-007), so pass the index and let the engine resolve it.
   **Report whether a partition *mounts*, not whether it is bootable.** Different questions, and a `PFS\0` partition is a real partition ADE cannot read — an empty listing there reads as an empty disk.
   **The bridge holds a mounted `Image`, not bytes** (IMP-006). Rebuilding it per call re-decoded the whole container — 160 tracks of flux, or a full gunzip — on every click: 8681 ms to expand 84 drawers became 63 ms. The handle's `image` is `Option`, because a container ADE cannot mount **still opens**: the container and the reason are what a person wants from such a file.
   **Generated test fixtures must be regenerated, not cached.** Declared to CMake as build outputs they go stale the moment the generator changes, and the resulting failure looks like a bug in the code under test.

   **SCP is two byte orders in one file** — little-endian header and offsets, big-endian flux values. `009e` is 158 one way and 40448 the other, and the wrong choice finds no sectors rather than failing. `ade-endian` has `u16_le_at`/`u32_le_at` for this; `clippy.toml` now disallows `from_le_bytes` too, a hole that existed for as long as nothing was little-endian.
   **A flux decoder's lock check must count what it rejected.** Drift alone is not enough: at a wrong data rate every interval is out of range, nothing corrects the estimate, and a drift-only check calls total failure a perfect lock. Same trap in integer arithmetic — a correction of 1/16 of one tick is zero, so the loop never runs and never drifts. Carry the estimate scaled.
   **Identification is configured, never automatic.** Loading the dataset takes 140 ms against `ade info`'s ten, so `ade_core::datfiles_location` checks `--datfiles=`, `$ADE_DATFILES`, then `$XDG_DATA_HOME/ade/datfiles`, and costs nothing when none is set. Scripted corpus work uses `batch --datfiles=`, which loads once rather than 4,652 times.
   **The fixture generator depends on nothing, including for CRC32** (D-010). It has its own implementation, and if it ever disagrees with `ade-block`'s the identification test fails — which is the signal the independence exists to give. The layering check enforces this; do not route around it.
   **A partial corpus measurement that agrees with the design is the easiest one to stop early.** F-021's region attribution was designed from the alphabetically first 500 images, where every `Copylock` hit is in block 0. Over all 4,652 the claim is false — 51 hits are elsewhere, and the most interesting cluster (ten disks whose *volume name* is `Copylock(tm) Amiga`, so the hit is in the rootblock) is entirely outside the sample. Sample to form the hypothesis; measure the whole corpus before writing the number down. It takes 17 seconds.
   **Check a build's exit status, not a grep of its output.** Filtering `cmake --build` through `grep -E "error" -A5` and reading a clean result as success is wrong twice over: the pattern can miss the diagnostic, and a failed build leaves the **previous binary in place**, so the test run that follows passes on stale code. That happened here — two consecutive "31 passed" runs against a binary whose source no longer compiled. Same shape as the stale-fixture trap already recorded below: the failure imitates success. Use the exit status, or read the tail unfiltered.
   **An Amiga filename is not a host filename, and the mapping is measured** (F-024). Latin-1 decoded to UTF-8; `%XX` escapes for what cannot be a filename — NUL, `/`, `.`/`..`, and `%` itself so the escape is reversible. What is *not* escaped is the judgement: 62 corpus names carry a Windows-illegal character and 328 end in a dot or space, all legal on POSIX. `host_name` takes an **already-decoded** `&str`, because `name_lossy` has done the Latin-1 pass — decoding a second time turns `für` into `fÃ¼r`, which loses a name while appearing to preserve it.
   **A `QSyntaxHighlighter` re-highlights its whole document when its inputs change.** Give it the disk map *before* the text, over a cleared pane: set afterwards, a 56,000-line whole-disk dump cost eleven seconds and was briefly painted in the previous disk's colours. Related, when reading formats back in a test: Qt splits a whole-line background into fragments wherever a foreground is set on top, so "one range covering the line" finds nothing while the colour is plainly on screen.
   **Qt does not reliably say when a scroll area's view moved.** Measured on QPlainTextEdit: a click in the scrollbar **trough** scrolled it from line 0 to line 37 while emitting `valueChanged` zero times and calling `scrollContentsBy` zero times — the wheel and a handle drag both notify, so the gap looks like the feature working until somebody pages down. Anything that must follow the scroll needs a cheap poll as well as the signal. Take the top line from `verticalScrollBar()->value()` (with wrap off it counts blocks, so it *is* the line) and never from `cursorForPosition`, which reads where the viewport was painted and lags.
   **`QTreeWidget::setCurrentItem` can move the current row without moving the selection.** The window listens for `itemSelectionChanged` and reads `selectedItems()`, so it then redraws whatever is *still* selected — which looks exactly like a failure to clear the view. Use the `QItemSelectionModel::ClearAndSelect` overload in tests; it is what a click does.
   **Never dispatch on SCP's disk-type byte.** `gw` writes 0x80 ("other") for an Amiga disk it encoded itself; the spec's 0x04 is aspirational.

The numbered history below is kept for its hard-won lessons, not as a plan; its numbering drifted long ago and the phase notes inside it are the state at the time of writing.

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
   **`bridge/` is the only crate that may write `unsafe`.** It opts out of the workspace lint set because `forbid` cannot be lifted by `allow`; keep every other lint restated there. Names crossing the ABI are `AdeBytes`, never `char*` — Amiga filenames are Latin-1. Every entry point wraps `catch_unwind` and tolerates null. The C smoke test is not optional: only a C compiler reading `ade.h` catches the header disagreeing with the library.
   **TOSEC datfiles live in `/datfiles/`** (gitignored, fetched from the archive.org DAT Pack — metadata only, never the ROM sets). 88,921 Amiga entries identify 98% of the corpus. **Several matches means duplicate names, not a collision** (measured 2026-08-29, correcting an earlier "71 collisions" note): all 77 groups sharing a CRC32 also share their SHA-1 and MD5 — one file listed under two names — and none is an `.adf`. Return every match, never pick one, and say *which kind* of several it is. SHA-1 runs only when more than one candidate survives, so it never runs on this corpus.
   **`gw` (Greaseweazle) is an SCP oracle**, installed 2026-08-27 — it converts any ADF to real SCP and round-trips byte-identically, so SCP is no longer blocked on material. But **generation is non-deterministic** (assert the round trip, never the bytes) and **`gw` silently mis-reads extended ADFs** as plain sector data, reporting a confident 100% about its own encode. Oracle for plain sector images only.
   **Phase 5 started 2026-08-26** with `ade batch` (F-014). Qt6 6.4.2 is installed here, so the GUI is buildable; Greaseweazle is not (F-006 needs hardware) and there is no TOSEC datfile (F-013's dataset).
   **A batch histogram counts images, not occurrences.** One damaged disk raising a code fifty times is one affected disk. Getting this wrong made 186 images read as 1050.
   **Consolidation reports agreement, not correctness** (F-008). The corpus's multi-dump titles are independent dumps of possibly different copies — some TOSEC-tagged `[m ...]`, i.e. deliberately edited — not repeated reads. Two dumps cannot vote at all. Never call a plurality a best estimate.
   **MFM encoding computes clock bits over the whole stream**, not field by field — a clock bit depends on the data bit before it. Sync words are excluded on purpose. Round-trip through the decoder is the test: a disk encoded and read back must be byte-identical.
   **An assembled volume is a reconstruction and must always say so** (F-007). Undecodable sectors are zeros, so a listing can omit half a disk silently. Never mount one without reporting `sectors_placed`. Place sectors by **physical position** — two corpus disks label every track 0.
   **A clock-violation count is not a protection score** — tried, measured, abandoned. Sync boundaries contribute their own violations, so subtracting sync words gives a number that changes sign depending on how you count the baseline. Report raw counts; derive nothing.
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

Three build systems, all real: Cargo for the core, CLI and bridge; `cc` for the C ABI smoke test, which is the only thing that catches `ade.h` disagreeing with the library; CMake + Qt6 for the GUI, which invokes Cargo itself. See [BUILD.md](BUILD.md) — every command in it has been run as written. Fuzzing the parsers (`cargo-fuzz`) is part of the Phase-1 acceptance bar, not an afterthought, and the ADFlib differential suite must **skip** rather than fail when the oracle binary is absent, so a fresh clone still builds and tests.

## Conventions

- British English; ISO 8601 dates.
- Project-scaffold standard: append-only IDs (F-/D-/C-/AV-/BUG-/IMP-), fixed status vocabularies, reversal conditions on every decision, README status blockquote header.
- Greek/Latin primordial-mythology naming for ecosystem projects (ADE keeps the descriptive "Amiga Disk Engine" name for parity with the Atari engine; a codename may be assigned later).
- **Maintenance Rule 8:** log discovered bugs / improvement candidates in BUGS.md / IMPROVEMENTS.md when found — do **not** silently fix or apply inline. (This is the rule AI partners most often break.)
- Cross-reference both directions when adding register entries.

## Known pitfalls

- See [ATTACK_VECTORS.md](ATTACK_VECTORS.md) for the canonical list (AV-001…AV-005). All five now have implemented detection with named tests (audited 2026-08-28) — but detection existing is not the vector closed, and each entry records what would prove its mechanism broken.
- IPF cannot be created (C-003); the flux write path is SCP / extended-ADF.
- DMS does not always round-trip (`errdms`, C-004) — fail loudly, never silently.

## Out of scope

- System/CPU emulation — ADE is not an emulator.
- Executing any guest code (bootblocks, binaries).
- IPF authoring.
- Re-litigating D-003, D-004, D-005, D-006, D-007 — these are settled unless their reversal conditions fire.
