# Architecture

Descriptive only. Rationale for these choices lives in [DECISIONS.md](DECISIONS.md); this document states the system as intended to be built. The stack is settled as of 2026-08-21: a Rust core with a Qt6 GUI over a C-ABI bridge (D-001), with OFS/FFS/RDB handling reimplemented rather than wrapped (D-002). Each pipeline layer below becomes its own crate, so the "no module spans two layers" invariant is enforced by the dependency graph rather than by review.

## System overview

ADE is a strict layered pipeline. Each layer is a separately-testable module with a trait/interface seam, so that no single component can accumulate responsibilities across layers (the god-class failure the Atari Disk Engine suffered; see D-003).

```
  ┌─────────────────────────────────────────────┐
  │ catalogue / export   (TOSEC/WHDLoad match,   │  ← F-013, F-014
  │                       ManifeST, reports)     │
  ├─────────────────────────────────────────────┤
  │ object model         (files, dirs, links,    │  ← F-012
  │                       comments, metadata)    │
  ├─────────────────────────────────────────────┤
  │ filesystem           (OFS/FFS, dostypes,     │  ← F-010, F-018
  │                       RDB partitions)        │
  ├─────────────────────────────────────────────┤
  │ sector / block       (512-byte blocks,       │  ← F-001
  │                       checksums, bitmap)     │
  ├─────────────────────────────────────────────┤
  │ track / MFM codec    (encode/decode,         │  ← F-007
  │                       sync words)            │
  ├─────────────────────────────────────────────┤
  │ flux                 (SCP, extended-ADF,     │  ← F-007, F-008
  │                       IPF-read, hardware)    │
  └─────────────────────────────────────────────┘
        ▲ container front-end normalises ADF/ADZ/HDF/HDZ/DMS
        │ into the block layer (F-003)
        ▼
  core library API  ──►  CLI  and  Qt6 GUI          ← F-002, F-004, F-015
```

Data flows upward on read (flux → catalogue) and downward on write (object model → block → track → flux/hardware). A **container front-end** sits beside the stack and normalises the packaged formats (ADF, ADZ, HDF, HDZ, DMS) into the block layer so the upper layers never see compression or wrapping.

## Module responsibilities

- **flux** — Reads/writes flux-level representations (SCP, extended-ADF) and, optionally, IPF (read-only, C-003). Talks to hardware (Greaseweazle) at Phase 5. Owns the raw-MFM-capable data model that must exist from day one (D-005) even while early phases only populate decoded sectors.
- **track / MFM codec** — Encodes and decodes MFM, locates sync words, presents decoded sectors upward and accepts sectors for encoding downward.
- **sector / block** — The 512-byte block abstraction with checksum verification and bitmap handling. Parameterised on the OFS/FFS usable-payload difference (C-005). All access is bounds-checked (AV-004).
- **filesystem** — OFS and FFS mount logic across all dostypes and long-filename variants; RDB partition parsing for hard-disk images (F-018). Directory traversal with loop detection (AV-001).
- **object model** — Files, directories, links, comments, protection bits, datestamps; the neutral representation the UI and catalogue consume. Undelete/salvage operates here (F-012).
- **catalogue / export** — Content hashing, dataset matching (TOSEC/WHDLoad/OpenRetro), ManifeST integration, batch reporting (F-013, F-014).
- **core library API** — The single seam the CLI and GUI both consume (F-002). No engine logic in UI code.

## Key invariants

1. **No god-class.** No module spans more than one pipeline layer. Cross-layer coordination happens through the core API, not by one class reaching across seams. (D-003)
2. **Big-endian discipline in one place.** All on-disk data is 68k big-endian; the host is little-endian. Every byte-order conversion routes through a single module (C-001). No ad-hoc byte-swapping elsewhere.
3. **Untrusted input.** Every image is hostile until proven otherwise. No parse path may crash, hang, or allocate unboundedly on malformed input (D-006, AV-001…AV-005).
4. **Raw-MFM-capable model from day one.** The internal representation must be able to hold a raw track even when only decoded sectors are populated (D-005), so flux support is not a bolt-on later.
5. **Read before write.** Every write path ships only after the corresponding read path is proven (D-004).
6. **One transparent open-path.** Format dispatch is by content sniffing, not file extension (F-003).

## Cross-cutting concerns

- **Error handling.** Typed, recoverable errors carrying block/offset context; no panics on data-dependent paths. The health report (F-010) is the user-facing surface of this discipline.
- **Threading / concurrency.** Batch operations (F-014) parallelise across images, not within a single image's parse (which stays deterministic for reproducibility and diffing, F-009). With the stack settled (D-001), the concrete mechanism — thread pool against async — is chosen during Phase 1 and does not need its own decision entry unless the two diverge in the API.
- **Logging.** Structured, machine-readable output for the scriptable surface (F-015); severity-tagged.
- **Hardware isolation.** All Greaseweazle interaction is confined to the flux layer so the rest of the engine is testable without a device attached.
