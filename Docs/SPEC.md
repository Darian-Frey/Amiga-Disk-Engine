# Spec — Amiga disk & filesystem formats

The authoritative technical reference for the formats ADE reads and writes. This is the **how it works** (structures, constants, layouts); the **what it does** is [FEATURES.md](FEATURES.md) and the **why** is [DECISIONS.md](DECISIONS.md).

> Primary external references (to be cited inline as sections are fleshed out during implementation): Laurent Clévy's ".ADF format FAQ" (the canonical stack-wide reference), the Linux kernel AFFS driver documentation, and RKRM: Devices Appendix C for the MFM track format. This document records ADE's own working understanding; the external sources remain ground truth.
>
> **ADFlib's source is deliberately absent from that list.** Under D-002 it is a black-box test oracle only — run as a separate binary and diffed against — and reading it to inform ADE's implementation would muddy provenance and forfeit the licence freedom that decision preserves. Format knowledge comes from the documentation above.

## Layers

ADE spans six format layers (see [ARCHITECTURE.md](ARCHITECTURE.md) for the module mapping):

1. **Flux** — SCP, extended-ADF (MFM track data), optional IPF-read.
2. **Track / MFM** — MFM encoding, sync words (0x4489), track gaps.
3. **Sector / block** — 512-byte blocks, checksums, the allocation bitmap.
4. **Filesystem** — OFS/FFS, dostypes, RDB partitioning.
5. **Object model** — files, directories, links, comments, metadata.
6. **Catalogue** — content hashes and dataset identity (not a disk format, but the terminal representation).

## Geometry

- **DD floppy:** 80 cylinders × 2 heads × 11 sectors × 512 bytes = **901,120 bytes** (the canonical 880 KB ADF).
- **HD floppy:** 22 sectors/track → **1,802,240 bytes** (1.76 MB).
- Blocks are logical, numbered from 0; the rootblock sits at the volume's midpoint (block 880 for a standard DD floppy).

## Bootblock

Blocks 0–1. Holds the dostype magic and, for bootable disks, boot code and a checksum. ADE parses and checksums the bootblock and **never executes** boot code (AV-002; D-006). Virus signature scanning operates here (F-011).

## Dostypes

`DOS\0`–`DOS\7` distinguish OFS/FFS and the INTL/dircache variants. ADE must recognise all eight and mount accordingly. (Full enumeration to be tabulated against the AFFS driver docs during Phase 1/2.)

## OFS vs FFS data blocks

- **OFS** data blocks carry a 24-byte header (including a checksum and next-block pointer) → **488 usable payload bytes** per 512-byte block.
- **FFS** data blocks use the **full 512 bytes** for payload.
- The block layer is **parameterised** on this difference; it must not be hard-coded (C-005).

## Rootblock, bitmap, directories

- **Rootblock** — volume name, datestamps, hash table, bitmap pointers.
- **Bitmap** — one bit per block, allocated/free; a bitmap-valid flag indicates trustworthiness. ADE treats the flag as advisory and can rebuild the bitmap defensively (AV-003).
- **Directories** — hash table of entries with hash-chain collision resolution. Traversal must detect chain loops (AV-001).
- **Files** — file header block → chain of extension blocks → data blocks. Every pointer is bounds-checked against device geometry before dereference (AV-004).

## Hard disks — RDB / HDF

- **HDF** is a raw hard-disk image; an **RDB** (Rigid Disk Block) near the start describes partitions, geometry, and filesystem drivers.
- ADE parses the RDB partition table and mounts each volume independently (F-018), supporting configurable block sizes (512/1K/2K/4K).

## Containers & compression

- **ADZ / HDZ** — gzip-wrapped ADF/HDF; handled transparently at the container front-end.
- **DMS (DiskMasher, `DMS!` magic)** — proprietary, fully reverse-engineered by xDMS across all compression modes including encryption. Whether ADE ports, wraps, or reimplements xDMS is **open (D-009)**, deferred to Phase 2 and turning on xDMS's licence; the lean is a port to safe Rust. Some DMS images are known-bad and will not round-trip (C-004).

## Flux formats

- **Extended-ADF** — carries raw MFM track data for non-standard/protected disks; plain ADF cannot.
- **SCP (SuperCard Pro)** — the open, documented flux container and ADE's write target for protected/flux content (D-007).
- **IPF (`CAPS␀␀␀` magic)** — stores flux-transition timings. Reading requires the closed CAPS library; creation is SPS-only. ADE supports IPF **read-only and optional**, behind a licence-gated feature flag; it **cannot emit IPF** (C-003).

## Format constraints (C-NNN)

Stable, append-only IDs; referenced from [ARCHITECTURE.md](ARCHITECTURE.md), [DECISIONS.md](DECISIONS.md), and [ATTACK_VECTORS.md](ATTACK_VECTORS.md).

- **C-001 — Endianness.** All on-disk data is 68k **big-endian**; the host is little-endian. Every conversion routes through one byte-order module. (ARCHITECTURE invariant 2.)
- **C-002 — FFS 32-bit limit.** FFS addresses ~4 GB max; TD64/NSD and third-party 64-bit patches exist and are mutually incompatible. HDF handling must detect which scheme an image uses.
- **C-003 — No IPF creation.** IPF authoring is closed (SPS-only) and the CAPS read library is restrictively licensed. ADE reads IPF (optional) but never writes it. (D-007.)
- **C-004 — DMS is buggy.** Some DMS images (`errdms` in TOSEC) will not round-trip. ADE surfaces this honestly rather than producing a silently-bad ADF.
- **C-005 — OFS/FFS payload difference.** OFS data blocks carry 488 usable bytes (24-byte metadata + checksum); FFS uses the full 512. The block layer is parameterised on this.
