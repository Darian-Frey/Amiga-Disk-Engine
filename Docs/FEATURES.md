# Features

## Target users

Retro-computing preservationists, demoscene archivists, and forensic hobbyists who need to read, verify, catalogue, and write Amiga floppy and hard-disk images across platforms — from a single marginal disk up to corpora numbering in the thousands.

## Out of scope

- Full Amiga system emulation (CPU/chipset). ADE is a disk/filesystem tool, not an emulator — use WinUAE/FS-UAE for execution.
- Executing any code found on a disk image (bootblocks, binaries). ADE inspects; it never runs guest code.
- Creating IPF images. IPF authoring is closed (SPS-only); see D-007 and C-003. ADE reads IPF (optional) but writes SCP / extended-ADF as its flux path.
- Modern Amiga journalling filesystems (SFS, PFS) as a write target for v1. Read support is a candidate, not a commitment.

## Features

Priorities are MoSCoW (Must / Should / Could / Won't). Effort is rough and pre-spike (S = days, M = weeks, L = months). "Phase" references [ROADMAP.md](ROADMAP.md). All statuses are **Not started** — this is a planning-stage repository.

### F-001 Defensive, fuzz-hardened core
**Priority:** Must
**Effort:** M · **Phase:** 1 (cross-cutting)
**Acceptance:**
- No input — however malformed, truncated, or hostile — causes a crash, hang, or unbounded memory growth; failures return typed errors.
- A fuzz corpus of malformed images runs to completion with zero panics/segfaults.
- Every parse fault is reported with block/offset context, never silently swallowed.
**Status:** Not started
**Notes:** Directly answers AV-001…AV-005. The competitor baseline (adftools segfaulting on compressed input) is the bar to clear.

### F-002 Fast core + library API + CLI, GUI layered on top
**Priority:** Must
**Effort:** L · **Phase:** 0–1
**Acceptance:**
- Core exposes a documented library API consumed by both a CLI and (later) the GUI, with no engine logic living in UI code.
- No single module owns more than one pipeline layer (anti-god-class; see D-003).
**Status:** Not started
**Notes:** The "Pontus pattern" (D-001). Serves bulk-corpus and casual users from one core.

### F-003 One transparent open-path for every container
**Priority:** Must
**Effort:** M · **Phase:** 1, 3, 4
**Acceptance:**
- A single "open" entry point accepts ADF, ADZ, HDF, HDZ, DMS, SCP, and extended-ADF, and (optionally) IPF-read, dispatching by content sniffing not extension.
- Gzip-wrapped containers (ADZ/HDZ) are handled transparently.
**Status:** Not started
**Notes:** Normalises everything into the block layer. Bounded by C-003 (no IPF write) and C-004 (DMS lossiness).
**Design constraint found 2026-08-22 (C-008):** a plain ADF has **no magic number**, and an unpartitioned HDF has only the `DOS` prefix it shares with every ADF. Content sniffing cannot be a magic lookup; it is a cascade of weighted evidence. Measured against 4288 real images: 7% do not begin with `DOS` at all (144 distinct custom bootloaders), only 74% of those that do have a valid bootblock checksum, 19% of them have no rootblock at block 880, and ten non-`DOS` images mount cleanly. Sizes are not fixed either — 81-, 82- and 83-cylinder ADFs occur. The open path must report what it decided and why, so a misidentification shows up in the health report rather than silently. See SPEC §The sniffing problem and §Corpus observations.

### F-004 Single cross-platform GUI
**Priority:** Must
**Effort:** L · **Phase:** 5
**Acceptance:**
- Linux-first Qt6 GUI presents a directory tree, hex view, and file preview for any opened image.
- Drag-and-drop extraction; search across the contents of multiple loaded images.
**Status:** Not started
**Notes:** ADF Opus's never-realised cross-platform port is the gap. Qt6 over the C-ABI bridge, per D-001.

### F-005 End-to-end pipeline in one app
**Priority:** Must
**Effort:** L · **Phase:** 4–5
**Acceptance:**
- A user can go flux capture → image → filesystem browse → catalogue → write-back without leaving the app or hand-invoking a third-party tool.
**Status:** Not started
**Notes:** Depends on F-003, F-006. The current ecosystem forces a relay race across gw / disk-utilities / xDMS / a browser.

### F-006 In-app Greaseweazle read/write
**Priority:** Should
**Effort:** M · **Phase:** 5
**Acceptance:**
- Detect a connected Greaseweazle; read a disk to an image and write an image to disk from within the app.
- Drive-unit and disk-definition selection handled through the UI, not raw `--drive` / `diskdefs` flags.
**Status:** Not started
**Notes:** Demand evidenced by the third-party GUI proliferation (Desert Sage, FluxMyFluffyFloppy, EasyRead). Depends on D-007.

### F-007 Extended-ADF / SCP as a browsable first-class citizen
**Priority:** Should
**Effort:** L · **Phase:** 4
**Acceptance:**
- Open an SCP or extended-ADF and view the raw MFM track alongside the mounted, decodable filesystem in one view.
**Status:** Not started
**Notes:** Most browsers punt on extended ADF entirely. Enabled by D-005 (raw-MFM-capable model from day one).

### F-008 Multi-read flux consolidation
**Priority:** Should
**Effort:** L · **Phase:** 4
**Acceptance:**
- Merge N reads of the same disk into a best-estimate image with a per-track confidence/quality report identifying unresolved tracks.
**Status:** Not started
**Notes:** Marginal disks need repeated reads; open tooling merges them poorly. Pairs with F-009.

### F-009 Image / disk diffing
**Priority:** Should
**Effort:** M · **Phase:** 2, 5
**Acceptance:**
- Compare two images of the same title block-by-block (and file-by-file) and report differences.
**Status:** Not started
**Notes:** No convenient tool does this; essential for validating repeated reads. Uses F-008 output.

### F-010 Health / integrity report
**Priority:** Must
**Effort:** M · **Phase:** 1–2
**Acceptance:**
- Report bad/unreadable sectors, weak bits, block-checksum failures, bitmap validity, and OFS/FFS recoverability for any image.
- Problems are surfaced explicitly, never failed silently.
**Status:** Not started
**Notes:** Relates to AV-003. A core forensic differentiator.

### F-011 Bootblock virus scanning
**Priority:** Should
**Effort:** S · **Phase:** 3
**Acceptance:**
- Scan bootblocks against a signature set for known historical viruses and flag matches; never execute bootblock code.
**Status:** Not started
**Notes:** The original 1990 DMS had this; modern convenient tools dropped it. Relates to AV-002.

### F-012 GUI undelete / salvage
**Priority:** Should
**Effort:** M · **Phase:** 2
**Acceptance:**
- Detect recoverable deleted entries and restore them through the GUI, with a clear recoverability indicator.
**Status:** Not started
**Notes:** DiskSalv is Amiga-native and ancient; ADFlib's undelete is CLI-only.

### F-013 Auto-identification on ingest
**Priority:** Must
**Effort:** M · **Phase:** 5
**Acceptance:**
- On open, content-hash the image and match against TOSEC / WHDLoad / OpenRetro datasets; expose results to a ManifeST-style catalogue.
**Status:** Not started
**Notes:** Game-art lookups exist (FS-UAE/OpenRetro) but not for forensic image management. Will define the ManifeST contract (future VOCABULARY.md).

### F-014 Corpus-scale batch operations
**Priority:** Must
**Effort:** M · **Phase:** 5
**Acceptance:**
- Bulk verify / convert / catalogue / report across thousands of images in one run, with a machine-readable summary.
**Status:** Not started
**Notes:** No friendly tool scales to the 4,500-image-corpus workflow hit on the ST. Depends on F-002, F-013.

### F-015 Stable scriptable surface
**Priority:** Should
**Effort:** M · **Phase:** 1+
**Acceptance:**
- A documented, versioned CLI and library binding suitable for automation, with stable exit codes and structured output.
**Status:** In progress — exit codes and JSON output landed 2026-08-22 (IMP-001)
**What exists:** five documented exit codes distinguishing clean / faults / usage / unreadable / no-volume; `--format=json` on `info` and `ls`, the latter as JSON Lines; typed faults with stable kebab-case codes. Output is pure ASCII, so Latin-1 Amiga names round-trip losslessly. Verified across 4288 images: 68,961 JSON Lines, all valid.
**Still to come:** a library binding, and a version policy for the JSON schema itself.
**Notes:** Friendly tools aren't scriptable; the scriptable one (amitools) is slow. Depends on F-002.

### F-016 Format-conversion matrix
**Priority:** Should
**Effort:** M · **Phase:** 3–4
**Acceptance:**
- Convert any supported input container to any supported output, refusing or warning honestly where lossy or impossible (e.g. no IPF write, DMS `errdms`).
**Status:** Not started
**Notes:** Conversions are today scattered across single-purpose tools; lossiness rarely surfaced. Bounded by C-003, C-004.

### F-017 FUSE mount of an image
**Priority:** Could
**Effort:** M · **Phase:** 2+
**Acceptance:**
- Mount an ADF/HDF as a host filesystem (read at minimum) on Linux/macOS.
**Status:** Not started
**Notes:** Supersedes the separate, limited fuseadf. Lowest-priority; partially covered by the Linux AFFS driver. Candidate for cutting.

### F-018 HD / RDB multi-partition browsing & editing
**Priority:** Should
**Effort:** M · **Phase:** 2, 5
**Acceptance:**
- Parse an RDB partition table in an HDF and browse/edit each volume through the GUI.
**Status:** Not started
**Notes:** rdbtool is CLI-only; friendly multi-partition HDF editing is essentially absent. Bounded by C-002.

## Candidate features (uncommitted)

- Read support for modern journalling filesystems (SFS, PFS).
- FloppyBridge-compatible output for direct WinUAE/Amiberry use.
- Cataloguing enrichment via Demozoo / ScreenScraper (mirroring ManifeST).
- Track-level visualisation (MFM sync-word map, weak-bit heatmap).
