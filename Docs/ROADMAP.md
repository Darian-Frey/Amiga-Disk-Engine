# Roadmap

Phased plan for the Amiga Disk Engine. Phases are append-only; mark Complete with an ISO date, never delete. Feature IDs reference [FEATURES.md](FEATURES.md); the format detail behind each phase lives in [SPEC.md](SPEC.md).

Background context (why ADE exists, the documentation landscape survey, and the competitive gap analysis that produced the feature set) is summarised in [FEATURES.md](FEATURES.md) and the decision log; it is not repeated here.

## Phase 0 — Scaffold & decisions
**Goal:** Stand up the plan-first documentation set and resolve the two stack decisions that gate all code.
**Status:** Complete 2026-08-22
**Features delivered:** —
**Deliverables:**
- [x] Documentation scaffold (this set) to the project-scaffold standard.
- [x] Resolve **D-001** (language/stack) — Accepted 2026-08-21 as Rust core + Qt6 over a C-ABI bridge.
- [x] Resolve **D-002** (ADFlib: wrap vs reimplement) — Accepted 2026-08-21 as reimplementation with ADFlib as a black-box test oracle. Scope narrowed to ADFlib; xDMS split out to **D-009** (Phase 2, non-blocking).
- [x] Choose a licence and add `LICENSE` — Apache-2.0, decided 2026-08-21 (**D-011**), added with a `NOTICE` before the first public commit. Discharges D-008.
- [x] Resolve **D-010** (test-fixture provenance) — Accepted 2026-08-22 as Option E: generate fixtures in code, commit no image data at all. `tests/fixtures/` holds a manifest and documentation only.
- [x] Acquire the differential corpus — 4288 TOSEC Amiga ADF images held locally in `disks/`, outside version control. Already returned C-008, the extended-ADF layout, and confirmation of BUG-001 against 20 real images.
- [x] Build the fixture generator — `ade-fixtures` (2026-08-22). Volumes of any geometry including the 81–83-cylinder cases; all eight dostypes; OFS and FFS data layout; hash-table insertion with same-hash chaining; accurate bitmap; both checksum algorithms. `corrupt` supplies AV-001 (self-cycle and two-block cycle), AV-003, AV-004, non-`DOS` bootblocks, truncation and trailing junk. Depends on no other ADE crate, deliberately.
- [x] Validate the generator against reality — its checksum implementations agree with 3976 real `DOS` images: 3226 of 3229 well-typed rootblocks validate, and the bootblock rate reproduces the measured 74.1%. Skips cleanly when the corpus is absent.
- [x] Manifest tooling — `cargo run -p ade-fixtures --bin manifest` emits `sha256 size name` rows; SHA-256 verified against `sha256sum`. The manifest itself is written when differential tests reference specific images.
**Acceptance:** met. D-001, D-002 and D-010 Accepted in [DECISIONS.md](DECISIONS.md); `LICENSE` present (D-011); the generator produces the Phase-1 fixture set and the differential suite runs against the local corpus, skipping cleanly without it.

**Phase 0 is complete** as of 2026-08-22, bar D-009, which is scoped to Phase 2 and blocks nothing.

## Phase 1 — Read-only ADF core (happy path)
**Goal:** Parse and extract from plain 880 KB DD ADF images, defensively, through the chosen stack.
**Status:** In progress — first vertical slice landed 2026-08-22
**Features delivered:** F-001, F-002 (initial), F-003 (ADF/ADZ), F-010 (initial), F-015 (initial)
**Deliverables:**
- [x] **`ade info <image>`** — the first vertical slice, cutting container → block → endian → filesystem so every seam is exercised before integration rather than after. Reports container kind with its evidence, bootblock, and volume as independent facts (C-008), with stable exit codes (F-015).
- [x] Bootblock parse + checksum; both checksum algorithms in `ade-block::checksum` as separately named functions. RDB *detection* (parsing is Phase 2).
- [ ] Mount OFS and FFS volumes: rootblock, bitmap, hash-table directory traversal, file header → extension → data blocks, extraction.
- [x] Single byte-order module (C-001); bounds-checked block access via `ValidBlock` (AV-004).
- [x] Content sniffing as an evidence cascade, not a magic lookup (F-003, C-008): verified against 4288 real images, correctly classifying 4270 canonical ADFs, 11 extended-ADFs, 5 extra-cylinder images and 2 size anomalies.
- [x] Directory-loop detection (AV-001) — a visited set of block numbers on every chain walked: hash chains, file extension chains, and the tree walk. Not a depth limit, which cannot distinguish a deep tree from a two-block loop.
- [x] Mount and traverse: rootblock hash table, directory blocks, same-hash chains, path lookup with the volume's own case folding, and file reading through the reversed `data_blocks[]` table plus extension blocks, for both OFS and FFS.
- [x] `ade ls` and `ade extract`.
- [x] Differential testing against ADFlib (D-002's oracle) — 2894 of 2896 files byte-identical; the two differences are D-012, where ADE recovers content ADFlib refuses.
- [x] Fuzz harness at the block level (F-001) — six targets over the block parsers, the sniffer, mount/traverse, deliberately hostile structures, and truncated images. Deterministic PRNG, so a failure reproduces from its seed; runs in CI on every push.
- [ ] Health report proper (F-010): aggregate the faults `ade info` already finds.
- [ ] Spike validated against three fixtures: one OFS DD, one FFS DD, one multi-partition HDF.
**Acceptance:** Round-trip-read extraction matches a reference tool on the clean fixtures *(met: 99.36% byte-identical against ADFlib over 3900 files; every disagreement investigated and attributed to D-012 or to genuine disk damage, none to a reader fault)*; the fuzz corpus runs with zero crashes *(met 2026-08-23: 900,000 cases across six targets, zero failures)*.

**Phase 1 acceptance is met**, and the three improvements raised against it (IMP-001, IMP-002, IMP-003) are all applied. Applying the last of them uncovered **BUG-003**, a live AV-005 unbounded allocation on the plain ADF path, now fixed.

Remaining before the phase can be called complete: nothing on the criteria. Outstanding by judgement: the fuzz harness cannot see allocation that is never used (see BUG-003), so lengths read from disk that size an allocation want explicit assertions.

*Progress 2026-08-22:* `ade info` runs over all 4288 corpus images with **zero crashes, hangs or unhandled errors** — 2608 clean, 626 with faults, 1054 with no AmigaDOS volume.

Traversal and extraction then went over a 400-image sample: **295 volumes mounted, 11,087 files extracted, 180.5 MB, zero read errors**. Extracted content carries the magic numbers it should — Amiga hunk executables, PowerPacker, IFF — which is independent evidence the block ordering and OFS/FFS payload handling are right, since a reader that had those wrong would produce garbage.

Eight files of 11,087 came up short of their declared size. Each was checked against an independent Python decode and found to be **genuine on-disk inconsistency**, not a reader fault: the OFS data blocks' own `data_size` fields sum to less than the header's `byte_size`. ADE now reports the shortfall rather than padding.

Neither run is the F-001 bar, which requires a fuzz corpus rather than well-formed real images.

## Phase 2 — Filesystem breadth
**Goal:** Cover the full non-flux filesystem surface, including hard-disk images, plus forensic recovery.
**Status:** Not started
**Features delivered:** F-009 (initial), F-010 *(largely delivered early, 2026-08-24)*, F-012, F-017, F-018
**Deliverables:**
- [x] HD (1.76 MB) geometry — mounts, round-trips OFS and FFS, rootblock computed at 1760 rather than read (C-007). Extra-cylinder (81–83) geometries too. Cross-checked against ADFlib.
- [x] Dostype identification and hashing across all eight, including the international variants, cross-checked against ADFlib on accented filenames (C-006).
- [ ] 5.25" DD geometry — SPEC has no source for it; see §Open questions.
- [ ] Real dircache blocks (`DOS\4`/`\5`) and LNFS long names (`DOS\6`/`\7`). Identification works; the structures do not exist yet.
- [ ] HDF + RDB multi-partition images; configurable block sizes (C-002, C-005).
- [x] Links, comments, protection bits, datestamps. Hard links resolve through `real_entry` with a bounds check and a visited set; `ade ls` shows targets; a link to a directory is walked once rather than once per link. Fixed BUG-005 on the way — reading a hard link had silently returned an empty file. **Caveat:** `unadf` omits links from its listings, so this is the one Phase 2 area with neither an oracle nor corpus material.
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
