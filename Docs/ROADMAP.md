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
**Status:** In progress
**Features delivered:** F-009 (initial), F-010 *(largely delivered early, 2026-08-24)*, F-012, F-017, F-018 *(reading half delivered 2026-08-24; the GUI half waits on Phase 5)*
**Deliverables:**
- [x] HD (1.76 MB) geometry — mounts, round-trips OFS and FFS, rootblock computed at 1760 rather than read (C-007). Extra-cylinder (81–83) geometries too. Cross-checked against ADFlib.
- [x] Dostype identification and hashing across all eight, including the international variants, cross-checked against ADFlib on accented filenames (C-006).
- [ ] 5.25" DD geometry — SPEC has no source for it; see §Open questions.
- [x] **Real dircache blocks (`DOS\4`/`\5`)** — the chain hanging off each directory's `extension` field is parsed, its blocks counted as reachable, and its records compared against the entries the hash chains reach. **This fixed a live false positive**: every `DOS\5` disk in the corpus was reporting its own cache blocks as orphaned, 19 of them on the Workbench 3.1 install disk. Validated two ways — 1252 records across 133 cache blocks on the corpus's 21 cached disks, with zero disagreements, and `unadf -c` (which lists *from* the cache) reading generated `DOS\4` and `DOS\5` volumes identically to their hash-chain listings. `DOS\4` exists nowhere in the corpus, so the generator is its only source.
- [x] **LNFS long names (`DOS\6`/`\7`) — deferred by D-013**, on verifiability rather than effort. The field-level pass SPEC asked for is done, so the layout is ready to implement; what is missing is any way to check the result. LNFS is absent from the corpus *and* unimplemented in ADFlib, which decodes `DOS\6`/`\7` by bit pattern and reports them as dircache volumes, and [AOS-LNFS] declares no long-name *file* header, so those fields would be placed by inference. Identification and hashing are correct and pinned by `dostype_lnfs.rs`; a volume that uses the layout now reports `lnfs-unsupported` rather than presenting names it cannot have read correctly. One real `DOS\6`/`DOS\7` image reverses the decision.
- [x] **HDF (unpartitioned hardfiles)** — mount, list, extract, health-check. A raw volume carries no geometry of its own: heads and sectors are a convention of whatever wrote it, and only the block count matters, since that fixes where the rootblock sits (C-007). ADFlib takes the same view, calling an 8 MB hardfile "Cylinders = 16384, Heads = 1, Sectors = 1". Cross-checked against `unadf`.
- [x] **RDB multi-partition images** — `RDSK` located by searching the first 16 blocks rather than assumed at 0, the `PART` chain walked with cycle detection, and each partition presented as a `Window` that re-validates every translated block number, so AV-004 holds across the partition boundary as well as inside it. `ade info` reports the table; `ls`, `extract` and `check` take `--partition=` by drive name or index. Two units the format makes easy to get wrong have their own tests: `SizeBlock` counts **longs**, and RDB lists terminate at **-1**, not 0. Cross-checked against ADFlib, which reads both partitions of a generated device at the same sector ranges we do. Scanning the corpus found the `RDSK` signature in the first 16 blocks of **0 of 4,652** images, so the search costs real floppies nothing.
- [ ] Configurable block sizes (C-002, C-005). Each partition's `SizeBlock` is parsed and honoured, but every fixture and every corpus image is 512 bytes, so larger blocks are untested.
- [x] Links, comments, protection bits, datestamps. Hard links resolve through `real_entry` with a bounds check and a visited set; `ade ls` shows targets; a link to a directory is walked once rather than once per link. Fixed BUG-005 on the way — reading a hard link had silently returned an empty file. **Caveat:** `unadf` omits links from its listings, so this is the one Phase 2 area with neither an oracle nor corpus material.
- [x] Standalone bitmap rebuild (AV-003) — `Bitmap::rebuild` computes the map a volume *should* have from the blocks its tree reaches, and the health report names the specific blocks that disagree in each direction. **Computed and offered, never written**: D-004 defers write paths to Phase 4 and is marked never-reversible in v1, and AV-003 asks for a rebuild to be *offered*. Proved correct without touching a disk — build it, read it back, confirm it describes the set it was built from.
- [ ] Undelete/salvage (F-012). **Blocked on material**: the corpus holds no recoverable deleted entries — zero intact deleted file headers across 90 disks, since mastered game disks have no editing history. Fixture-only, and the oracle cannot help either. Worth deferring until there is something real to recover.
**Acceptance:** All Phase-2 fixtures mount and enumerate correctly; a deleted-entry fixture is recovered.

## Phase 3 — Containers & compression
**Goal:** Absorb the compressed and packaged formats transparently, with honest lossiness reporting.
**Status:** In progress — everything unblocked is delivered; DMS remains blocked on material (D-009)
**Features delivered:** F-003 (DMS/HDZ), F-011, F-016
**Deliverables:**
- [x] **ADZ / HDZ gzip at the container front-end** — a DEFLATE decoder ([RFC 1951]) and gzip wrapper ([RFC 1952]) written from the specifications, since the workspace has no dependencies. An ADZ now inspects, mounts, lists, extracts and health-checks exactly as the ADF inside it, with the wrapper reported rather than hidden. Verified against the system `gzip` on real corpus images — byte-identical, which is a stronger oracle than D-002's, because there is no interpretive gap to adjudicate. AV-005 is discharged for this path: a 970 KB stream expanding to 1 GB is refused at a peak RSS of 515 MB against a 512 MiB cap, and the cap is checked before every write rather than after.
- [ ] DMS (all modes + encrypted) by whichever route D-009 settles on. **Still blocked on test data** — there is not one DMS file to work from, and D-009 records why writing a decompressor with nothing to decompress is the failure D-002 and D-010 exist to prevent.
- [x] **Banner extraction** (F-011) — printable text is pulled from the boot code and reported, filtered so it is prose rather than 68k opcodes (`NqNqNq` is three `NOP`s). It carries publisher banners, Copylock notices, cracker signatures and virus-protector menus: catalogue material as much as security. ~20% of corpus disks yield text after filtering; 91% of kept runs contain a space, the measure used to separate prose from code.
- [x] **AV-002's primary defence recorded as structural** — ADE has no execution path of any kind, pinned by tests using boot code written to be hostile to a reader.
- [ ] **Bootblock virus scanning — deferred by D-014**, on verifiability. No checkable signature set is published, and matching strain names is *measurably inverted*: all 107 corpus disks naming a virus carry anti-virus bootblocks, so a name-matcher would flag exactly the disks that were cleaned. A published CRC set or a real infected disk reverses it.
- [x] **FILE_ID.DIZ surfacing** — `ade info` reads the disk's own description and prints it. Rare (0.6% of a 500-disk sample) but disproportionately informative: one corpus disk is labelled `Empty` while its `FILE_ID.DIZ` reads `TEST MATCH CRiCKET SAVE DISK (NEEDED) [2/2]`. Found case-insensitively — all of `file_id.diz`, `FILE_ID.DIZ` and `File_ID.Diz` occur (C-006) — read from the root only, capped at 8 KB because the length comes off the disk, and decoded as Latin-1 so the release art survives.
- [x] **Conversion matrix** (F-016) — every pair of formats has an explicit answer carrying its own reason: lossless, lossy and what is lost, not implemented and why, or refused and why. `ade formats` prints it; `ade convert` obeys it. The distinction between *refused* and *not implemented* is load-bearing — IPF output is a licence decision that does not expire (C-003), while DMS input is a gap waiting on material (D-009), and the two invite very different follow-up.
  **Lossy conversions are refused outright, not warned about.** A warning nobody reads is how the loss happens, and flattening an extended ADF or an SCP capture to sectors discards the copy protection that was the reason to capture it. No flag enables it today.
  Conversions that run: the decompression direction (ADZ→ADF, HDZ→HDF), whose reader is proven byte-identically, and copies between sector containers, which differ only by naming convention. An existing output is never overwritten — a conversion that replaces a source image is exactly the irreversible damage D-004 is about.
**Acceptance:** DMS fixtures decompress to byte-correct ADFs where the source permits; `errdms` cases fail loudly, not silently. **Met for gzip already**: real images round-trip byte-identically, and corrupt ones fail with a named reason rather than yielding plausible bytes.

[RFC 1951]: https://www.rfc-editor.org/rfc/rfc1951
[RFC 1952]: https://www.rfc-editor.org/rfc/rfc1952

## Phase 4 — Track / flux level
**Goal:** Handle the copy-protection frontier — the Amiga analogue of STX/Pasti — on the open flux path.
**Status:** In progress — the extended-ADF container reads; MFM decoding is next
**Features delivered:** F-003 (SCP/ext-ADF/IPF-read), F-005 (initial), F-007, F-008
**Deliverables:**
- [x] **Extended-ADF reading** — the track table parses, each track's kind and extent are reported, and damage is a fault rather than a failure. All eleven corpus images parse; `Demolition.adf` is genuinely truncated 25,732 bytes short and yields 163 of its 166 tracks instead of nothing.
  Two readings a plausible implementation gets wrong, both now measured and pinned: **`space` is the file allocation and `length` the meaningful extent** — `length` is 45056 bits for all 428 type-0 tracks while `space` is 5632, 12650 or 12668 depending on the writer — and **a track may be empty**, 154 having both fields zero.
  Verified rather than merely fitted: assembling the type-0 tracks of `Demolition` into a plain ADF produces a mountable volume named `Demolition`, and `Realm of the Trolls` likewise. Arithmetic can be satisfied by a wrong reading; a legible rootblock at block 880 cannot.
- [ ] **MFM decode**, and the internal track model it populates (D-005). The raw tracks are reachable but not yet decoded. SPEC has no MFM section: [RKRM] Appendix C is the source and has not been consulted (§Open questions). The verification story is unusually good — an Amiga sector carries its own header and data checksums, so a correct decode is self-evidencing.
- [ ] Extended-ADF write; SCP read/write (D-007); optional read-only IPF behind a licence-gated feature flag (C-003).
- [ ] **Consider FDI** as a second licence-free flux format alongside SCP. It stores raw low-level track data like IPF, but its specification is public and its tools are open source, where C-003 forbids ADE from ever emitting IPF — so it is the only route to *writing* an interchange format that carries copy protection. Not committed; see SPEC §FDI is the licence-free flux format. Its magic bytes need verifying first.
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
