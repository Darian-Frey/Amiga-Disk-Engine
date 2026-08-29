# Features

## Target users

Retro-computing preservationists, demoscene archivists, and forensic hobbyists who need to read, verify, catalogue, and write Amiga floppy and hard-disk images across platforms — from a single marginal disk up to corpora numbering in the thousands.

## Out of scope

- Full Amiga system emulation (CPU/chipset). ADE is a disk/filesystem tool, not an emulator — use WinUAE/FS-UAE for execution.
- Executing any code found on a disk image (bootblocks, binaries). ADE inspects; it never runs guest code.
- Creating IPF images. IPF authoring is closed (SPS-only); see D-007 and C-003. ADE reads IPF (optional) but writes SCP / extended-ADF as its flux path.
- Modern Amiga journalling filesystems (SFS, PFS) as a write target for v1. Read support is a candidate, not a commitment.

## Features

Priorities are MoSCoW (Must / Should / Could / Won't). Effort is rough and pre-spike (S = days, M = weeks, L = months). "Phase" references [ROADMAP.md](ROADMAP.md).

**Statuses audited 2026-08-28** against what the built binaries actually do, command by command, rather than against ROADMAP's phase notes. Nine of them said "Not started" for work that had shipped, and two things the audit found are worth stating here because they are easy to re-import: ROADMAP's per-phase **"Features in scope"** line is a scope list, not a delivery record — it names features the phase covers, including ones not begun — and a feature can be *delivered* and *not met* at once, because a decision has since ruled part of its acceptance out (F-011).

Status vocabulary: **Not started** | **In progress** | **Partially delivered** (some acceptance clauses met, the rest identified) | **Delivered** (every clause met) | **Met** (delivered and verified against something external).

### F-001 Defensive, fuzz-hardened core
**Priority:** Must
**Effort:** M · **Phase:** 1 (cross-cutting)
**Acceptance:**
- No input — however malformed, truncated, or hostile — causes a crash, hang, or unbounded memory growth; failures return typed errors.
- A fuzz corpus of malformed images runs to completion with zero panics/segfaults.
- Every parse fault is reported with block/offset context, never silently swallowed.
**Status:** Met for the ADF read path, 2026-08-23
**Evidence:** 900,000 fuzz cases across six targets, zero failures; 4652 real images inspected with zero crashes; 12,468 files extracted from a 400-image sample with two read errors and no panics. The harness is hand-rolled rather than `cargo-fuzz` because the latter needs nightly and CI runs stable — a harness CI cannot run protects nothing against regressions, which is most of what a fuzzer does after the first round.
**Caveat:** unbounded *growth* is caught by structural bounds on outputs rather than by measuring the heap, since the workspace forbids the `unsafe` an allocator hook needs. IMP-003 records where that leaves a thin margin.
**Notes:** Directly answers AV-001…AV-005. The competitor baseline (adftools segfaulting on compressed input) is the bar to clear.

### F-002 Fast core + library API + CLI, GUI layered on top
**Priority:** Must
**Effort:** L · **Phase:** 0–1
**Acceptance:**
- Core exposes a documented library API consumed by both a CLI and (later) the GUI, with no engine logic living in UI code.
- No single module owns more than one pipeline layer (anti-god-class; see D-003).
**Status:** Met 2026-08-27
**Evidence:** `ade-core` is the documented library API; the `ade` CLI and the Qt6 GUI both consume it and nothing else — the GUI reaches it through the C ABI (`ade-bridge`), which `tools/check-layering.py` holds to depending on `ade-core` alone. The anti-god-class clause is machine-checked across all 12 crates on every push, not asserted: a cross-layer dependency fails CI.
**What proves the "no engine logic in UI" half** is the GUI's search. It needed a whole-volume walk, and the walk went into the ABI rather than into Qt, because traversal carries cycle detection. The rule held under pressure from a real feature, which is the only test of it that counts.
**Notes:** The "Pontus pattern" (D-001). Serves bulk-corpus and casual users from one core.

### F-003 One transparent open-path for every container
**Priority:** Must
**Effort:** M · **Phase:** 1, 3, 4
**Acceptance:**
- A single "open" entry point accepts ADF, ADZ, HDF, HDZ, DMS, SCP, and extended-ADF, and (optionally) IPF-read, dispatching by content sniffing not extension.
- Gzip-wrapped containers (ADZ/HDZ) are handled transparently.
**Status:** Partially delivered — six of eight containers open
**What opens and mounts:** ADF (DD, HD and 81–83-cylinder), ADZ, HDF, HDZ, extended ADF, and **SCP flux** (2026-08-28). Dispatch is by content sniffing, and the evidence for the decision is printed.
**What is recognised but not read:** DMS and IPF. Both sniff correctly and say so rather than failing — the honest half of the acceptance clause working, though the clause itself is not yet met.
**Why each is outstanding:** DMS waits on D-009 and material (only HEAVY2 files were ever found); IPF stays licence-gated behind C-003 and D-007, and Greaseweazle refusing IPF as an output format corroborates that independently.
**Notes:** Normalises everything into the block layer. Bounded by C-003 (no IPF write) and C-004 (DMS lossiness).
**Design constraint found 2026-08-22 (C-008):** a plain ADF has **no magic number**, and an unpartitioned HDF has only the `DOS` prefix it shares with every ADF. Content sniffing cannot be a magic lookup; it is a cascade of weighted evidence. Measured against 4288 real images: 7% do not begin with `DOS` at all (144 distinct custom bootloaders), only 74% of those that do have a valid bootblock checksum, 19% of them have no rootblock at block 880, and ten non-`DOS` images mount cleanly. Sizes are not fixed either — 81-, 82- and 83-cylinder ADFs occur. The open path must report what it decided and why, so a misidentification shows up in the health report rather than silently. See SPEC §The sniffing problem and §Corpus observations.

### F-004 Single cross-platform GUI
**Priority:** Must
**Effort:** L · **Phase:** 5
**Acceptance:**
- Linux-first Qt6 GUI presents a directory tree, hex view, and file preview for any opened image.
- Drag-and-drop extraction; search across the contents of multiple loaded images.
**Status:** Delivered 2026-08-27 (Linux; Windows and macOS builds untried)
**Notes:** ADF Opus's never-realised cross-platform port is the gap. Qt6 over the C-ABI bridge, per D-001. Both acceptance clauses met: an image drops in, a file drags out, and search covers every open image at once. 28 headless tests run without an X server.
**The hex pane's selection is clamped to one field** (2026-08-29). A hex dump is three columns pretending to be one line of text, and ordinary text selection does not know that: dragging down two lines in the hex field takes the end of that line's hex, then the ASCII column, then the next line's offset, and only then more hex — so copying a screenful of hex was impossible, which is one of the two things anybody does with a hex view. A drag now stays in the field it began in, including when the pointer wanders out of it, since a selection that changes meaning underneath the pointer is the fault being fixed. Copying returns what was highlighted and nothing else, one clipboard line per line on screen.
**A selection in one field marks the same bytes in the other.** Hex and characters are two readings of the same bytes, and *which bytes* is the question a hex view exists to answer — so selecting hex marks the characters those bytes spell, and selecting characters marks their hex. The mark is **deliberately weaker than the selection**, a paler wash with the text colour left alone: painted alike the two fields would look equally selected and nothing on screen would say which one Ctrl+C is about to copy, which replaces one ambiguity with a worse one. Selecting *offsets* marks nothing, because that selects whole lines and both other fields would then be "corresponding" — marking every column of every line says nothing the highlighted offsets have not already said.
**What that took away is put back in the context menu.** Clamping removes the only way there was to copy a line as it appears, offsets and all, which is what somebody pasting into a bug report wants — so "Copy Whole Lines" sits beside "Copy", rather than behind a modifier key nobody would guess. `copy()` and `selectAll()` are not virtual, so Ctrl+C, Ctrl+A and the context menu are all intercepted rather than redirected; the standard menu's Copy would have returned an empty clipboard, because the selection this pane draws is not the document's. Keyboard selection with shift and the arrows is deliberately left alone: making it agree would mean reimplementing cursor movement over a layout the document does not describe, and the drag is where the problem bites.
**Null bytes in the hex field are dimmed** (2026-08-29), carried over from the Atari Disk Engine, where it makes real data stand out from the padding and empty space that fills most of a disk. Two details are not cosmetic. The dimming is placed by **column arithmetic, not by searching each line for `00`** — a search would dim the `00000000` offset at the start of every line, and would dim the ASCII column of any file containing the characters `00`, which is the opposite of making data stand out. And the grey is **blended from the palette** rather than hardcoded: the Atari engine picks between two constants with a hand-written light/dark test, which is invisible on a theme neither anticipated. `hexview::dump` and `HexNullDimmer` live in one file because the highlighter has to know the dump's layout, and kept apart a change to one would silently misplace the other.
**Design consequence found 2026-08-27:** searching means walking a disk, and safe traversal is cycle detection (AV-001) and a depth bound (IMP-003) — engine logic, not GUI logic. The capability was therefore added to the C ABI as `ade_walk_open` rather than written in Qt, and the header says so, because the same reasoning applies to every future front end. A binding that is missing a safe primitive is an invitation to reimplement it unsafely.
**Not cross-platform yet in fact, only in construction.** Qt6 and the C ABI are what make the other platforms possible; nothing but Linux has been built or run.

### F-005 End-to-end pipeline in one app
**Priority:** Must
**Effort:** L · **Phase:** 4–5
**Acceptance:**
- A user can go flux capture → image → filesystem browse → catalogue → write-back without leaving the app or hand-invoking a third-party tool.
**Status:** Not started as a pipeline — four of its five stages exist separately
**Where it stands:** image, filesystem browse and catalogue all work, in both the CLI and the GUI; capture and write-back need the Greaseweazle (F-006), and nothing chains the stages into one flow. The feature is a *sequence*, so having the pieces is not having it.
**Notes:** Depends on F-003, F-006. The current ecosystem forces a relay race across gw / disk-utilities / xDMS / a browser.

### F-006 In-app Greaseweazle read/write
**Priority:** Should
**Effort:** M · **Phase:** 5
**Acceptance:**
- Detect a connected Greaseweazle; read a disk to an image and write an image to disk from within the app.
- Drive-unit and disk-definition selection handled through the UI, not raw `--drive` / `diskdefs` flags.
**Status:** Not started — blocked on hardware
**Blocked on:** a physical Greaseweazle board. The host tools are installed and verified as an SCP oracle (2026-08-27), which unblocked the *format*; detecting and driving a device cannot be written against nothing.
**Notes:** Demand evidenced by the third-party GUI proliferation (Desert Sage, FluxMyFluffyFloppy, EasyRead). Depends on D-007.

### F-007 Extended-ADF / SCP as a browsable first-class citizen
**Priority:** Should
**Effort:** L · **Phase:** 4
**Acceptance:**
- Open an SCP or extended-ADF and view the raw MFM track alongside the mounted, decodable filesystem in one view.
**Status:** Delivered 2026-08-28 — both halves of the clause
**Evidence:** one `ade info` on an extended ADF *or* an SCP reports the track table, how many tracks are raw MFM, how many decoded, and the volume mounted from them. A 30 MB flux capture of `1000cc Turbo` shows its capture parameters, 160 of 160 tracks decoded, 1760 sound sectors, 100% recovered and the rootblock legible at 880 — in 0.4 seconds. `ls`, `check` and `extract` then work on it like any disk. The raw and filesystem views are the same report, which is the clause.
**Always says it is a reconstruction.** Undecodable sectors are zeros, and zeros make a listing quietly omit half a disk, so no assembled volume is ever mounted without `sectors_placed` alongside it, and `check` raises `volume-reconstructed` as an explicit finding.
**A flux capture reports how it was captured**, separately from what it holds: revolutions stored, resolution, RPM, index alignment, and — the line a preservationist should read twice — whether the timings were **normalised**, since a normalised capture has already had the jitter that carries weak bits averaged out of it.
**Notes:** Most browsers punt on extended ADF entirely. Enabled by D-005 (raw-MFM-capable model from day one).

### F-008 Multi-read flux consolidation
**Priority:** Should
**Effort:** L · **Phase:** 4
**Acceptance:**
- Merge N reads of the same disk into a best-estimate image with a per-track confidence/quality report identifying unresolved tracks.
**Status:** Delivered 2026-08-26 — `ade consolidate`
**What exists:** N dumps merge per sector, with a per-track report naming the disputed tracks and how the votes fell.
**It reports agreement, not correctness**, and the distinction is not pedantic. The clause assumes N reads of one physical disk, where disagreement means a read failed. The corpus's multi-dump titles are independent dumps of possibly *different copies* — several TOSEC-tagged `[m ...]`, i.e. deliberately edited — and one pair differs in exactly one sector: the rootblock, by 17 bytes of volume datestamp, with neither dump wrong. So it declines to call a winner correct, and **two dumps cannot vote at all**, since every disagreement ties by definition.
**Notes:** Marginal disks need repeated reads; open tooling merges them poorly. Pairs with F-009.

### F-009 Image / disk diffing
**Priority:** Should
**Effort:** M · **Phase:** 2, 5
**Acceptance:**
- Compare two images of the same title block-by-block (and file-by-file) and report differences.
**Status:** Delivered 2026-08-26 — `ade diff`
**What exists:** two images compare block by block, with the differing blocks named and located rather than a bare "they differ". Text output only; `--format=json` is not honoured here (see F-015).
**Notes:** No convenient tool does this; essential for validating repeated reads. Uses F-008 output.

### F-010 Health / integrity report
**Priority:** Must
**Effort:** M · **Phase:** 1–2
**Acceptance:**
- Report bad/unreadable sectors, weak bits, block-checksum failures, bitmap validity, and OFS/FFS recoverability for any image.
- Problems are surfaced explicitly, never failed silently.
**Status:** Largely met 2026-08-24 — `ade check`
**What exists:** container identification with its evidence; bootblock and rootblock condition; per-entry and per-data-block checksum failures; a bitmap cross-check against the blocks the tree actually reaches, in both directions; cross-linked block detection; file shortfalls and OFS structural faults. Findings carry a stable code and a severity, and the report is available as text or JSON.
**Deliberately absent:** bad sectors and weak bits. Both are flux-level properties — a sector is "bad" because it will not read off the physical medium, and weak bits exist only in a flux capture. Neither is knowable from a decoded image, so both wait for Phase 4 rather than being faked.
**Measured:** over a 776-disk sample — 472 clean, 112 with warnings, 187 holding no AmigaDOS volume, and 5 where a finding would lose data.
**Notes:** Relates to AV-003. A core forensic differentiator.

### F-011 Bootblock virus scanning
**Priority:** Should
**Effort:** S · **Phase:** 3
**Acceptance:**
- Scan bootblocks against a signature set for known historical viruses and flag matches; never execute bootblock code.
**Status:** Half delivered, half **declined by D-014** — the acceptance clause as written will not be met
**What ships:** boot-code text extraction. `ade info` prints the printable runs found in a bootblock, filtered so the result is prose rather than 68k opcodes — `NqNqNq` is `NOP` repeated, and 91% of kept runs contain a space. On a cracked disk it recovers exactly what it should: *"DEFJAM and CCS Proudly Present … CRACKED BY: -IL SCURO-"*.
**What will not ship in this form:** matching those strings against virus names. It is *measurably backwards* — every corpus disk naming a strain carries an **anti-virus** bootblock, because cracking groups shipped virus killers. ADE reports the text and draws no verdict (D-014).
**AV-002's real defence is structural**, not a scanner: ADE has no execution path of any kind, pinned by tests using boot code written to be hostile to an interpreter. Revisiting this needs a checkable signature set to exist, which is D-014's reversal condition.
**Notes:** The original 1990 DMS had this; modern convenient tools dropped it. Relates to AV-002.

### F-012 GUI undelete / salvage
**Priority:** Should
**Effort:** M · **Phase:** 2
**Acceptance:**
- Detect recoverable deleted entries and restore them through the GUI, with a clear recoverability indicator.
**Status:** Not started — blocked on material, not effort
**Blocked on:** something to recover. A survey of 90 corpus disks found **zero intact deleted file headers**: mastered game and application disks have no editing history, so the case the feature exists for does not occur in the corpus. Writing a salvager with nothing to salvage means testing it only against fixtures built from the same assumptions as the code — the trap D-002 was written to avoid.
**What would unblock it:** disks with a real editing history (user data disks, work disks), or a decision to accept fixture-only evidence, which is a DECISIONS entry rather than a coding choice.
**Notes:** DiskSalv is Amiga-native and ancient; ADFlib's undelete is CLI-only.

### F-013 Auto-identification on ingest
**Priority:** Must
**Effort:** M · **Phase:** 5
**Acceptance:**
- On open, content-hash the image and match against TOSEC / WHDLoad / OpenRetro datasets; expose results to a ManifeST-style catalogue.
**Status:** Delivered 2026-08-29 for TOSEC; the other two datasets are settled by D-016
**What exists:** `ade identify --datfiles=DIR <image>...` and `ade batch --datfiles=DIR`, indexing Logiqx datfiles by CRC32. **4586 of 4652 corpus images identified (98%)** from 88,921 entries across 98 datfiles, recovering the full TOSEC name a renamed file has lost.
**Several matches usually means duplicate names, not doubt.** Checked 2026-08-29, correcting a claim this entry carried since 2026-08-27: **77 groups of entries share a CRC32 and a size, and every member of every group carries the same SHA-1 and the same MD5** — duplicate content under different names (the same CD audio track as track 6 and track 10, the same ISO in two sets), not collisions. There are **zero** CRC32 collisions in the set, and not one of the 77 groups involves an `.adf`. So `identify` returns every match and, where there is more than one, says which kind of several it is — `duplicated` (every name correct), `collision` (different content claiming one hash: the disk is none of them), or `unverified` (the dataset gives no SHA-1 to tell). SHA-1 is computed only when more than one candidate survives, which over the whole corpus is never.
**WHDLoad and OpenRetro are settled by D-016, and neither by writing code.** WHDLoad's obtainable database keys on the SHA-1 of a *WHDLoad LHA archive*, not a disk image, so matching a corpus of ADFs against it would produce zero matches by construction — it is the wrong kind of dataset, not a missing one. OpenRetro is the right kind, recording ADF and IPF variants by SHA-1 of the disk file, but its bulk data is behind an authenticated `/api/sync` with no public export.
**The ManifeST contract is written** — [VOCABULARY.md](VOCABULARY.md), 2026-08-29, against ManifeST's own `DiskRecord.hpp` rather than against an idea of one. It maps ADE's terms onto a cataloguer's fields, and names the three ADE deliberately will not fill: parsing a dataset name into title/publisher/year (the cataloguer already does it, and two implementations drift), guessing a launcher, and drawing conclusions from boot text (D-014).
**Two fields were added to close it:** `container_code`, a stable code where `container` is a sentence, and `sha1` behind `--hash` — the key a catalogue finds duplicates with, opt-in because SHA-1 runs at 349 MB/s and would add twelve seconds to a five-second corpus pass. `ls --hash` hashes each file for the same reason ManifeST has a deep mode.
**Identification on open landed 2026-08-29**, and the acceptance clause is met: `ade info`, `ade check` and the GUI name a disk as it opens, from the bytes already read.
**It is configured, not automatic, and the measurement is why.** Loading 88,921 entries takes **140 ms** where `ade info` itself takes under ten — identifying unconditionally would make the fastest command in the tool fourteen times slower for everyone, including the corpus scripts that call it thousands of times. So a dataset is looked for in `--datfiles=`, then `$ADE_DATFILES`, then `$XDG_DATA_HOME/ade/datfiles`, and when none is configured nothing is loaded and nothing costs anything.
**The GUI is where it pays**: the dataset loads once for the session and every image opened afterwards arrives already named, which is the shape the clause describes. Scripted use over a corpus should still use `batch --datfiles=`, which loads once rather than once per image — a second against thirteen minutes over 4,652 images.
**Notes:** Game-art lookups exist (FS-UAE/OpenRetro) but not for forensic image management.

### F-014 Corpus-scale batch operations
**Priority:** Must
**Effort:** M · **Phase:** 5
**Acceptance:**
- Bulk verify / convert / catalogue / report across thousands of images in one run, with a machine-readable summary.
**Status:** Delivered 2026-08-29 — `ade batch`
**What exists:** verify, catalogue and report across a whole corpus in one run, text or JSON (records as JSON Lines, then a summary object). **The 4652-image corpus runs in 5.5 seconds at 9 MB peak**, because one image is read, examined and dropped — memory does not scale with the corpus. Nothing aborts a run: an unreadable image becomes a record, since a pass over four thousand disks that stops at the first bad one has reported on one disk.
**Bulk convert landed 2026-08-29.** `ade batch --convert=<code> --output=<dir>` converts a whole corpus in the same pass as the health check, from the bytes already read — converting separately would read all 4.2 GB a second time to produce a report ADE has just produced. 400 real images convert to extended ADF in **1.36 s at 9.7 MB peak**, and 30 spot-checked outputs list identically to their sources.
**Nothing aborts a run, F-016's rules intact.** A corpus is heterogeneous: one target is lossless for most images, `lossy` for the flux captures, `not-implemented` for DMS. Each outcome is reported per image with its reason and counted in the summary; a run that stopped at the first refusal would convert nothing. Existing outputs are never overwritten — reported as `exists` and left alone, which in bulk is the difference between one mistake and four thousand.
**A histogram counts images, not occurrences** — one damaged disk raising a code fifty times is one affected disk. Getting that wrong made 186 images read as 1050.
**Notes:** No friendly tool scales to the 4,500-image-corpus workflow hit on the ST. Depends on F-002, F-013.

### F-015 Stable scriptable surface
**Priority:** Should
**Effort:** M · **Phase:** 1+
**Acceptance:**
- A documented, versioned CLI and library binding suitable for automation, with stable exit codes and structured output.
**Status:** Met 2026-08-28
**What exists:** five documented exit codes distinguishing clean / faults / usage / unreadable / no-volume; `--format=json` on **every command that reports** — `info`, `ls`, `check`, `batch`, `diff`, `consolidate`, `identify` and `formats`; typed faults with stable kebab-case codes; a stable `Kind::code()` for container kinds, so the machine surface is not keyed on a display string carrying a geometry; and a C ABI (`bridge/include/ade.h`) that is the library binding half of the clause, hand-written and checked by a real C compiler on every push. Output is pure ASCII, so Latin-1 Amiga names round-trip losslessly. Verified across 4288 images: 68,961 JSON Lines, all valid, plus 400 more from `identify` and 50 documents from `diff`/`consolidate`.
**A closed pipe is the ordinary end of a command, not a failure** — every line goes through one writer, and `cli/tests/pipes.rs` holds each command to it (BUG-008).
**The schema is versioned, and the version is enforced (D-015).** Every document carries `schema` as its first field — including each line of a JSON Lines stream, which is the case versioning is for. Major means a field was renamed, removed, retyped or **redefined under an unchanged name**; minor means one was added. What makes that more than a promise is `src/api/tests/schema.rs`, which inventories every field path ADE can emit: any change to the output fails it, and the fix is to edit the inventory *and* move the constant in one commit, where a reviewer sees both. Same shape as the layering check and the byte-order tripwire — the policy is not that it cannot change, but that it cannot change quietly.
**Status note:** every acceptance clause is now met. What is left is upkeep rather than construction: the inventory has to stay honest, and D-015 records what it would mean for it to stop working.
**Notes:** Friendly tools aren't scriptable; the scriptable one (amitools) is slow. Depends on F-002.

### F-016 Format-conversion matrix
**Priority:** Should
**Effort:** M · **Phase:** 3–4
**Acceptance:**
- Convert any supported input container to any supported output, refusing or warning honestly where lossy or impossible (e.g. no IPF write, DMS `errdms`).
**Status:** Delivered 2026-08-25 — `ade convert` and `ade formats`
**What exists:** every pair of formats has an explicit answer carrying its own reason. The conversions that run are the decompression direction (ADZ→ADF, HDZ→HDF), whose reader is proven byte-identical against `gzip`, copies between sector containers, and sector→raw-MFM via `--raw`.
**Lossy conversions are refused outright, not warned about.** A warning nobody reads is how the loss happens.
**Refused and not-implemented are kept apart**, because they invite opposite follow-up: IPF is a decision that does not expire (C-003), DMS is a gap with a cause (D-009).
**Notes:** Conversions are today scattered across single-purpose tools; lossiness rarely surfaced. Bounded by C-003, C-004.

### F-017 FUSE mount of an image
**Priority:** Could
**Effort:** M · **Phase:** 2+
**Acceptance:**
- Mount an ADF/HDF as a host filesystem (read at minimum) on Linux/macOS.
**Status:** **Cut 2026-08-29 (D-017)**
**Why:** its own justification did not survive being measured. The Linux kernel's AFFS driver handles `DOS\0`–`DOS\3` read **and write** and `DOS\4`/`DOS\5` read-only, which over a 400-image sample is **94% of the corpus read/write** and essentially all of the 77% that mount. For the ADF and HDF this clause names, a FUSE filesystem would reimplement a driver that has been in the kernel since 1993, worse and slower, with "no root required" as its only advantage.
**A filesystem interface also cannot say what ADE is for.** `read()` returns bytes; it cannot report that a file came from a volume reassembled out of flux with 3% of its sectors missing, or that the bitmap disagrees with the directory tree. Mounting is the one interface that must discard the reporting that is the point of the tool.
**What would be worth building is a different feature** and is on the candidate list: a mount of the containers the kernel *cannot* read — ADZ, extended ADF, SCP, RDB partitions, reconstructions. See D-017 for the measurement and the reversal condition.

### F-018 HD / RDB multi-partition browsing & editing
**Priority:** Should
**Effort:** M · **Phase:** 2, 5
**Acceptance:**
- Parse an RDB partition table in an HDF and browse/edit each volume through the GUI.
**Status:** Browsing delivered 2026-08-28; editing is deferred by D-004
**Notes:** rdbtool is CLI-only; friendly multi-partition HDF editing is essentially absent. Bounded by C-002.
**Partitions are a level of the tree, not a menu.** A device holds no volume of its own — every volume is inside a partition — so the window shows each partition under the image, with its files under it. A picker would have made the disk look like one volume with a switch beside it, which is what a partition-blind reader assumes and is exactly the misunderstanding to avoid. A partition that holds no AmigaDOS volume says so on its row: `PFS\0` and `SFS\0` partitions are real partitions ADE cannot read, and an empty listing would read as an empty disk.
**The ABI grew a partition selector rather than a second family of calls.** `ade_dir_open`, `ade_walk_open` and `ade_file_read` each take one, with `ADE_WHOLE_IMAGE` for an image that holds its own volume. A device is not a special case of an image; it is what an image is when it has an RDB.
**A partition is not an offset.** It carries its own block size and reserved-block count, and the rootblock is computed from both (C-007), so the engine resolves it and the front end passes an index. A caller adding `first_block` to an assumed layout would miss the rootblock of any partition reserving other than two blocks.
**Editing remains out**, per D-004: read paths ship before their write counterparts, and nothing writes to an image yet.

The parser reads `RDSK`, the `PART` chain and a minimal `FSHD`/`LSEG`, and every partition mounts through a bounds-checked window. `ade info` prints the table, and `ls`/`extract`/`check` take `--partition=` by drive name or index. What remains is the browse/edit surface in the GUI, which belongs to Phase 5, and editing itself, which D-004 defers to Phase 4.

### F-019 Create a blank formatted disk
**Priority:** Should
**Effort:** S · **Phase:** 5
**Acceptance:**
- Produce a new, empty, mountable AmigaDOS volume — OFS or FFS, DD or HD — that ADE, an independent generator, and ADFlib all agree is well-formed.
- Never overwrite an existing file.
**Status:** Delivered 2026-08-29 — `ade create`
**Why it exists:** asked for while testing drag-out in the GUI, and the gap was real: ADE read six container formats and could create none of them. There was no way to obtain a disk to *put* anything on.
**It does not breach D-004, and the distinction matters.** D-004's rule is that "every write path ships only after its read path is proven on fixtures", with write arriving "from Phase 4/5". The OFS/FFS read path is proven — 4,652 corpus images, 99.36% agreement with ADFlib over 3,900 extracted files — and this is Phase 5. It is also the safest write there is: it produces a **new** file and touches nothing that exists, which is the irreversible damage D-004 is actually about. Adding a file to a disk somebody already owns is a different feature with a different risk, and is not this.
**Written from SPEC, deliberately not from the fixture generator.** `ade-fixtures` already builds volumes and reusing it would have been quicker — and would have destroyed what makes it useful, since D-010 keeps it dependent on nothing so a misreading in a layer crate cannot cancel out against it. Instead there are now **three independent statements** of what a blank disk is, and the tests require all three to agree: ADE reads back what it wrote, the fixture generator's equivalent matches structurally, and ADFlib mounts it.
**One defect the health check caught immediately:** the first disk produced reported three `datestamp-day-zero` findings *against itself*, because the stamp defaulted to zero and SPEC records day zero as what Amiga software treats as unset. The library still takes an explicit stamp so tests stay deterministic; the command uses the clock, because "created" means when the disk was made.
**Not bootable, deliberately.** AmigaDOS's own `format` leaves the boot code zeroed unless `install` is run, and ADE will not write boot code it would then refuse to interpret (AV-002).

### F-020 Content signature scanner
**Priority:** Should
**Effort:** S · **Phase:** 5
**Acceptance:**
- Find known file formats anywhere in an image by their magic bytes, reporting the offset and block of each, whether or not a directory entry points at them.
**Status:** Delivered 2026-08-29 — `ade scan`
**Why it exists:** mapped across from the Atari Disk Engine, which has a 62-signature scanner ADE had no equivalent of. What a directory entry calls a file and what the bytes are often disagree on a thirty-year-old disk, and the interesting content is frequently in space nothing points at any more.
**The table is measured, not recalled.** Every signature was scanned across all 4,652 corpus images — 24 seconds — and the counts are in SPEC §Content signatures. A magic that never appears in 4.2 GB of real Amiga disks is recorded as untested rather than quietly trusted.
**Three rules the corpus taught**, each correcting an answer that was confidently wrong: magics are **anchored to block starts** unless the format puts them inside a file (a ProTracker `M.K.` is 1,080 bytes in); the **most specific match at an offset wins**; and a pattern **repeating across consecutive blocks is filler, not files** — that last one turned 91 reported "DMS archives" into 3 real ones plus a damaged disk.
**It found damage the catalogue had missed.** `DMS!!ERR` and `DMS!1.52` are what xDMS writes over a track it could not decompress, so an ADF carrying either came from a DMS that failed partway. **Nine corpus images carry it.** One is already TOSEC-tagged `[b errdms]` — independent confirmation from the people who catalogued the collection — and the other eight are not.
**Notes:** the Atari engine scans for 62 signatures across 8 categories; ADE's table is 25 and Amiga-flavoured. Breadth is not the goal — every entry here has a documented source and a corpus count.

### F-021 Content search
**Priority:** Should
**Effort:** S · **Phase:** 5
**Acceptance:**
- Search a whole image for a byte sequence given as text or hex, reporting every occurrence with its offset, its block, and what part of the disk it landed in.
**Status:** Delivered 2026-08-29 — `ade find`
**Why it exists:** the companion to F-020. `scan` answers "what is on this disk that I did not know about"; `find` answers "is *this* on this disk" — a string, a filename, an opcode, a copyright notice — and neither question is answerable from a directory listing, because the interesting bytes are usually not in a file.
**The hex-or-text guess is made, and made visible.** A pattern that is entirely hex digits and separators, with the digits pairing into bytes, is read as bytes; anything else is text. The rule catches a few English words — `dead`, `face`, `added` — and reads them as hex. That is the deliberate direction, because the opposite mistake is silent: someone searching for `60 1A`, getting the ASCII of "60 1A" and finding nothing concludes the disk is clean. This mistake announces itself (`hex: true` in the output) and `--text` reverses it.
**Every hit says which region it is in, and that came from a measurement.** `Copylock` appears on **103 of the 4,652 corpus images**, and the region turns one string into four different findings: on 86 it is in the **bootblock**, where protection and the trackloader it starts live — the part no directory entry points at; on 10 it is in the **rootblock**, because those disks are *named* `Copylock(tm) Amiga`; on 11 it is in space nothing reaches; on 5 it is inside a file. The first implementation reported every one of those as "unallocated", which is wrong about the most deliberately written block on the disk and wrong about a volume name. **The bootblock is named even when the volume does not mount** — C-008 again, and a protected disk is frequently the one that fails to mount.
**The first version of this claim was wrong, and the corpus said so.** Measured over the alphabetically first 500 images it read "every hit is in block 0", which was true of that sample and false of the collection: the whole corpus has 51 hits outside the bootblock. A sample that agrees with the design is the easiest kind of measurement to stop early.
**Occurrences are not disks.** Across all 4,652 corpus images `DOS` matches **7,833,991** times, of which 207,159 are one image filled end to end with `DOS\0` — a count of occurrences describes that image and little else. Counted by image it appears on **4,450**: in the bootblock on 4,359, inside a file on 1,317, in space nothing points at on 840, in a directory header on 55, in a rootblock on 37, in a bitmap block on 14. Same trap as the batch histogram, in a new place.
**Overlapping occurrences are all reported**, because a caller counting a repeating sequence — the xDMS failure filler is one — would otherwise be given a number that is quietly low. The text output shows the first twenty and says how many it kept back; `--format=json` carries every one.
**Notes:** nothing found exits 1, as `grep` does — the search worked, and a script wants to branch on the result rather than on an error. A malformed pattern exits 2, because "searched, found nothing" and "never searched" must not look alike.

## Candidate features (uncommitted)

- Read support for modern journalling filesystems (SFS, PFS).
- FloppyBridge-compatible output for direct WinUAE/Amiberry use.
- Cataloguing enrichment via Demozoo / ScreenScraper (mirroring ManifeST).
- Track-level visualisation (MFM sync-word map, weak-bit heatmap).
- **FUSE mount of the containers the kernel cannot read** — ADZ, extended ADF, SCP, RDB partitions and reconstructions, letting ordinary tools reach content nothing else can. Replaces the cut F-017, whose ADF/HDF scope the Linux AFFS driver already covers (D-017). Would need an answer for what reading a reconstruction's missing sectors returns.
