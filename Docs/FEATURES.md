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
**Notes:** ADF Opus's never-realised cross-platform port is the gap. Qt6 over the C-ABI bridge, per D-001. Both acceptance clauses met: an image drops in, a file drags out, and search covers every open image at once. 51 headless tests run without an X server.
**The window opens at the size its contents need** (2026-08-30). It used to open at a hardcoded 1100x700 with an even split, giving the hex pane about 550 pixels for a dump line that measures 78 monospaced characters — so the characters column was cut off on the first disk anybody opened, and the remedy was to resize the window by hand every time. Both halves are now measured the way the tree's fixed columns already were: from the text they must hold, in the font they will hold it in. **An even split sounds fair and is not** — the tree's content has a natural width and the hex pane's does not, so half the window is more than the tree can use and less than the dump needs; the tree keeps what its columns need and every extra pixel goes to the pane showing the disk. The result is clamped to the screen, because a measurement is not permission to be bigger than the display, and when there is not room for both the **tree** gives way, down to a floor still wide enough to read a name in: a long filename elides and is still a filename, where a clipped dump line is simply missing.
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
**What exists:** container identification with its evidence; bootblock and rootblock condition; per-entry and per-data-block checksum failures; a bitmap cross-check against the blocks the tree actually reaches, in both directions; cross-linked block detection; file shortfalls and OFS structural faults; and, on a volume with no entries, free blocks that hold data — a trackloaded game behind an empty root (`free-blocks-hold-data`), or what deleting left (`free-blocks-hold-deleted-files`), IMP-019. Findings carry a stable code and a severity, and the report is available as text or JSON.
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

### F-029 Disk surface view
**Priority:** Should
**Effort:** M · **Phase:** 5
**Acceptance:**
- Show what came off each track of the medium, for containers that recorded it.
**Status:** Delivered 2026-09-01 — `ade surface`, and the GUI's **Disk > Disk surface**
**Why it exists:** mapped across from the Atari Disk Engine's `DiskSurfaceWidget`. F-007 already reported `sectors_placed` as a single number; this is the same information laid out as the medium, which is where protection and damage become legible.
**Two rows of eighty, not one run of a hundred and sixty.** The same cylinder on side 0 and side 1 are different tracks and fail independently, and drawing them as one run hides the pattern that matters: `Realm of the Trolls` has one whole side readable and the other entirely blank, which is a sentence in this layout and noise in the other.
**Four states, and the fourth is the one people forget.** Whole, partial, read-but-nothing-decoded, and **not in the container at all**. The last is missing information where the third is information, and collapsing them would make "nothing was recovered here" and "nobody looked here" the same picture. Every track has a state, always 160 of them, including those the container never mentioned.
**Most containers cannot know, and the menu item is greyed rather than lying.** A plain ADF is already sectors — every one present by construction, nothing recorded about how it was read — so drawing 160 whole tracks would claim a measurement nobody made. Only an extended ADF or a flux capture knows. `ade surface` exits 4 and says which containers do.
**Measured across the 11 corpus extended ADFs, and it reads like a diagnosis.** `Champ, The` shows a single unreadable track at cylinder 0 head 1 — corroborating SPEC's existing note that it is "protected by making exactly the track that matters unreadable". `Realm of the Trolls` is one perfect side and one entirely dead one. `Deep Space` is partial for fifty cylinders and then absent, which is a dump that degraded partway. `Demolition` is 160 whole tracks, a plain disk that happens to be stored as raw ones.
**Notes:** the per-track record is kept from the assembly that already happens at open rather than recomputed — what came off the medium lives in the raw tracks and is gone once they are sectors, so re-reading the file would be learning something already known. Four kilobytes per open image.

### F-028 What a disk says it needs
**Priority:** Should
**Effort:** M · **Phase:** 5
**Acceptance:**
- Report what a disk requires to run, with the evidence for each claim, and name what cannot be known.
**Status:** Delivered 2026-09-01 — `ade specs`, and the GUI's **Disk > Disk information**
**Why it exists:** asked for as "can we tell from a disk image what hardware it needs". Partly, and the interesting half of the answer is which parts cannot be told.
**Facts with evidence, never a verdict.** Every line carries what it is and why: `Needs at least Kickstart 2.0 — because it opens iffparse.library, which Release 2 introduced`. A claim without its evidence is asking to be believed; one that names the library is asking to be checked. This is D-014's posture over virus names applied to a second question, and for the same reason — a tool that says "needs 1 MB" from a disk that never said so is worse than one that says nothing.
**Always a lower bound.** A disk opening `asl.library` cannot run on 1.3; it may need far more than 2.0, and nothing on it would say. Every claim is phrased "at least".
**The blind spots are printed, not omitted.** Memory, processor, chipset and video standard are not on an Amiga disk and cannot be had without disassembly, which AV-002 makes structurally impossible here. A report that simply stops reads as "there is nothing more to know" — this one names where to go instead. The `(AGA)` in a TOSEC name is the **catalogue's** claim, and is labelled as such rather than borrowed.
**312 of 4,651 corpus images can be dated** from their own bytes, on six libraries Release 2 introduced. Two further leads — `locale.library` and `datatypes.library` — appear on 14 and 6 of 400 sampled images but have no sourced introduction version, so they are recorded in SPEC and **not** used. Same rule as F-020's signature table.
**A bootblock that is not AmigaDOS's is not called self-booting.** It is either a custom loader or a damaged AmigaDOS one, and nothing short of running the code tells them apart — the first draft said "self-booting" and was corrected by `Abandoned Places_Disk2`, which begins `\x00OS\x00` and is plainly the second. Both readings are reported.
**Notes:** a library name needs a word boundary. Without one, a scan of 400 images reported `udos.library`, `ugraphics.library` and `uintuition.library`, none of which exists.

### F-027 Block map visualisation
**Priority:** Should
**Effort:** M · **Phase:** 5
**Acceptance:**
- See what occupies a disk as a picture, and go from a cell to the bytes.
**Status:** Delivered 2026-09-01 — the GUI's **Map** tab
**Why it exists:** mapped across from the Atari Disk Engine's `FatVisualizerWidget`. The hex pane answers "what is at this offset" one screen at a time, and the health report gives counts; a grid of blocks answers "where did the space go" at a glance. F-022 already built the data and proved it tiles exactly, so this is a widget over a map that exists.
**Files are coloured here and are not in the hex pane**, which is the one deliberate divergence. Under text being read, tinting the four fifths of a disk that hold file data would colour everything and distinguish nothing; on a map every cell *is* a block, and leaving files blank would leave the map empty.
**The colours were solved, not chosen.** Files and empty space carry most of the map between them, and at the first values `unclaimed` sat at **1.16 contrast** against the page — invisible, which made "empty" and "off the end of the disk" the same picture. The chosen mix holds in both themes: on dark, file 3.93 and unclaimed 1.39 against the page with 2.82 between them; on light, 2.27 and 1.41 with 1.61 between. A test asserts the ratios rather than the colours, so they can be retuned.
**A cell that covers many blocks shows the most structural one.** Above about 30,000 blocks a cell stands for several, and showing the commonest would erase exactly what is worth finding — a rootblock is one block in seventeen hundred. The tooltip then says `blocks 640–703` rather than naming one, because a cell that reports one of sixty-four would be lying about its own resolution.
**Selecting a file picks out its blocks.** The map belongs to the disk, so choosing a file does not clear it. This is the thing no listing can show: a file's blocks sit wherever the bitmap had room, and "fragmented" stops being an abstraction the moment it is drawn.
**Notes:** clicking a cell opens the whole-disk hex view scrolled to that block, with the byte highlighted — the map answers *where*, and the question that follows is always *what*. The legend's swatches are outlined, because it sits on the window background while its colours are mixed against the map's page, and `unclaimed` is deliberately close to the latter.

### F-026 Making a disk from the window
**Priority:** Should
**Effort:** S · **Phase:** 5
**Acceptance:**
- Make any disk `ade create` can make, from the GUI, and have it open.
**Status:** Delivered 2026-08-31 — File > New disk...
**Why it exists:** creating a disk was command-line only, so the GUI could browse and extract but never produce. F-019's own note said dragging a file to the desktop needed a disk to drag *from*; making one needed the terminal.
**The list of filesystems is the engine's.** `ade_create_type_count`/`_name`/`_label` enumerate what ADE will write, so the window offers six because the engine writes six — not because a list in Qt says so. Two front ends deciding separately which disks exist is two chances to disagree, and a test pins the bridge's strings to the engine's.
**It moved policy out of the CLI on the way.** The type table, the shape-to-geometry mapping and the AmigaDOS clock all lived in `cli/src/main.rs`; the GUI needed the same three, and duplicating them would have been IMP-007 again. They are now `ade_core::create`, and both front ends ask.
**`AdeResult` gained `ADE_ALREADY_EXISTS`.** "Already exists" is something a person can fix by choosing another name; "could not write" is not, and reporting them alike would leave somebody re-trying the same thing. It matters here because a save dialog has *already* asked about overwriting and ADE declines anyway, which needs explaining rather than reporting as a failure.
**Notes:** the new disk opens straight away, because a disk you cannot see is not obviously a disk. The hard-disk size is capped at 49 MB in the dialog, which is where a bitmap extension chain would be needed (F-025).

### F-025 Every disk type ADE can verify
**Priority:** Should
**Effort:** M · **Phase:** 5
**Acceptance:**
- Create any AmigaDOS filesystem variant ADE can check, at any floppy size, and unpartitioned hard disks.
**Status:** Delivered 2026-08-31 — `ade create --type=… [--hd|--dd525|--size=N]`
**Why it exists:** `create` wrote two of eight filesystems and two of six geometries. The set is closed and small, so "which ones" is answerable rather than open-ended.
**Six of the eight, and the two absences are decisions.** `DOS\0`–`DOS\5` are written: OFS and FFS, each plain, international, and with a dircache. `DOS\6`/`DOS\7` (LNFS) are refused **by name, with the reason** — D-013 defers them on verifiability, and a caller who asked for something real deserves better than "unknown type". Beyond AmigaDOS there are some forty other 4-byte tags in SPEC's registry; none is ADE's to write and none appears in the corpus.
**The dircache question was settled by the oracle, not by assumption.** SPEC records the rootblock's dircache pointer as "first dircache block, else 0", which left open whether a blank `DOS\4`/`DOS\5` needs a cache block. It does not: ADFlib mounts one written with a zero pointer. All twelve type-and-floppy-size combinations are checked against it.
**Four geometries.** DD (901,120), HD (1,802,240), 5.25" DD (450,560 — the A1020's 440 KB), and hard disks by `--size=N` megabytes. Hard disks needed **multi-block bitmaps**: one bitmap block maps 4,064 blocks, so 8 MB needs five, exactly as SPEC's arithmetic says. Past 25 pointers a `bm_ext` chain is required, which ADE does not write — refused rather than half-written, because a volume whose bitmap is partly described reports free blocks that are not. The boundary is exact: 49 MB fits, 50 does not.
**What is deliberately absent, and what it found.** No cylinder count, because a corpus measurement said an 81-, 82- or 83-cylinder image is **not a larger volume** — five of six such images keep their rootblock at 880, and ADE looks at the midpoint of the whole file and finds a file header. That is **BUG-009**, logged rather than fixed inline: it is a read-path defect that this work uncovered, not part of it.
**5.25" has no oracle.** ADFlib refuses the size before reaching any filesystem, so it can neither confirm nor deny those bytes. Written on the formula being verified at 1760, 3520 and 16,384 blocks elsewhere, and the fixture generator agreeing at 880 — two of the usual three checks, and said so rather than counted as a pass.

### F-024 Extract everything to a folder
**Priority:** Should
**Effort:** S · **Phase:** 5
**Acceptance:**
- Write every file on an image into a folder, with its drawers, in one command and from the window.
**Status:** Delivered 2026-08-31 — `ade extract --all`, `ade_unpack`, and the GUI's File menu
**Why it exists:** mapped across from the Atari Disk Engine's "Extract All Files to Folder". ADE's `extract` took one path at a time and the GUI extracted one dragged file; whole-image extraction existed nowhere, which is the ordinary thing to want from a disk.
**The names are the hard half, and every rule came from a measurement.** Across all 4,652 corpus images and 83,487 distinct filenames: non-ASCII names are real and meaningful (`Effekte für AE 2 Deutsch.info`, `CD³²_Prefs`) so a name is Latin-1 decoded to **UTF-8**; three names carry a **NUL**, which cannot go in a POSIX filename at all; **zero** contain `/`; one is exactly `.` or `..`, which would resolve to a different directory rather than fail; three contain a literal `%`. Those are escaped as `%XX` of the original byte, `%` included so the escape stays reversible — for a file taken off a disk this is the only record of what its name was.
**And what is deliberately not escaped.** 62 names carry a character Windows forbids (`>>> BY AEON <<<`, ` * DRAGO & AMADEUS * `) and 328 end in a dot or a space or are nothing but spaces. All are legal on POSIX, and escaping them would mangle 390 real names to buy portability to a platform ADE has never been built on. A Windows build will need to escape more; that is a decision to take with a Windows build in front of you, and the numbers above are what it costs.
**Nothing is ever overwritten.** Two files in one drawer cannot share a name — AmigaDOS's hash table prevents it — so a collision means a case-insensitive host or two names that escaped alike. Exactly one corpus image collides that way: `1869 (AGA)_Disk1` has `Startup-sequence.bak` and `startup-sequence.bak` in the same drawer. The second is skipped and named, and the exit code says the recovery was partial: a run that reports success while quietly missing a file is how somebody comes to believe they have the whole disk.
**Nothing stops at the first bad file**, the same reasoning as `ade batch`. Over 400 corpus images: 14,995 files written in one second, 2 skipped — both genuine read errors on one damaged disk, each named.
**Notes:** a hard link is resolved before it is read. Reading one directly gives an empty file of the right name, silently (BUG-005). Verified against the single-file path: all 49 files of `4-Get-It.adf` are byte-identical whether taken one at a time or all at once.

### F-023 Content search in the window
**Priority:** Should
**Effort:** S · **Phase:** 5
**Acceptance:**
- Search the bytes of every open image from the GUI, not only their filenames, and go to a hit.
**Status:** Delivered 2026-08-30 — the search box's **Contents** mode
**Why it exists:** F-021 was CLI-only. The window could search *names* across every open image and could not search *contents* at all, which is the half of the question that reaches the bootblock, the rootblock and the space no directory entry points at — the parts a filename search can never see.
**One box, two questions.** A mode beside the field rather than a second field: two boxes would leave one of them stale and wrong-looking whenever the other was used. The results columns follow the mode, because an offset and a filename are not the same answer and should not share a heading.
**A refused pattern is not a search that found nothing.** `ade_find_open` returns a handle carrying the reason rather than null — the only opening call in the ABI that does, and deliberately so: reporting "0 matches" for a pattern that was never searched would have someone conclude their disk is clean. Same distinction the command line draws with exit 2 against exit 1.
**Clicking a hit goes to it**, in the whole-disk view (F-022), scrolled with a few lines of lead and with the byte highlighted — a match on the very first visible line reads as the top of the view rather than as a result. This is where F-022's colouring pays off: the byte arrives in a region the eye can already name, and the tree marks the owning file.
**A hit is attributed by block, so a hit past a file's end is in its slack.** Searching a real disk for `SUPERSHEET` reports a hit inside `FONTSET`, a 20-byte file: the string is in the unused remainder of the 512-byte block that file's data occupies. That is the truth at block granularity and worth knowing before reading too much into an attribution.
**Notes:** searching an open image reuses the mounted handle (`Search::of_image`) rather than mounting a second copy from bytes, which is a whole extra copy of the disk and a second walk of its directory tree. A test pins the two entry points to the same answer.

### F-022 Whole-disk hex with regions
**Priority:** Should
**Effort:** M · **Phase:** 5
**Acceptance:**
- View the hex of an entire image, not only of a file, with each part of the disk distinguished: bootblock, rootblock, bitmap, directory, file and unclaimed space.
**Status:** Delivered 2026-08-30 — `ade layout`, `ade_layout_open`, and the GUI's disk row
**Why it exists:** mapped across from the Atari Disk Engine, which has a full-disk hex view with colour-coded sections and a legend. A file view can never reach the bytes that matter most on an old disk — the bootblock where protection lives, the rootblock, and the space no directory entry points at, which is where a damaged disk keeps whatever is left.
**The map is a tiling, not an annotation.** Spans cover every byte with no gaps and no overlaps, `unclaimed` where nothing else applies, because a front end colouring from a partial map paints holes that read as ordinary data — the map being wrong looks exactly like the disk being different. Verified over the whole corpus: **4,651 of 4,652 images tile exactly**, in 6.6 seconds. The one exception is a 166-track extended ADF whose assembled size matches no geometry, which `ade ls` has always refused in the same way.
**Runs, not blocks.** An 880 KB floppy is 1,760 blocks and about ninety spans; the largest in the corpus has about seven hundred. A row per block would make the map bigger than the thing it describes.
**Files are deliberately not coloured.** They are most of a disk, and colouring everything is the same as colouring nothing — the eye has to be able to find four structural blocks among sixteen hundred that hold data. Unclaimed space gets the faintest wash there is, because "not part of any file" is worth seeing and is also the second most common answer.
**The bootblock is mapped even when the volume does not mount**, which is a quarter of real images: C-008 again, and an unmountable disk is usually a protected one, so it is exactly the case where knowing where the bootblock ends is worth something.
**Only whole images, not partitions.** A device's map would place several volumes, each with its own block size, at absolute offsets — and **no image in the corpus carries an RDB**, so there is nothing to check such a map against. `ade_layout_open` returns null for a partition index rather than guessing.
**A Help menu with an About box** (2026-08-30). The version in it comes from `ade_version()` rather than from a string written in Qt: an About box is the one place people go to find out exactly what they are running, and a version the front end supplies is one that can disagree with the library it was built against. The licence line repeats NOTICE's claim that ADE contains no third-party code, which is a claim that has to change if D-009 ever brings xDMS in — a test pins both. **The menu holds About and nothing else**: the manual will join it, and until it does nothing stands in for it, because a menu item that is greyed out or opens an apology is a promise the window has already broken.
**Scrolling the disk says which file is on screen** (2026-08-30). The row is **marked, not selected** — selection is what chose the whole-disk view, so following the scroll with it would replace the very view being scrolled, and the feature would undo itself on the first wheel click. The mark is a pale wash **and bold**: the wash alone is a paler version of the selection colour and the selected row is right there in the same tree, so rendered side by side the two read alike. The status bar always names the region and the owning path, because a file inside a drawer nobody has opened has no row to mark and "file" on its own says nothing. The tree is deliberately **not** expanded to reveal it: a tree that reorganises itself under the pointer while you scroll is worse than one that does not.
**Qt does not reliably say when the view moved**, which is why the position is polled as well as signalled. Measured: a click in the scrollbar **trough** scrolled the pane from line 0 to line 37 — the painted text proves it moved — while emitting `valueChanged` zero times and calling `scrollContentsBy` zero times. The wheel and a drag of the handle both notify, so the gap reads as the feature working right up until somebody pages down. A 120 ms timer runs only while a whole disk is shown, and costs one integer comparison per tick because the work is skipped unless the top line actually changed.
**The top line comes from the scrollbar, not from `cursorForPosition`.** That was the first implementation and reads where the viewport has been *painted*, which lags the scroll. With word wrap off, QPlainTextEdit's vertical scrollbar counts blocks and one block is one dump line, so its value is the answer rather than an approximation of it.
**Matching is by block, scoped to the disk being viewed.** The map carries the owning entry's block as well as its path — the path names the owner for a person, the block identifies it for a program, and comparing Latin-1 path strings can go wrong in ways a block cannot. The search is confined to the image and partition on screen, because a block number is unique within a volume and nowhere else: unscoped, scrolling one open disk would quietly mark a file in another.
**Notes:** the whole-disk view was capped at 4 MB, because a text widget laid the whole dump out: clicking the disk row cost about 400 ms. Since IMP-017 (2026-10-25) the hex pane paints only what is on screen and reads the disk as it scrolls, so a hard disk is shown whole (to 4 GiB) and the click costs about 4 ms. The text tab keeps the 4 MB cap.

### F-030 Recovery and carving
**Priority:** Should
**Effort:** M · **Phase:** 5
**Acceptance:**
- Recover files whose directory entries are gone, and grade each recovery by how far the disk itself supports it.
**Status:** Delivered 2026-09-01 — `ade carve`, `ade carve --all DIR`, `ade_carve_*`, and the GUI's Disk menu
**Why it exists:** mapped across from the Atari Disk Engine, and the last item on that map that was blocked on something other than effort. A disk whose directory has been damaged, or whose files were deleted, still holds the blocks; nothing in ADE could reach them. F-022's `unclaimed` region already found the candidates.
**The blocker was verifiability, and an OFS file answers it by proving itself.** A carver produces files no directory claims, so there is nothing to compare them against — the same trap as LNFS, and what D-002 gave up ADFlib's knowledge to avoid. But every OFS data block carries a header: its type (`T_DATA`), the block of the file header that owns it, its sequence number, and its own checksum. The orphaned header names a chain of blocks; each of those blocks independently names the header back. **Three agreements per block, none of them ADE's opinion** — the same self-evidencing property that makes the MFM decode checkable. The feature exists because of that and would not otherwise.
**Every result is graded, and the grading is the deliverable.** `self-evident` — every data block agreed. `partial` — some agreed, some did not, so the file is partly overwritten. `header only` — the header is sound and **nothing confirms a byte of the contents**. An FFS data block is raw payload with no header at all, so an FFS carve can never be better than header-only, and that is stated rather than hidden. A tool that hands over all three under the same word is the untrustworthy thing this was blocked on being.
**What is written out is narrower than what is listed.** `--all` writes self-evident files whole, writes partial ones under a **`.partial`** suffix, and **writes header-only carves not at all**. A file on disk with the right name and unconfirmed bytes is worse than no file, because somebody will believe it. A run that produced any partial exits non-zero, so a script can tell "everything came back whole" from "some of it came back with holes".
**It carves disks that do not mount**, because those are the ones worth carving. `carve` never asks for a volume: it reads the unclaimed spans, parses headers out of the bytes, and walks the block chain itself. The first version required a mounted volume and found **nothing at all** on `'Allo 'Allo! Cartoon Fun!_Disk1.adf` — an unmountable disk holding 29 orphaned headers, one of which (`L2_BLKS_ICE`, 15,314 bytes) carves out beginning with the `Ice!` magic. The name came from the header and the contents from the data blocks, two different places on the disk, and they agree.
**Measured over all 4,652 corpus images in 8.5 seconds:** **402 images hold orphaned headers**, 4,627 of them — 4,240 files and 387 directories. Of the file headers, **2,626 are fully self-evidencing**, 695 partial and 919 confirm nothing; **16.2 MB of content is recoverable with every byte accounted for**. Directories are header-only by construction, having no data blocks to agree with them.
**Names are prefixed with the block number.** Two lost files can share a name — a deleted file and its replacement usually do — and the header's block is the only thing that makes each answer unique. It is also the number to go back to in the hex view.
**The window shows the grading before it shows the files.** The dialog lists every orphaned header with its evidence, what is recoverable against what the header claims, and greys the header-only rows rather than hiding them — a row saying "a file called this, of this size, was here" is real information and is also the one row nobody should treat as content. The three words are spelled out in the dialog every time. Recovering with nothing selected takes everything; with a selection, those.
**The menu item is enabled for any open disk**, unlike the surface view beside it. "Nothing is lost on this disk" is a result worth asking for, where a plain ADF's missing track information is an inability — and carving is not conditioned on the disk mounting, since the disks worth carving largely do not.
**What cannot be tested yet, and why.** Deletion is simulated — a fixture's root hash table cleared and re-checksummed — because ADE cannot delete a file. That is honest as far as it goes: the generator is independent of the engine (D-010), and clearing the hash slots is what AmigaDOS leaves behind. But the test the carver really wants is a round trip — write a known file, delete it *through ADE*, carve it back, require the bytes to be identical — and that needs the write suite. Recorded against it in [ROADMAP.md](ROADMAP.md).
**Notes:** the chain walk keeps its own visited set and caps at 4,096 blocks, per D-006: an orphaned header's extension chain is by definition damaged, and a loop in it must not hang the carver. Tests build the case directly — a fixture volume whose rootblock hash table is cleared and re-checksummed, which is what deletion looks like from the outside.

### F-031 Writing into an existing disk
**Priority:** Should
**Effort:** M · **Phase:** 5
**Acceptance:**
- Make a directory and add a file on a disk somebody already owns, under D-018's four terms: edits buffered in memory, a save that is explicit and never over the file the disk was opened from, a save that is atomic, and a result that passes `ade check` and mounts under ADFlib.
- Reachable from the CLI and from the window, where a file dropped onto a disk or a folder in the tree is copied onto it.
**Status:** Delivered 2026-10-11 — `ade mkdir`, `ade put`, `ade_edit_*` / `ade_image_save*`, and the GUI's drop target, Edit menu and Save / Save as. **Delete and rename added 2026-10-25** — `ade rm`, `ade rename`, `ade_edit_delete` / `ade_edit_rename`, and the window's Delete and Rename. **Moving, formatting in place and wiping added 2026-10-27** — `ade mv`, `ade format`, `ade wipe --slack|--free`, `ade_edit_move` / `ade_edit_format` / `ade_edit_wipe`, rows dragged onto a folder in the window, and Disk ▸ Format, Wipe file slack and Wipe free space. Hard links are deferred on verifiability; editing the bootblock stays declined (AV-002).
**Why it exists:** asked for from the window — dragging a file *out* of a disk worked, dragging one *in* said "not a disk image". The window was right: it had been handed a file and asked to open it as a disk, because nothing in ADE could put a file on one. D-018 had already settled the terms and named a directory as the first operation.
**The engine places, the layer above decides whether to keep it.** `ade_filesystem::edit` does the filesystem's bookkeeping — allocate from the rootblock outward, write the header, insert into the parent's hash chain kept in block order (SPEC §Collision chains), clear the bitmap bits, move the parent's and the volume's datestamps, and on `DOS\4`/`DOS\5` add a record to the parent's directory cache, growing the chain when a block fills. Every block is built in a plan first and copied in only when the whole plan exists, so a refusal half way — a full disk, a damaged directory — leaves the image as it was. `ade_core::edit::Session` then runs the health report on the result and **undoes any edit that adds a warning or error the disk did not already have**: that is ADE's bug, and is reported as one rather than saved.
**What it refuses, and why each is a refusal rather than a best effort.** A volume with no dostype (ADE cannot know how its names hash), LNFS (D-013), a bitmap flagged invalid or failing its checksum, a parent whose listing or cache is damaged, a disk whose bitmap marks live blocks free (the next allocation could land on data), a name AmigaDOS cannot hold or already holds, and anything that is not a plain ADF or unpartitioned hardfile — a compressed or flux container would have to be saved as something other than it was, and a device needs a partition chosen. **And one refusal that asks rather than forbids (IMP-019, 2026-10-28):** on a volume with no files whose free blocks hold data — 87 corpus disks, every one a game read by its own loader — adding a file or a folder and wiping free space wait for `--overwrite-free-data`, `ade_edit_allow_overwriting_free_data`, or a yes in the window, since reusing an old game disk as a blank is a fair thing to want.
**Measured across the whole corpus:** a directory and a 3,000-byte file inside it were added to every image ADE would edit — **3,023 of 4,652** — with **no new health finding on any of them**, and ADFlib extracted the new file byte-identically from **3,013**; the other ten are disks ADFlib cannot read in the first place. The rest were refused with a reason: 1,052 are not plain images, 288 are full, 255 have a bitmap flagged invalid, 15 hide live blocks behind free bits, 9 have a damaged directory, 10 have no usable dostype.
**Checked by two readers, and the tests were checked by breaking the code.** Every filesystem ADE writes, every awkward file size (0, 1, either side of an OFS and an FFS block, either side of the 72-pointer header that forces an extension block), crowded directories that force collision chains and multi-block caches, nested directories, accented names — each result must satisfy ADE's health report *and* extract byte-identically through ADFlib, including `unadf -c`, which lists from the dircache alone. Five deliberate breakages were each caught: wrong hash function, no dircache update, no bitmap update, pointer table forwards, OFS sequence numbers from zero. The first of those got through the first version of the tests, because ADE's listing walks every slot and ASCII names hash alike either way; the test now uses accented names, reads the slot itself, and asks ADFlib to find each file by name — and either check alone catches it.
**The window.** Dropping files onto a disk's row or a folder copies them there, folders recursively; dropping onto empty space opens them as disks, as it always did. A drag that began in the tree is not taken back, so letting go of a drag-out a pixel early copies nothing. A modified disk says *unsaved changes* on its row and the title carries an asterisk; closing asks first. **Save** on an opened disk asks, then writes over it, keeping the original as `<file>.orig` the first time and refusing if the file changed on disk since it was read; a disk made with New disk saves straight back to its own file; Save As always makes a new file (D-020, 2026-10-23 — D-018 first made every save a Save As). Undo and redo step through whole copies of the disk, which is what D-018 meant by undo being ordinary.
**Delete and rename (2026-10-25).** *Delete* takes a file, a soft link or an empty folder out of its parent's hash chain and cache and marks its blocks free — **without erasing them**, as AmigaDOS does (SPEC §Deletion), so a deleted file is recoverable until its blocks are reused. A folder that holds anything is deleted with everything in it only when asked (`ade rm --all`, a confirmation in the window), deepest first, as one undo step, and all-or-nothing. *Rename* moves an entry to the hash slot its new name falls in (C-006, so an accented name on an international volume lands where AmigaDOS looks), updates its cache record, clears the old name's tail from the header, and keeps the entry's own datestamp; a change of case alone is allowed. Both move the parent's and the volume's datestamps. **Refused:** the root, a non-empty folder without `--all`, and anything a hard link is part of — removing one end of a link chain means relinking the other, which ADE does not write yet.

**The carve round trip F-030 was waiting for.** A file put on a disk, deleted through ADE and carved back comes back byte-identical on every OFS variant, block by block confirmed; on FFS, whose data blocks carry no owner, the carver honestly offers only the header. Tested through the engine and from the command line (`ade put`, `ade rm`, `ade carve --all`).

**Moving, formatting and wiping (2026-10-27).** *Move* takes an entry, a folder with everything in it included, from one directory to another under the same name: out of the old hash chain and cache, into the new, its `parent` field changed, its own datestamp kept. *Format* is AmigaDOS's quick format, in place — a new bootblock, rootblock and bitmap, the old blocks left free for `carve`. *Wipe* zeroes file slack, and with `--free` every free block too; it is for a copy to give away, and is never part of anything else. All three are one undo step and saved only to a new file. **Checked against the corpus:** a file moved into a new folder on 3,000 disks, and ADFlib found it there by name on 2,971; both wipes run on all 3,600 editable disks with no file changed by ADE's reading or ADFlib's. **Refused:** a folder moved into itself or inside itself; a free-space wipe through a bitmap that cannot be trusted, or a tree that cannot be read in full. Measuring them found **BUG-017**, `create` storing a name outside ASCII as UTF-8, and **BUG-018**, entries whose parent field names another block, which every edit of an entry now refuses with the reason. **Hard links** are not written: ADFlib does not list them and no corpus disk has one, so there is nothing but ADE to check them against.

**A deletion keeps a block another file shares.** AmigaDOS would free every block of the deleted file; on a cross-linked disk that hands the other file's block to the next write. Found on the corpus — deleting `golf` on Championship Golf freed block 1687, which another file uses, and ADE's own health check refused the result as damage it had caused. A deleted file's blocks are now freed only if no other entry uses them, with a built test that fails without it.

**Measured across the whole corpus:** the first plain file in the root of every disk ADE would edit was renamed and then deleted — **3,062** of each — with no new `ade check` finding on any, and ADFlib found **2,985** of the renamed files by their new name with the bytes it had read under the old one. Two kinds of file are not used to judge ADFlib, and the test says why: damaged ones, on which `unadf -p` gives different bytes on every run (Beverly Hills Cop_DiskB: three runs, three digests), and names beginning with `-`, which unadf reads as an option. One rename was at first refused, on Typhoon, where renaming the stale copy of `tubes` exposed a cross-link the health report had been missing — **BUG-014**, fixed the same day, after which all 3,062 rename and delete.

**Notes:** a drop is one undo step however many files it copies (IMP-014, applied 2026-10-23). `ade put` and `ade mkdir` have no in-place form — `--output` is required and refused when it names the input. Finding the place to put things goes through `Volume::lookup`, which could not find an accented name until BUG-011 was fixed on 2026-10-22.

### F-032 Assembly view
**Priority:** Should
**Effort:** M · **Phase:** 5
**Acceptance:**
- Beside Hex, Map and Text, an Assembly view showing a program's 68000 code in Motorola syntax, and a disk's boot code.
- Code found the way the disk says it is — an executable's code hunks, a bootblock from byte 12 — never guessed, except when asked, and then headed as a guess.
- The decoder checked against an implementation nobody here wrote, on every opcode word and on real programs.
**Status:** Delivered 2026-10-22 — `ade disasm`, `ade_disasm_*`, and the GUI's Assembly tab. Scoped by **D-019**: a viewer, not the reverse-engineering suite.
**Why it exists:** asked for from the window — a hex dump of 68000 code tells almost nobody what a program does, and the moment of reading a disk is when a person wants to know. The Atari map had a disassembler as "needs a decision first"; D-019 is that decision, and splits the viewer from the suite.
**It is not "just converting hex".** A 68000 instruction is two to ten bytes long, so decoding from the wrong byte produces confident nonsense. An executable's hunk header says which hunks are code; those are decoded, data hunks shown as `dc.b` with their text, BSS as a size, symbols as labels, and every relocated address as the hunk and offset it points at (`hunk1+$10`) rather than the zero the file stores. Branch and PC-relative targets get labels. Anything else is decoded only on request — the window's *Decode anyway*, the CLI's `--raw` — and the listing opens by saying it is a guess.
**Checked against GNU objdump, twice over.** Every one of the 65,536 opcode words, followed by two different extension patterns, decodes to the same instruction with the same length — and across the corpus, **all 39,368 executables, 131,976 code hunks and 31.35 million instructions** agree with objdump at every address. The words skipped are listed with reasons in `objdump_differs`: objdump decodes coprocessor words, the 68020's full-format index words, `swbeg` and `subq.b` to an address register even when told the CPU is a 68000. Where ADE was stricter than the chip the oracle corrected it — a 68000 ignores an index's scale bits and reads a branch byte of `FF` as −1 — and the corpus found what the sweep could not: a `movem` with an empty mask, in 22 programs, which ADE printed as nothing.
**It says when code was not written for a 68000.** A full-format index word is reported as *68020 addressing mode* rather than as an unknown word. At least 17,043 corpus code hunks contain one — a lower bound, since the comparison stops a hunk at the first word objdump reads differently — though data mixed into a code hunk can look like one too, so it is evidence rather than proof. Weighing that evidence is F-028's business, and a separate change.
**Bounded and fuzzed (D-006).** The decoder has been run on every opcode word against every truncation; the hunk parser checks every count against the bytes remaining, caps the hunk table, and stops with a note at a block it does not know; a listing stops at 400,000 lines and says so. 300,000 fuzz cases over mutated executables, none failing.
**Every instruction says what it does (2026-10-23).** Asked for from the window: the listing said *what* each instruction was and not what it *did*. An explanation column, generated from the decoded instruction rather than the listing's text so the two cannot disagree, in assignment notation — `word at a1 = word at a0; a0 += 2, a1 += 2`, `d0.w -= 1; loop back to lab_002c until d0.w reaches -1`. **Amiga hardware is named** from the *Hardware Reference Manual*: every custom-chip register from `$dff000` (ECS and AGA ones marked), the CIA registers, the 68000's exception vectors and ExecBase at `$4` — so `ori.b #$88,($bfd100).l` reads as setting bits of CIA-B port B, the floppy drive control, and `btst #6,$bfe001` as testing the left mouse button, the meaning of the bit rather than the number. The status register's interrupt level is spelled out (`ori.w #$700,sr` turns every interrupt off), and `lea $dff000,a6` is named as the custom chips' *base* rather than after the register at offset 0. Addresses not in the tables stay numbers. An **Explain** checkbox shows or hides the column without decoding again; the CLI's listing carries it at column 72. The bytes column also gained two characters: a ten-byte instruction had run into its own mnemonic. **Checked over every opcode word.** An audit decodes all 65,536 first words, each with three different extension tails (131,125 instructions), and checks every explanation against its instruction: the operation, the condition a branch, loop or `Scc` tests, every register, each post-increment and pre-decrement step at the right size (MOVEM's step times its register count, `sp`'s byte step of two), the size a memory value is read at, immediates, hardware names and branch targets. It found wrong explanations that the screenshots had not shown: bit operations, `tas`, `nbcd`, `abcd`, `sbcd` and `Scc` described at the instruction's default size instead of a byte, MOVEM's address register moved by one size rather than one per register, `link`'s direction, `btst d0,#imm` naming no value, and `movea.w (sp)+` not saying it sign-extends. What a property cannot judge — whether the sentence is *right* — is pinned by a reference table, `src/object/tests/m68k_explanations.txt`, of one example of each of the 1,931 instruction forms, reviewed against the *M68000 Programmer's Reference Manual*; a change to any explanation fails the test until the table is regenerated (`ADE_BLESS=1`) and the difference read. That review corrected `st`/`sf` (a constant, not a condition), `dbt` (never loops), `andi #…,sr` clearing the supervisor bit as well as the interrupt level, and the grammar of memory values mid-sentence. **The hardware names have the same pin** (IMP-018, 2026-10-25): `src/object/tests/hardware_names.txt` holds every address ADE names, including the custom offsets it leaves unnamed; the names were checked against AROS's `hardware/custom.h` and `cia.h`, the meanings against the *Hardware Reference Manual*, and six meanings corrected.
**Notes:** nothing is executed — AV-002's defence is unchanged. Library calls are shown as offsets (`jsr -$228(a6)`), not named: naming them needs the base register tracked, and D-019 leaves that for its own entry. The ADE side of the comparison is `Instruction::gnu`, which renders in objdump's syntax including its quirks; the view uses Motorola syntax throughout.

### F-033 Decoded view
**Priority:** Could
**Effort:** S · **Phase:** 5
**Acceptance:**
- Beside Hex, Map, Text and Assembly, a Decoded view showing a file ADE recognises in plain English, and saying what it decodes for anything else.
- First format: Intuition's Preferences, `devs/system-configuration` — every field named, colours shown as colours, the pointer as a picture.
- The layout checked against a source ADE did not write, and every corpus file decoded.
**Status:** Delivered 2026-10-25 — `ade decode`, `ade_decoded_*`, and the GUI's Decoded tab. **Workbench icons and IFF ILBM pictures added 2026-10-26; bitmap fonts and IFF 8SVX sounds 2026-10-27.**
**Why it exists:** asked for from the window: most disks carry a `system-configuration`, and asked whether it says what hardware a disk needs, the answer was no — it is how somebody liked their Workbench. Logged as a candidate on 2026-10-24 as a reading aid, and committed to the next day. What a disk needs is F-028's question.
**One view for every decoded format, not a tab each.** The engine returns labelled fields in groups, colours as colours, and an optional picture; the window adds swatches and draws the picture and decides nothing about the format. An icon or a font is the same shape — fields and a picture — so the next format needs no new tab and no new ABI.
**Checked against an independent transcription and the whole corpus.** The offsets were laid out from `intuition/preferences.h` and checked against AROS's copy; SPEC §Intuition Preferences has the table. Every corpus disk holding the file was decoded: **1,109 of 4,653**, every one 232 bytes, every field named except one font height of 0 reported as unknown; two files of the wrong size are refused with their size. The corpus also corrected the design: the 30 bytes at offset 186 are one name on 1.x and three fields on 2.0, and real files carry 1.x text across the 2.0 boundary, so ADE reads them one way or the other by whether the padding is clean.
**Recognised by name and exact size**, because the format has no magic number. A copy elsewhere on the disk under the same name is decoded too.
**Bounded and fuzzed (D-006).** A fixed size, every read bounds-checked; 300,000 fuzz cases over mutated and truncated files, none failing.
**Icons and pictures (2026-10-26).** Chosen by measuring what the corpus holds: icons on 1,777 disks, ILBM pictures on 566, fonts on 454, 8SVX sounds on 300. Both fit the view as built — fields and a picture — and needed only one addition to it: the **shape of a pixel**, since a Workbench icon's pixels are twice as tall as wide and drawn square it looks flattened (`ade_decoded_picture_pixel_shape`). The window scales a picture to fit rather than always eightfold.
- *Icons:* kind, which Workbench saved it, the image, how it shows selection, position, stack, default tool, tool types, tool window, and a drawer's window; drawn with the selected image (or the complement) beside it, in the palette its revision says — taken from real Workbench disks' `system-configuration`, not from memory. The order of the parts is AROS's `icon.library` reader's; **12,817 of 12,826 corpus icons decode and 12,807 account for every byte of their file.** Measuring corrected a misreading early: with the image pointers four bytes out, the stricter selected-image rule looked worse than AROS's, which it is not.
- *Pictures:* size, planes, display mode, compression, masking, aspect, page size, other chunks and the colour map; indexed, extra-half-brite, hold-and-modify and 24-bit, and a colour map with no picture as a palette. Read as AROS's `ilbm.datatype` reads them, except that a hold-and-modify line starts from colour 0 as the hardware's does. **4,664 corpus pictures decode and 4,637 use their `BODY` to the byte**; the 161 refused are game formats wearing an ILBM header.
- *Bounded (D-006):* both decoders are fuzzed, 300,000 cases each, and both fuzz targets found an allocation a header could demand without paying for it — an icon image of depth 0, a picture whose `BODY` could never fill it — now refused before anything is allocated.
- *Not decoded:* NewIcons images (9 icons), OS 3.5 colour icons (none in the corpus), `ANIM`, and the older `$F34C` `.info` format, for which there is no independent source.

**Fonts and sounds (2026-10-27).** The next two by corpus reach: fonts on 454 disks, 8SVX sounds on 300.
- *Fonts:* a `.font` contents file decodes as the sizes it lists; each size's file as the font — height, width or proportional spacing, baseline, characters, style, kerning, the screen it was drawn for — with a three-line specimen set as `Text()` sets it, kerning before each glyph and spacing after, in the pixel shape the font asks for. **2,575 corpus size files, every one decoded**; and where a contents file and a size file both state a height, written separately, **2,552 of 2,561 agree**. 21 files whose last table runs a few bytes past the end of the file are read as far as they go, and say so.
- *Sounds:* name, author, length in samples and seconds, what plays once and what repeats, rate, octaves, volume, channel and compression, with the first octave's waveform, the repeating part in orange. **2,599 sounds, every one decoded, 2,525 with exactly the body their header describes.** Fibonacci-delta compression, on 6, is described and not decoded: AROS refuses it too, and there is nothing here to check a decoder against.
- *Bounded:* fuzzed together, 300,000 cases; every table, count and length comes from the file, and none allocates beyond what it holds or the bounded specimen.

**Notes:** the pointer's hot spot is reported as the stored offsets, not as a pixel of the image — which pixels the offsets count is not in the file. The CLI reports in text only, like `ade disasm`, so the JSON schema is unchanged. Kickstart 2.0 and later keep preferences as separate files under `ENVARC:Sys/`; those are not decoded yet.

### F-034 Repairing a disk
**Priority:** Should
**Effort:** M · **Phase:** 5
**Acceptance:**
- Put back what a volume's own structures say should be there, as one edit under D-018's terms: buffered, undoable as one step, saved only to a new file.
- The bitmap: every block the tree uses marked used, every bitmap block sealed, the valid flag set — refused, as AmigaDOS's validator refuses, when the tree cannot be read in full.
- Entries filed under the wrong hash slot (IMP-012) moved to the slot their name hashes to.
- Anything that needs repair and is not repaired is reported with the reason. Nothing that hides evidence of damage: no checksum is resealed over contents that failed it.
**Status:** Delivered 2026-10-26 — `ade repair [--free-orphans]`, `ade_edit_repair`, and the window's Disk ▸ Repair disk.
**Why it exists:** the repair suite in the Atari feature map, which said ADE already *detected* every Amiga analogue and *computed* the bitmap rebuild, and was missing only the applying. Measured before it was built: of 4,653 corpus images, **607** carry a bitmap finding and on **552** it is the only warning or error; the cleared valid flag alone is on 261, and ADE refused to write to any of them.
**What was declined, and why.** `fixBootChecksum` — a valid bootblock checksum is what makes a Kickstart *run* the boot code, so fixing one is "Make Disk Bootable" by another name, declined under AV-002. Resealing a rootblock or header that fails its checksum — the five corpus disks that do are damaged, and Xenon 2's bad headers are named `0000…`, which a seal would certify. `repairGeometry`, `repairRootOffset` and `bruteForceGeometry` — Atari BPB repairs, and an ADF has no BPB: its geometry is its size.
**Orphans stay used unless asked.** A block the map marks used that nothing reaches is not necessarily free: a trackloaded game keeps data outside any file and marks it used so AmigaDOS never writes there. AmigaDOS's validator frees such blocks; ADE frees them only with `--free-orphans`. The window does not offer it.
**The bitmap is the health report's.** The set of blocks in use comes from the same scan `ade check` judges the bitmap with, so a repaired map and the report cannot disagree about what a file owns. Bits past the end of the volume map nothing and are left as found: ADE's formatter clears them and `Bitmap::rebuild` sets them, and a repair has no business choosing.
**Checked by giving back the bytes.** The built cases damage a disk ADE wrote — the flag cleared, a live block marked free in a map whose checksum then fails, an orphan, a bitmap checksum alone, an entry moved to the wrong slot — on every dostype ADE writes, and require the repair to return the disk **byte for byte**. ADFlib cannot open the misfiled entry by name before the repair and can after.
**Checked against the corpus by writing to it.** Every image was repaired; **270** needed it — 259 flags set, 12 with live blocks marked used again, one entry refiled — and none came out with a warning or error it did not have. Then each was **filled**: one file as large as the free space takes. A map still marking a live block free would put the filler over it, so every file ADE and ADFlib read in full before must read the same after. 261 took the write; ADFlib confirmed 260, and nothing changed.
**What it leaves.** Every misfiled entry the corpus declines shares its name with another entry (TeleEpic II, Typhoon, Valhalla, 1869, Xenon 2), where refiling would change which copy AmigaDOS opens — the owner's call, with `ade rename` and `ade rm` to make it. Three disks whose root directory has hash slots pointing at non-entries keep their bitmap — `ade check` says so as `directory-damaged` since BUG-016 was fixed; and `1000 Miglia`, whose rootblock is noise and names its bitmap outside the volume, is declined: where a bitmap goes is not a repair's to choose (`bitmap-pointer-invalid`, BUG-015).
**Notes:** the repair changes no datestamp: it puts back what the volume already says rather than changing its contents. A disk whose bitmap marks live blocks free now opens for editing, so that it can be repaired; every edit but the repair is refused on it until then.

### F-035 Saying what a disk is
**Priority:** Should
**Effort:** S · **Phase:** 5
**Acceptance:**
- A disk with nothing to list says why, in words a person browsing it can act on, wherever it is shown: the window's tree, `ade check`, `ade ls`, the JSON and the ABI.
- Each claim is read from evidence the disk carries, and none is made that the evidence does not support.
**Status:** Delivered 2026-10-28 — `DiskKind` in the health report, `"kind"` in `ade check`'s JSON (schema **1.19 → 1.20**), a `kind` line in `ade check` and the reason on `ade ls`, `ade_image_kind` / `_summary` / `_explanation` in the ABI, and in the window the disk's row and a note beneath it.
**Why it exists:** asked for from the window. *Zyconix* opened as a disk row reading "no rootblock at block 880: type 0xfb08f1ef, secondary type 0x120f1311" over an empty tree. True, and true of a quarter of real disks, and it explains none of them and reads as damage.
**What a disk can be, and the evidence for each.** **Its own loader**: no volume, and a bootblock Kickstart will run — `DOS`, a valid checksum and code. **A data disk**: no volume, and a bootblock that cannot boot, read by another disk's loader. **A game behind an empty volume**: an empty root over data in blocks the bitmap calls free (IMP-019), or blocks marked used that nothing reaches. **Custom tracks**: a raw-track image without a volume, as copy protection used. **Blank**, **emptied** (only what deleting left), **unformatted** (every byte zero), a **device**, and **unreadable**.
**Measured over the corpus, 2026-10-28:** of 4,652 images, **702** boot their own loader, **339** are data disks — 322 of them disk 2 or later of a set, and the 16 first disks that cannot boot belong to sets numbered from 0 — **107** are games behind an empty volume (87 with the game in free blocks, 20 with it marked used), **13** are blank (save disks: "CoK_Save", "empty", "Amberstar_A"), **5** are raw-track images, and 3,486 hold files. None is all zeros; the kind exists because the built all-zero test image would otherwise have been called another disk's data, which it is not.
**What the window shows.** The disk's row says what the disk is in place of the rootblock message, which moves to the tooltip as the evidence. A disk that would otherwise look broken — its own loader, a data disk, a game behind an empty volume, custom tracks — gets a greyed note beneath it, not selectable or draggable, saying where to look instead — the boot code in the Assembly tab, the disk in the Hex tab, the tracks under Disk ▸ Disk surface — with the explanation in its tooltip. The first version repeated the row's words in the note, which a screenshot showed said the same thing twice. Blank and unformatted disks need no note.
**Notes:** the engine's words name nothing of the window's; the window adds where to look. A loader's bootblock tag — `RNC.`, `ATN!`, `trak` — names the loader's format on many data disks, and is shown as the bootblock's prefix but not interpreted: naming a format from four bytes would be a guess.

## Candidate features (uncommitted)

Nothing is committed here. The **Atari Disk Engine feature map** in [ROADMAP.md](ROADMAP.md#atari-disk-engine-feature-map) (surveyed 2026-08-30) is where unbuilt candidates are currently written down: it separates the genuine gaps from the differences that are already decisions (bootblock writing, themes) and the ones that are simply a different format (MSA/STX, FAT repair). A candidate graduates to an F- number here when it is committed to, not when it is listed there.

**Named and unblocked** (requested 2026-09-01, settled by **D-018** the same day): **saving a modified disk** and **creating a directory on one**. These are the first two writes *into* a disk somebody already owns — what D-004 defers and what v1's never-reversible stance is about, `ade create` having been permitted only because it makes a new file. D-018's terms: edits are buffered in memory, a save is explicit and **never writes over the file the image was opened from**, a save is atomic, and a write is not finished until the result passes `ade check` and mounts under ADFlib. Creating a directory is the first operation — one block, one hash-table insertion, one bitmap bit, and verifiable end to end. **Delivered 2026-10-11 as F-031**, with adding a file alongside it; deletion, renaming and the rest of the suite remain candidates.

- Read support for modern journalling filesystems (SFS, PFS).
- FloppyBridge-compatible output for direct WinUAE/Amiberry use.
- Cataloguing enrichment via Demozoo / ScreenScraper (mirroring ManifeST).
- Track-level visualisation (MFM sync-word map, weak-bit heatmap).
- **FUSE mount of the containers the kernel cannot read** — ADZ, extended ADF, SCP, RDB partitions and reconstructions, letting ordinary tools reach content nothing else can. Replaces the cut F-017, whose ADF/HDF scope the Linux AFFS driver already covers (D-017). Would need an answer for what reading a reconstruction's missing sectors returns.
