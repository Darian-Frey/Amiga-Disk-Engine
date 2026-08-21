> **Status:** Active
> **Provenance:** Claude (primary auditor / initial scaffolding, 2026-08-21)
> **Last reviewed:** 2026-08-21
> **Why this status:** Planning phase. Documentation set and directory skeleton in place; no code yet. The two blocking stack decisions were resolved on 2026-08-21 (D-001, D-002), so Phase 1 is unblocked; `LICENSE` and fixture provenance (D-010) remain outstanding.

# Amiga Disk Engine (ADE)

A forensic-grade, cross-platform toolkit for reading, validating, cataloguing, and writing Amiga floppy and hard-disk images — the successor to the (now complete) Atari Disk Engine, carrying its lessons forward.

ADE targets the gap no current tool fills. The capable Amiga engines (amitools, ADFlib) are CLI-only and developer-facing; the friendly tools (ADF Opus, the various Greaseweazle GUIs) are narrow, Windows-bound, or abandoned; and none joins flux capture to catalogue in a single application.

## What makes it different

- **A never-crash core.** Every image is untrusted input. No parse path may crash, hang, or allocate unboundedly, however malformed the input — and fuzzing is an acceptance bar from the first phase, not an afterthought. (F-001, D-006)
- **Capture to catalogue in one application.** Flux capture → image → filesystem browse → catalogue → write-back, without a relay race across `gw`, disk-utilities, xDMS, and a browser tab. (F-005)
- **Forensic health reporting.** Bad sectors, weak bits, checksum failures, bitmap validity, and OFS/FFS recoverability surfaced explicitly — never failed silently — with auto-identification against TOSEC / WHDLoad / OpenRetro. (F-010, F-013)
- **Corpus scale.** Bulk verify, convert, catalogue, and report across collections numbering in the thousands, with machine-readable output. (F-014)

## Status

**Planning stage — there is no runnable build yet.** The repository holds the documentation set and the directory skeleton.

The two decisions that gated all implementation were settled on 2026-08-21:

- **D-001 — stack.** Rust core exposing a C-ABI bridge, with a Qt6 GUI over it from Phase 5. Memory safety and the untrusted-input mandate (D-006) are the same decision, and a Cargo workspace makes the "no module spans two layers" invariant a matter of the dependency graph rather than of review.
- **D-002 — ADFlib.** Reimplement OFS/FFS/RDB in Rust rather than wrap the C library, using ADFlib as a **black-box differential-test oracle** — run as a separate binary, never linked, source not read. A segfault inside wrapped C is uncatchable from Rust, so wrapping would make F-001's "zero segfaults across the fuzz corpus" unclaimable. Licence freedom is preserved as a side effect.

Still open: the licence (now a free choice, and required before this repository goes public), **D-009** (xDMS's role, Phase 2, non-blocking) and **D-010** (what fixtures may lawfully be committed). See [DECISIONS.md](Docs/DECISIONS.md) and [ROADMAP.md](Docs/ROADMAP.md).

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
amiga-disk-engine/
├── README.md          This file — project landing page.
├── Docs/              Documentation set (see index below).
├── src/               Core engine, one directory per pipeline layer.
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
└── tools/             Development and corpus-maintenance scripts.
```

The directories are currently empty of build files — the skeleton was laid down before D-001 was settled, so it commits only to the layered architecture (D-003). With the stack now chosen, `src/<layer>/` becomes a Cargo workspace of `ade-*` crates: an additive change, no renames.

## Building

No build yet. The toolchain is settled (D-001): a Rust core with a Qt6 GUI over a C-ABI bridge — the "Pontus pattern". Linux (x86-64) is the primary target; Windows and macOS come later. ADFlib is *not* a build dependency; it is needed only to run the differential test suite. Exact commands and versions land in [BUILD.md](Docs/BUILD.md) when the first build succeeds.

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

Not yet chosen, but no longer constrained. The choice was coupled to D-002 — wrapping ADFlib would have propagated GPL — and D-002 landed on reimplementation, so no obligation is inherited and the licence is a free decision. **D-008**'s deferral is therefore discharged: selecting a licence is now the outstanding action rather than a pending one.

**A `LICENSE` file must be added before the first public commit**, and it is not the only gate — see D-010 on fixture provenance.
