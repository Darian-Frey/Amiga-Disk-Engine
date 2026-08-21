# Roadmap

Phased plan for the Amiga Disk Engine. Phases are append-only; mark Complete with an ISO date, never delete. Feature IDs reference [FEATURES.md](FEATURES.md); the format detail behind each phase lives in [SPEC.md](SPEC.md).

Background context (why ADE exists, the documentation landscape survey, and the competitive gap analysis that produced the feature set) is summarised in [FEATURES.md](FEATURES.md) and the decision log; it is not repeated here.

## Phase 0 — Scaffold & decisions
**Goal:** Stand up the plan-first documentation set and resolve the two stack decisions that gate all code.
**Status:** In progress
**Features delivered:** —
**Deliverables:**
- [x] Documentation scaffold (this set) to the project-scaffold standard.
- [x] Resolve **D-001** (language/stack) — Accepted 2026-08-21 as Rust core + Qt6 over a C-ABI bridge.
- [x] Resolve **D-002** (ADFlib: wrap vs reimplement) — Accepted 2026-08-21 as reimplementation with ADFlib as a black-box test oracle. Scope narrowed to ADFlib; xDMS split out to **D-009** (Phase 2, non-blocking).
- [x] Choose a licence and add `LICENSE` — Apache-2.0, decided 2026-08-21 (**D-011**), added with a `NOTICE` before the first public commit. Discharges D-008.
- [ ] Resolve **D-010** (test-fixture provenance): decide what may lawfully be committed to `tests/fixtures/`. Now load-bearing, because D-002 makes differential testing the primary route to the edge-case coverage reimplementation forgoes.
- [ ] Acquire the fixture set per D-010: clean DD, clean HD, OFS, FFS, INTL, dircache, multi-partition HDF, plus known-bad (`errdms`/protected) nasties, and hand-authored malformed images for AV-001 / AV-004.
**Acceptance:** D-001 and D-002 Accepted in [DECISIONS.md](DECISIONS.md) *(done 2026-08-21)*; `LICENSE` present *(done 2026-08-21, D-011)*; D-010 decided and the fixture set available and labelled known-good / known-bad.

## Phase 1 — Read-only ADF core (happy path)
**Goal:** Parse and extract from plain 880 KB DD ADF images, defensively, through the chosen stack.
**Status:** Not started
**Features delivered:** F-001, F-002 (initial), F-003 (ADF/ADZ), F-010 (initial), F-015 (initial)
**Deliverables:**
- [ ] Bootblock parse + checksum; RDB detection ([SPEC.md](SPEC.md) §Bootblock, §RDB).
- [ ] Mount OFS and FFS volumes: rootblock, bitmap, hash-table directory traversal, file header → extension → data blocks, extraction.
- [ ] Single byte-order module (C-001); bounds-checked block access; directory-loop detection from the outset (AV-001, AV-004).
- [ ] Spike validated against three fixtures: one OFS DD, one FFS DD, one multi-partition HDF.
**Acceptance:** Round-trip-read extraction matches a reference tool on the clean fixtures; the fuzz corpus runs with zero crashes (F-001).

## Phase 2 — Filesystem breadth
**Goal:** Cover the full non-flux filesystem surface, including hard-disk images, plus forensic recovery.
**Status:** Not started
**Features delivered:** F-009 (initial), F-010, F-012, F-017, F-018
**Deliverables:**
- [ ] HD (1.76 MB) and 5.25" DD geometry; all dostypes (OFS/FFS × INTL × dircache) and LNFS long names.
- [ ] HDF + RDB multi-partition images; configurable block sizes (C-002, C-005).
- [ ] Links, comments, protection bits, datestamps.
- [ ] Undelete/salvage (F-012); standalone bitmap rebuild (AV-003).
**Acceptance:** All Phase-2 fixtures mount and enumerate correctly; a deleted-entry fixture is recovered.

## Phase 3 — Containers & compression
**Goal:** Absorb the compressed and packaged formats transparently, with honest lossiness reporting.
**Status:** Not started
**Features delivered:** F-003 (DMS/HDZ), F-011, F-016
**Deliverables:**
- [ ] DMS (all modes + encrypted) by whichever route D-009 settles on; ADZ/HDZ gzip at the container front-end.
- [ ] FILEID.DIZ / banner extraction; bootblock virus scanning (F-011, AV-002).
- [ ] Conversion matrix with lossy-conversion warnings (F-016, C-003, C-004).
**Acceptance:** DMS fixtures decompress to byte-correct ADFs where the source permits; `errdms` cases fail loudly, not silently.

## Phase 4 — Track / flux level
**Goal:** Handle the copy-protection frontier — the Amiga analogue of STX/Pasti — on the open flux path.
**Status:** Not started
**Features delivered:** F-003 (SCP/ext-ADF/IPF-read), F-005 (initial), F-007, F-008
**Deliverables:**
- [ ] Internal MFM track model populated (designed-for since Phase 0 per D-005); MFM encode/decode; sync-word handling.
- [ ] Extended-ADF read/write; SCP read/write (D-007); optional read-only IPF behind a licence-gated feature flag (C-003).
- [ ] Multi-read consolidation with confidence reporting (F-008); raw-track + filesystem dual view (F-007).
**Acceptance:** A protected disk survives capture → SCP → write-back and boots in an emulator; consolidation resolves a marginal multi-read fixture.

## Phase 5 — Catalogue & GUI
**Goal:** The single cross-platform application: capture-to-catalogue in one place.
**Status:** Not started
**Features delivered:** F-004, F-005, F-006, F-013, F-014
**Deliverables:**
- [ ] Qt6 GUI: tree + hex + preview, drag-drop, cross-image search (F-004).
- [ ] In-app Greaseweazle read/write (F-006); end-to-end pipeline wired (F-005).
- [ ] Auto-identification on ingest against TOSEC/WHDLoad/OpenRetro; ManifeST catalogue integration (F-013) — defines the future `VOCABULARY.md` contract.
- [ ] Corpus-scale batch operations with machine-readable summaries (F-014).
**Acceptance:** A cold user images a real disk, sees it auto-identified and catalogued, and batch-verifies a multi-thousand-image corpus in one run.
