> **Status:** Active
> **Provenance:** Claude (primary auditor / initial scaffolding, 2026-08-21)
> **Last reviewed:** 2026-08-21
> **Why this status:** Early implementation. The stack decisions are settled (D-001, D-002) and the licence chosen (D-011). The workspace builds, tests, and lints, but the engine is a scaffold: no image is parsed yet. Fixture provenance (D-010) remains open and gates Phase 1 validation.

# Amiga Disk Engine (ADE)

A forensic-grade, cross-platform toolkit for reading, validating, cataloguing, and writing Amiga floppy and hard-disk images — the successor to the (now complete) Atari Disk Engine, carrying its lessons forward.

ADE targets the gap no current tool fills. The capable Amiga engines (amitools, ADFlib) are CLI-only and developer-facing; the friendly tools (ADF Opus, the various Greaseweazle GUIs) are narrow, Windows-bound, or abandoned; and none joins flux capture to catalogue in a single application.

## What makes it different

- **A never-crash core.** Every image is untrusted input. No parse path may crash, hang, or allocate unboundedly, however malformed the input — and fuzzing is an acceptance bar from the first phase, not an afterthought. (F-001, D-006)
- **Capture to catalogue in one application.** Flux capture → image → filesystem browse → catalogue → write-back, without a relay race across `gw`, disk-utilities, xDMS, and a browser tab. (F-005)
- **Forensic health reporting.** Bad sectors, weak bits, checksum failures, bitmap validity, and OFS/FFS recoverability surfaced explicitly — never failed silently — with auto-identification against TOSEC / WHDLoad / OpenRetro. (F-010, F-013)
- **Corpus scale.** Bulk verify, convert, catalogue, and report across collections numbering in the thousands, with machine-readable output. (F-014)

## Quick start

```bash
git clone https://github.com/Darian-Frey/Amiga-Disk-Engine
cd Amiga-Disk-Engine
cargo test --workspace

cargo run -p ade-cli -- info    disk.adf     # container, geometry, bootblock, volume
cargo run -p ade-cli -- check   disk.adf     # health report, severity-ranked
cargo run -p ade-cli -- ls      disk.adf     # directory tree
cargo run -p ade-cli -- extract disk.adf Tools/thing out.bin
```

Add `--format=json` to any of them for machine-readable output; the field names and fault codes are a stability commitment (F-015). On a partitioned hard disk, add `--partition=DH0` — a device holds no volume of its own.

ADE never writes to an image: no image-write path exists in the codebase, and D-004 defers one to Phase 4. Even the bitmap rebuild is computed and reported, never applied. See [BUILD.md](Docs/BUILD.md).

## Status

**Phases 0 and 1 are complete; Phase 2 is in progress.** ADE reads OFS and FFS on DD, HD and extra-cylinder floppies, on unpartitioned hardfiles, and on RDB-partitioned hard disks. All eight dostypes are identified and hashed correctly, including the international variants; the dircache and long-name *structures* of `DOS\4`–`DOS\7` are still to come. It lists, extracts, resolves hard links, and reports volume health including a bitmap cross-check.

Measured rather than asserted:

- **4652 real images** read with zero crashes — including 11 that crash ADFlib.
- **99.36%** byte-identical agreement with ADFlib on extracted file contents (3875 of 3900 files). Every difference is attributed to a recorded disagreement (D-012) or to genuine damage on the disk.
- **900,000 fuzz cases** across six targets, zero failures. AV-004 is enforced by the type system, not by discipline: `read_block` takes a `ValidBlock`, which only `Geometry::validate` can construct.

Still to come in Phase 2: real dircache blocks, LNFS long names, and undelete — the last blocked on material rather than effort, since the corpus holds no recoverable deleted entries. Then containers (Phase 3), flux (Phase 4), and the GUI (Phase 5).

The two decisions that gated all implementation were settled on 2026-08-21:

- **D-001 — stack.** Rust core exposing a C-ABI bridge, with a Qt6 GUI over it from Phase 5. Memory safety and the untrusted-input mandate (D-006) are the same decision, and a Cargo workspace makes the "no module spans two layers" invariant a matter of the dependency graph rather than of review.
- **D-002 — ADFlib.** Reimplement OFS/FFS/RDB in Rust rather than wrap the C library, using ADFlib as a **black-box differential-test oracle** — run as a separate binary, never linked, source not read. A segfault inside wrapped C is uncatchable from Rust, so wrapping would make F-001's "zero segfaults across the fuzz corpus" unclaimable. Licence freedom is preserved as a side effect.

The licence followed from D-002 and is settled: **Apache-2.0** (D-011). D-010 (fixture provenance) is settled too: fixtures are generated, never committed, and the oracle validates the generator as well as the parser. **D-009** (xDMS's role) remains accepted-but-blocked — on test material rather than on a decision. See [DECISIONS.md](Docs/DECISIONS.md) and [ROADMAP.md](Docs/ROADMAP.md).

## Formats

| Layer | Formats |
|---|---|
| Floppy images | ADF (DD 880 KB / HD 1.76 MB), ADZ, extended-ADF |
| Hard-disk images | HDF, HDZ, RDB multi-partition |
| Compressed | DMS (all modes, including encrypted), gzip wrappers |
| Flux | SCP (read/write), IPF (read-only, optional, licence-gated) |
| Filesystems | OFS and FFS across all eight dostypes, INTL and dircache variants, long filenames |

ADE **cannot create IPF** — authoring is closed (SPS-only), so the open flux write path is SCP and extended-ADF (C-003, D-007). Some DMS images are known-bad and will not round-trip; ADE fails loudly rather than emitting a silently-bad ADF (C-004).

## Architecture at a glance

A strict layered pipeline, each layer a separately-testable module behind an interface seam, so that no component can accumulate responsibilities across layers — the god-class failure the Atari Disk Engine suffered (D-003).

```
catalogue / export  →  object model  →  filesystem  →  block  →  track / MFM  →  flux
```

Data flows upward on read and downward on write. A container front-end normalises ADF/ADZ/HDF/HDZ/DMS into the block layer, so the upper layers never see compression or wrapping. One core library API is the single seam both the CLI and the Qt6 GUI consume.

Full detail in [ARCHITECTURE.md](Docs/ARCHITECTURE.md).

## Repository layout

```
Amiga-Disk-Engine/
├── README.md          This file — project landing page.
├── LICENSE            Apache-2.0 (D-011).
├── NOTICE             Attribution; records that ADE has no third-party code.
├── Docs/              Documentation set (see index below).
├── Cargo.toml         Workspace manifest; the lint set encodes the invariants.
├── clippy.toml        C-001 enforcement: byte-order conversion is ade-endian's alone.
├── src/               Core engine, one crate per pipeline layer.
│   ├── endian/        Big-endian ↔ host conversion. The only place it happens (C-001).
│   ├── flux/          SCP, extended-ADF, optional IPF-read, hardware isolation.
│   ├── track/         MFM encode/decode, sync words, gaps.
│   ├── block/         512-byte blocks, checksums, bitmap, bounds-checked access.
│   ├── filesystem/    OFS/FFS mount, dostypes, directory traversal, RDB partitions.
│   ├── object/        Files, directories, links, comments, metadata, salvage.
│   ├── catalogue/     Content hashing, dataset matching, reporting, export.
│   ├── container/     Front-end normalising ADF/ADZ/HDF/HDZ/DMS into the block layer.
│   └── api/           The core library seam consumed by the CLI and GUI.
├── cli/               Command-line front-end.
├── gui/               Qt6 GUI (Phase 5).
├── tests/
│   ├── fixtures/      Curated TOSEC images, labelled known-good / known-bad.
│   ├── fuzz/          Fuzz harnesses and malformed-input corpus.
│   ├── unit/          Per-layer tests.
│   └── integration/   Cross-layer and end-to-end tests.
└── tools/             Development scripts, incl. the D-003 layering check.
```

Each `src/<layer>/` directory is a crate (`ade-endian`, `ade-block`, …), and the crate dependency graph *is* the architecture: layers depend downward on abstractions only. `ade-block` defines the `BlockSource` seam; `ade-container` and `ade-track` implement it; so `ade-block` depends on neither. `ade-core` is the one crate permitted to know every layer, and the front-ends see only `ade-core`.

Two invariants are enforced by the build rather than by review:

- **C-001** (one byte-order module) — `clippy.toml` disallows `u32::from_be_bytes` and its siblings everywhere except `ade-endian`.
- **D-003** (no module spans two layers) — [`tools/check-layering.py`](tools/check-layering.py) declares each crate's permitted dependencies and fails CI on any deviation, so widening the graph is a deliberate, reviewable edit.

## Building

A Rust core with a Qt6 GUI over a C-ABI bridge — the "Pontus pattern" (D-001). Linux (x86-64) is the primary target; Windows and macOS come later. The GUI does not exist yet, so only the Rust half builds today; `cargo build --workspace` is enough.

ADFlib is *not* a build dependency. Under D-002 it is a black-box differential-test oracle, needed only to run that suite, and the suite skips rather than fails when it is absent.

Full commands, the lint set and why each lint is there, in [BUILD.md](Docs/BUILD.md).

## Documentation

| Document | Contents |
|---|---|
| [FEATURES.md](Docs/FEATURES.md) | Capability list F-001…F-018, with priorities and acceptance criteria |
| [ROADMAP.md](Docs/ROADMAP.md) | Phased plan, Phase 0…5, referencing feature IDs |
| [ARCHITECTURE.md](Docs/ARCHITECTURE.md) | Layered pipeline, module responsibilities, invariants |
| [DECISIONS.md](Docs/DECISIONS.md) | Design-decision log D-001…D-010, with reversal conditions |
| [SPEC.md](Docs/SPEC.md) | Disk and filesystem format reference; constraints C-001…C-005 |
| [ATTACK_VECTORS.md](Docs/ATTACK_VECTORS.md) | Failure modes for untrusted input, AV-001…AV-005 |
| [BUILD.md](Docs/BUILD.md) | Environment, toolchain, build commands (stub until first build) |
| [BUGS.md](Docs/BUGS.md) · [IMPROVEMENTS.md](Docs/IMPROVEMENTS.md) | In-repo bug and refactor catalogues |
| [CHANGELOG.md](Docs/CHANGELOG.md) | Version history |
| [CLAUDE.md](Docs/CLAUDE.md) | Handoff contract for AI-assisted sessions |

## Scope

ADE is a disk and filesystem tool, not an emulator. It does not emulate the Amiga CPU or chipset, and it **never executes code found on a disk image** — bootblocks included. Use WinUAE or FS-UAE for execution. IPF authoring is out of scope permanently; modern journalling filesystems (SFS, PFS) are a read-support candidate, not a v1 commitment.

## Licence

**Apache-2.0** — see [LICENSE](LICENSE) and [NOTICE](NOTICE).

The choice was coupled to D-002: wrapping ADFlib would have propagated GPL. D-002 landed on reimplementation, so no obligation is inherited and the licence was a free decision — Apache-2.0 was chosen for its explicit patent grant (ADE implements reverse-engineered and in places proprietary formats), its standing with institutional adopters, and the `NOTICE` convention for attribution. Recorded as **D-011**; **D-008**, which deferred the choice, is discharged.

`NOTICE` records that ADE contains no third-party code, and that ADFlib is a black-box test oracle rather than a dependency.

Two licence surfaces sit outside this: the optional CAPS library for IPF-read remains restrictively licensed and compile-time-gated (C-003), and D-009 may reopen the question at Phase 3. Separately, **D-010 remains an open constraint on what may be committed** — most TOSEC Amiga images are copyrighted, so `tests/fixtures/` is gated by a `.gitignore` tripwire until that decision lands.
