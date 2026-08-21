# CLAUDE.md

## Project

Amiga Disk Engine (ADE) — a forensic-grade, cross-platform toolkit for reading, validating, cataloguing, and writing Amiga floppy and hard-disk images. Successor to the Atari Disk Engine, carrying its lessons (chiefly: no god-class, protected formats designed-for early, untrusted-input stance). Early implementation: documentation-first, with a Rust workspace that builds and lints but does not yet parse an image.

## Current state

- **Docs:** full scaffold present in `Docs/` (FEATURES, ROADMAP, ARCHITECTURE, DECISIONS, SPEC, ATTACK_VECTORS, BUILD stub, BUGS, IMPROVEMENTS, CHANGELOG, this file) plus an index `Docs/README.md`. Consistent, cross-referenced. The repository landing page is the root `README.md`.
- **Tree:** Cargo workspace, one crate per pipeline layer under `src/<layer>/`, plus `cli/` (`ade` binary). CI runs fmt, clippy, tests, docs, and the layering check.
- **Code:** scaffold only. Real: `ade-endian` (C-001, complete for the widths so far) and `ade-block` (geometry, `BlockSource` seam, `ValidBlock` bounds proof). Partial: `ade-filesystem::dostype`. The other layer crates are documented stubs. No parsing of real images yet.
- **Enforcement worth knowing about:** C-001 is a `clippy.toml` tripwire (raw `from_be_bytes` fails the build outside `ade-endian`); D-003 is `tools/check-layering.py` (a cross-layer dependency fails CI); AV-004 is type-level — `BlockSource::read_block` takes a `ValidBlock`, constructible only via `Geometry::validate`. Do not route around these; widen the policy deliberately or not at all.
- **Stack:** settled 2026-08-21. **D-001** = Rust core + C-ABI bridge + Qt6 GUI (Phase 5). **D-002** = reimplement OFS/FFS/RDB in Rust; ADFlib is a black-box differential oracle only — never linked, source never read (that would forfeit the licence freedom). Implementation is unblocked.
- **Open decisions, neither blocking:** **D-009** (xDMS — wrap/port/reimplement; Phase 2; turns on xDMS's unestablished licence; lean is a port to safe Rust). **D-010** (test-fixture provenance — TOSEC images are copyrighted and cannot simply be committed to a repo intended to go public; lean is freely-distributable + synthetic fixtures, with a fetch script for the wider corpus).
- **Licence:** **Apache-2.0** (D-011), with `LICENSE` and `NOTICE` at the root; D-008 discharged. The repository is public. Keep `NOTICE` accurate if D-009 ever introduces third-party code, and remember D-010 still constrains what may be committed to `tests/fixtures/` — a `.gitignore` tripwire ignores disk-image extensions there until it lands.

## Active task / next milestone

1. Resolve **D-010**, then acquire the fixture set it permits (clean DD/HD, OFS/FFS/INTL/dircache, multi-partition HDF, known-bad `errdms`/protected, plus hand-authored malformed images for AV-001/AV-004).
2. Stand up the Cargo workspace over the existing `src/<layer>/` skeleton.
3. Begin **Phase 1** (F-001, F-002, F-003-ADF, F-010): read-only ADF parse validated against one OFS DD, one FFS DD, one multi-partition HDF fixture, with the fuzz corpus passing (zero crashes) and the ADFlib differential suite agreeing on clean fixtures.

## Architectural invariants

Do not violate without a DECISIONS entry:
- No god-class; no module spans more than one pipeline layer (D-003).
- All byte-order conversion through one module — data is big-endian, host is little-endian (C-001).
- Every image is untrusted; no parse path may crash/hang/allocate unboundedly (D-006).
- Internal model can hold a raw MFM track from day one (D-005).
- Read paths ship before their write counterparts (D-004).
- Format dispatch by content sniffing, not extension (F-003).
- No FFI dependency on ADFlib in any shipping path; it is a test oracle only, and its GPL source is not read (D-002).

## Build & test

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
