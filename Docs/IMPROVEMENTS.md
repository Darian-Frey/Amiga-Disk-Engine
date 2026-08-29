# Improvements

Catalogue of code-quality improvements, refactors, and architectural changes proposed during development. Per Maintenance Rule 8, improvements are logged here when noticed, not silently applied. The author decides whether to apply, defer, or decline.

This is the dual of [BUGS.md](BUGS.md): bugs are broken; improvements work but could be better.

Status vocabulary: suggested | applied | declined | deferred.
Effort vocabulary: trivial | small | medium | large.

> First entry logged 2026-08-22. Use `IMP-001`, `IMP-002`, … sequentially. Every entry needs a **Trade-offs** field, or it is a feature request, not an improvement candidate.

## Suggested

### IMP-008 A directory expansion costs 9 ms, and it is not the bytes
**Status:** suggested
**Effort:** medium
**Found:** 2026-08-29, measuring IMP-005 and needing a baseline to compare against.
**Where:** [bridge/src/lib.rs](../bridge/src/lib.rs) `with_volume`, and `Volume::mount` beneath it.

**What works today.** Expanding a drawer in the GUI lists a directory correctly and fast enough that nobody has complained.

**Why it could be better.** At scale it is not fast. Over 60 images and 3,827 rows, expanding every one takes **35 seconds** — about 9 ms each — and selecting every row takes 12 seconds. Neither figure moved when the image stopped being held in memory (IMP-005: 35.1 s against 35.5 s), so the cost is not the bytes and not the file.

What it is, most likely, is that every call through the ABI mounts a fresh `Volume`: `with_volume` re-reads the bootblock and rootblock, and the filesystem re-derives whatever it derives, once per directory listed and once per file previewed. IMP-006 removed the *container* rebuild from that path; the volume mount is what is left.

**Trade-offs.** Caching a mounted `Volume` on the handle means it borrows the `Image` in the same struct — self-reference, which is the thing IMP-006 was pleased to avoid. The alternatives are a mount that is cheap enough not to matter, or a listing call that does the walk itself rather than being called per directory. Measuring which part of the 9 ms is the mount comes first: this entry names a symptom and guesses at a cause, which is not the same as knowing.

**Not urgent.** A person expands one drawer at a time and will not notice 9 ms. The number matters because it is the whole cost of the GUI's tree at scale, and because it is now the largest thing left that a measurement has found.

## Applied

### IMP-005 An open image is held whole in memory, so the GUI scales with images open
**Status:** applied
**Effort:** medium
**Found:** 2026-08-27, measuring the GUI's cross-image search against 400 corpus images.
**Where:** [bridge/src/lib.rs](../bridge/src/lib.rs) `ade_image_open`, and the `ade-core` open path beneath it.

**What works today.** Opening reads the file into memory and keeps it there for the lifetime of the handle. 400 floppy images opened in 580 ms and searched in 79 ms — but at **400 MB resident**, almost exactly the sum of their sizes. The CLI never notices, because `ade batch` reads one image, examines it and drops it; that is how a 4652-image corpus runs at 9 MB peak. The GUI does notice, because the whole point of the multi-image model is that every image stays open.

**Why it could be better.** A person cataloguing a collection may reasonably drop a few hundred floppies in, and a hardfile is not 880 KB — one 500 MB HDF is larger than the entire floppy corpus opened at once. The ceiling is not a floppy count, it is total bytes, and nothing currently reports approaching it.

**Trade-offs.** Memory-mapping or a windowed reader would cut resident size to what is actually touched, but both break the current guarantee that a decompressed container (ADZ, HDZ) and a plain one behave identically — a gzip stream has no seekable backing file, so one of them must stay materialised. It would also put I/O errors on paths that today cannot fail, which is a change to every signature that reads a block. The honest interim measure is far cheaper: report the total resident cost in the GUI and let a person see it climb.

**Partly addressed 2026-08-28 by IMP-006.** What is held is now the *assembled* image rather than the file, so a flux capture costs its 880 KB volume instead of its 30 MB of timings, and a compressed container costs what it decompresses to. Four captures fell from 153.6 MB resident to 36.8 MB. A plain ADF is unchanged, since assembled and raw are the same bytes — so the 400-floppy figure above still stands, and this entry with it.

**Not urgent.** 400 images is well beyond what the acceptance criteria ask for, and the failure mode is a slow machine rather than a wrong answer.

**Applied 2026-08-29.** `ade-container::FileSource` is a `BlockSource` that reads each block from the file when it is asked for, and `Image::open_lazy` uses it. The bridge opens that way, because a front end holds every image it opens; the CLI still opens eagerly, because it holds one and exits.

| 400 plain ADFs held open | before | after |
|---|---|---|
| resident | 364 MB | **12.9 MB** |
| on disk | 344 MB | 344 MB |

**28× less**, and now far below the images' own size rather than slightly above it: what remains is a file handle and an `Inspection` each, about 32 KB per image. What the operating system caches is its own affair, reclaimable and shared with every other reader of the file.

**It cost 1–3% of time, measured rather than assumed.** Over 60 images and 3,827 rows: expanding 35.1 s → 35.5 s, selecting 11.6 s → 11.9 s, searching 19 ms → 23 ms. The first figure looked alarming until it was compared — 9 ms per directory expansion is **pre-existing** and has nothing to do with where the bytes live. It is now [[imp-008]].

**No `unsafe` and no dependency.** Memory-mapping is the obvious answer and is unavailable twice over: D-006 forbids `unsafe` and the workspace has no dependencies, so every mmap crate is one or the other. Positional reads — `read_at` on Unix, `seek_read` on Windows — are safe, in `std`, and enough.

**The trade this entry predicted is real and is now tested.** An eagerly opened image is a snapshot; a lazily opened one is a *window*, and the file underneath it can be truncated or replaced. `src/api/tests/lazy.rs` pins both halves: the same answers while the file is whole, an error or an empty listing rather than stale content when it is not, and `Image::open` still indifferent to the file vanishing. That is why lazy is a separate call rather than the default — a front end holding many images wants it, and a command holding one does not.

**The entry's other prediction was wrong in a useful way.** It expected this to "break the current guarantee that a decompressed container and a plain one behave identically". It does not: `open_lazy` sniffs the container and falls back to reading whole for anything whose blocks are not its file — gzip wrappers, flux captures, reconstructions. The guarantee holds because the fallback is inside the call rather than left to the caller.


### IMP-007 Conversion logic lives in the CLI, where F-002 says it must not
**Status:** applied
**Effort:** small
**Found:** 2026-08-29, starting F-014's bulk-convert clause and discovering there was nothing for `batch` to call.
**Where:** [cli/src/main.rs](../cli/src/main.rs), `convert` and `encode_raw_mfm`.

**What works today.** `ade convert` reads the image, decides the target, checks the verdict, decompresses a gzip wrapper, encodes raw MFM where `--raw` is given, and writes the result — all inside the CLI. It is correct and it has worked since F-016 landed on 2026-08-25.

**Why it could be better.** `encode_raw_mfm` is sixty lines that derive a geometry, split an image into tracks, call `encode_track` per track and assemble an extended ADF. That is engine work, and **F-002's acceptance says in terms that no engine logic may live in UI code.** The layering check cannot see it: `ade-cli` is allowed to depend on `ade-core`, and this is a function *inside the CLI* using `ade-core`'s layers directly, which is a different thing from a cross-crate edge.

It became visible the moment a second caller wanted it. `ade batch` cannot convert a corpus without either calling into the CLI — impossible — or duplicating sixty lines of track encoding, which is how two implementations of one algorithm start.

**Trade-offs.** None worth the name: the move is mechanical, and `ade-core::convert` already owns the *decision* half of conversion (the matrix). Splitting the decision from the doing is the odd arrangement, not joining them.

**Applied 2026-08-29** as part of F-014's bulk-convert clause rather than separately, because the clause cannot be built without it. Recorded here rather than fixed silently (Maintenance Rule 8) — the point of the rule is that the change is visible, not that it must wait.


### IMP-006 The C ABI rebuilds the whole image on every call that reads it
**Status:** applied
**Effort:** small
**Found:** 2026-08-28, extending the bridge with partition support and reading the code it would sit beside. **Measured the same day**, and the measurement changed the entry — see below.
**Where:** [bridge/src/lib.rs](../bridge/src/lib.rs), `ade_dir_open`, `ade_walk_open`, `ade_file_read`, and `with_volume` beneath all three.

**What works today.** Each call that reads a volume does `Image::from_bytes(image.bytes.clone())`, because `AdeImage` stores the bytes while `Volume` borrows from an `Image`. It is correct.

**The clone is the smaller half.** `Image::from_bytes` also runs `assemble_container`, so what repeats per call is not a memcpy but the **whole container reconstruction**: an SCP has its 160 tracks of flux decoded again, an ADZ is decompressed again, an extended ADF is reassembled again. Every directory expansion, every file preview, every drag.

**Measured in the GUI, offscreen, on this machine:**

| set | per interaction |
|---|---|
| plain ADFs (880 KB) | ~16 ms to expand a drawer, ~20 ms to select a row |
| with 30 MB SCP captures | **~103 ms to expand, ~131 ms to select** |

That is the difference between a window that feels instant and one that does not, and it grows with the image. A 500 MB hardfile would copy 500 MB per click.

**When I logged this I wrote that it was "invisible on a floppy: 880 KB copied per directory expansion".** That was true and it was the wrong measurement — it accounted for the copy and not for the decode, which is the part that scales with how interesting the image is. Flux and compressed containers are exactly the images someone opens a *browser* for.

**Trade-offs.** The fix looks like storing an `Image` in `AdeImage` rather than a `Vec<u8>` — `Image::from_bytes` takes ownership, nothing else needs the raw bytes, so no self-reference is involved and no signature changes. What needs checking first is whether anything depends on the bytes outliving a mounted volume, and whether `Inspection` and `Image` can be held side by side without the borrow checker forcing one into an `Rc`. If it does force that, the cure is worse: a reference count in the one crate that writes `unsafe` is a lifetime question moved rather than answered.

**Deliberately not fixed inline** (Maintenance Rule 8). It was noticed while adding code next to it, which is when a cleanup is most tempting and least reviewed.

**Applied 2026-08-28.** `AdeImage` holds the mounted `Image` instead of the raw bytes, and the health count is computed once at open rather than per call. Nothing borrowed had to become reference-counted, which was the trade-off this entry was worried about: `Inspection` and `Image` are independent values and sit side by side without complaint.

| with two 30 MB flux captures open | before | after |
|---|---|---|
| expand 84 drawers | 8681 ms | **63 ms** |
| select 84 rows | 11023 ms | **1115 ms** |
| search across them | 839 ms | **1 ms** |
| drag out 12 files | 1276 ms | **0 ms** |

**And it cut memory rather than trading for it**, which is the opposite of what caching usually costs. Four 30 MB captures held 153.6 MB resident before and **36.8 MB** after — below the on-disk size, because what is now kept is the *assembled* 880 KB volume rather than the flux it came from. That improves [[imp-005]]'s figures for every container that is not already a plain ADF.

**The mounted image had to become optional**, and that is the part worth remembering. A container ADE cannot mount — a truncated file, an unrecognised format — still opens, because the container and the reason are exactly what a person wants from such a file, and a quarter of real images hold no AmigaDOS volume. The first version returned null there and would have made the GUI refuse to describe the very disks someone is puzzled by. `bridge/tests/abi.rs` pins it.

**The finding count is now cached**, so a second risk appeared: a cached number can quietly become a *different* number. The test checks it against `examine`'s own answer rather than against itself.


### IMP-004 The fixture generator cannot build file extension blocks
**Status:** applied
**Effort:** small
**Found:** 2026-08-24, writing a hardfile test whose file needed 82 data blocks.
**Where:** [tools/fixtures/src/lib.rs](../tools/fixtures/src/lib.rs), `Volume::add_file`.

**What works today.** Files up to 72 data blocks — about 35 KB on OFS, 36 KB on FFS — which covers every fixture written so far. Beyond that the generator asserts and says so, loudly:

```
fixture files must fit one header block; extension blocks are not built yet
```

**Why it could be better.** A larger file stores the overflow pointers in *file extension blocks*, chained from the header's `extension` field (SPEC §Files). ADE **reads** that chain — it is how `taterm1` on the Amigan Radio disk was recovered, following an extension block to seven more data pointers — but nothing can **generate** one.

So the extension-chain path in `read_file` is exercised only by real corpus disks. It has no fixture, which means no test on a fresh clone and, more importantly, **no oracle**: D-010's amendment turns on ADFlib being able to read what we generate, and we cannot generate this at all. The visited set guarding that chain against loops (AV-001) is likewise untested by fixture.

It also caps what can be tested on hardfiles, where multi-megabyte files are the normal case rather than an edge one.

**Suggested fix.** Allocate extension blocks in `add_file` when the data-block count exceeds the hash-table size: `T_LIST` primary type, `parent` naming the file header, the reversed pointer table as in the header, and `extension` chaining onward. Then generate a file spanning several and cross-check it against `unadf`.

**Trade-offs.** More generator surface to keep correct, and the generator is already the thing BUG-004 and BUG-006 were found in — each new structure is somewhere else a fixture can be quietly wrong. Against that: it is the only way to bring the extension path under the oracle, and a reader path validated by neither fixture nor independent implementation is exactly what D-010 exists to prevent.

**Related.** D-010, AV-001, SPEC §Files.

**Applied 2026-08-24.** `add_file` allocates one extension block per group of pointers beyond the header's table, chains them through `extension`, and numbers `seq_num` and `next_data` across the whole file rather than per header block — the file-wide numbering being what `read_file` already assumed.

**Validated against ADFlib.** A 200 KB file spans six extension blocks past the header's 72 pointers, and both readers extract it byte-identically, for OFS and FFS alike. The oracle test now covers eleven generated volumes and four content round-trips, two of them across extension chains.

**What it unlocked.** The visited set guarding the extension chain against loops (AV-001) had never been exercised by a fixture — only by whatever real disks happened to contain. Now `corrupt::extension_chain_loop` and `extension_chain_cycle` build both shapes, and the tests confirm each terminates with a reported cycle rather than spinning. Wild `extension` pointers (AV-004) and a chain block claiming the wrong primary type are covered too.

The two-block cycle matters separately from the self-loop: a "next != self" check catches the first and misses the second, which is the same reasoning that shaped the hash-chain tests.


### IMP-003 `walk` and `read_file` are bounded only by their visited sets
**Status:** applied
**Effort:** small
**Found:** 2026-08-23, by mutation-testing the fuzz harness — deliberately removing the visited set from `Volume::walk` to confirm the fuzzer would notice.
**Where:** [src/filesystem/src/volume.rs](../src/filesystem/src/volume.rs), `Volume::walk` and `Volume::read_file`.

**What works today.** Both terminate correctly on every input tried: 900,000 fuzz cases and a 4652-image corpus, with no runaway. The visited sets are right, and D-006 is satisfied.

**Why it could be better.** `walk` accumulates into an unbounded `Vec`, and its boundedness depends *entirely* on one `HashSet::insert` returning false. There is no second line of defence on the invariant AV-001 exists to protect. Removing that single call makes ADE allocate **28.8 GB** and take the host down with it — the same failure shape as ADFlib's 29 GB blow-up recorded in SPEC §Corpus observations, from ADE's own code.

`read_file` is bounded more loosely than it looks: extension-chain length × 72 pointers × block size, which on a floppy permits roughly 64 MB for a file that cannot exceed 880 KB.

Two consequences beyond tidiness. A genuine regression in the cycle detection would not fail CI, it would **exhaust the runner's memory** — the fuzz harness asserts on `walked.len()` after the call returns, which is exactly what would never happen. And the invariant cannot be mutation-tested without risking the developer's machine, which is how this was found.

**Suggested fix.** Give both a hard structural cap that does not depend on the visited set being right:

- `walk` stops at the volume's block count. A volume cannot contain more entries than it has blocks.
- `read_file` stops at the volume's total bytes. A file cannot exceed the volume that holds it.

Hitting either cap is a **fault to report**, not a silent truncation — it means the visited set failed, which is a finding in itself. `FileContents` already has somewhere to say so; `walk` would need a result type rather than a bare `Vec`.

**Trade-offs.** A redundant check is dead weight while the primary defence holds, and two mechanisms enforcing one invariant can rot apart — the cap could drift wrong and nobody would notice, because the visited set gets there first. Against that: the cap is two comparisons, it converts a host-killing failure into a test failure, and it is the difference between an invariant that is *tested* and one that is *asserted*. D-006 forbids unbounded allocation on a parse path; resting that on a single line, for the specific vector rated Critical, is thinner than the decision intends.

**Related.** AV-001, D-006, F-001, and SPEC §Corpus observations for the reference implementation doing exactly this.

**Applied 2026-08-24.** `walk` returns a `Walk` struct rather than a bare `Vec`, carrying `hit_limit`, and stops at three structural bounds that do not consult the visited set: entry count, pending-directory count, and **depth**. `read_file` stops at the volume's total bytes.

Depth was not in the original plan and turned out to be the one that mattered. Bounding the entry count is not enough: each path is built from its parent's, so a cycle makes the *strings* grow without bound — `a/b/a/b/a/b/…` — while the count stays comfortably inside its cap. The first version of this fix still reached 4 GB.

**It found a real vulnerability.** Chasing that remaining 4 GB showed it was not in `walk` at all: `read_file` reserved `Vec::with_capacity(byte_size)` from a `u32` read off the disk, so a crafted header made ADE allocate 4 GB before reading anything. Logged and fixed as **BUG-003**. The improvement was hardening; the bug it uncovered was live.

**Verified by mutation.** Removing the visited set now produces a clean test failure in one second, naming the seed:

```
seed 1: the structural cap fired on a deliberately hostile image
        — the visited set should have stopped it first (IMP-003)
```

Previously the same mutation allocated 28.8 GB and the kernel killed the session. That is the whole point: the invariant is now *tested* rather than *asserted*.

The fuzz harness asserts `!hit_limit` on mutated and hostile images, so the cap firing is itself a failure — it is a backstop, not a mechanism, and a walk stopped by the backstop means cycle detection broke.


### IMP-002 OFS data blocks are read without validating their headers
**Status:** applied
**Effort:** small
**Found:** 2026-08-23, from an oracle disagreement on a corpus of 4652 images.
**Where:** [src/filesystem/src/volume.rs](../src/filesystem/src/volume.rs), `Volume::read_file`.

**What was wrong.** For OFS, `read_file` took each data block's `data_size`, clamped it to the 488-byte payload, and copied from offset 24 — never checking that the block *was* a data block. An OFS data block carries `type == T_DATA (8)`, a `header_key` pointing back at its file header, and a `seq_num` counting from 1; none was examined.

On `A500+A2000 Systest v9.1 …[cr A-Ha].adf`, the file `instruments/thriller2.ss` has table entries past index 38 pointing at blocks holding raw audio: `type`, `seq_num` and `data_size` all read `0x6db66db6`. ADE clamped the absurd length to 488 and copied sample data as payload, silently, producing plausible output with no sign the structure had stopped making sense a third of the way in.

**Applied 2026-08-24.** `read_file` now checks each OFS data block against what the table claimed of it, and records what it finds in `FileContents::faults`.

Nothing is refused. The bytes are read regardless, because refusing to recover data is the one thing a forensic tool must not do (D-012); an oversized `data_size` is still clamped rather than trusted. What changes is that the doubt is visible.

Faults are **summarised by kind**, not enumerated — one real file has 18 zeroed blocks in a row, and eighteen identical lines would bury the finding. Coalescing is on the fault's discriminant rather than on equality, so twenty blocks each naming a *different* wrong owner still report as one cross-linking finding.

`is_complete()` now means full-length **and** structurally sound; `is_full_length()` preserves the old meaning, because the two are genuinely different questions.

**What it caught immediately**, on the two disks that prompted the entry:

```
taterm1: recovered 29768 of 38320 declared bytes — 8552 short
taterm1: data block is entirely zero (18 blocks, first at 1749)

instruments/thriller2.ss: not a data block: type 0x6db66db6, expected 8 (22 blocks, first at 33)
```

The first now explains *why* it came up short. The second was previously **silent** — it returned exactly its declared length and looked like a clean read.

**The noise worry was unfounded.** Across a 388-disk sample and 4128 extracted files: 20 files carry a structural fault (0.48%), on 10 disks. All four kinds occur in the wild, including cross-linked blocks and out-of-sequence numbering. Rare enough to be a finding rather than a flood.

FFS reports nothing here, having no data-block header to check — C-005's forensic asymmetry showing up in the tooling.

**Trade-offs (as recorded before applying).** Strictness risks rejecting data a lenient reader would recover, so these are reports and never refusals. The noise cost was the other worry, and measurement settled it.

**Related.** F-010, D-006, C-005, D-012.


### IMP-001 `ade ls` output is not machine-parseable
**Status:** applied
**Effort:** small
**Found:** 2026-08-22, while validating extraction across the corpus.
**Where:** [cli/src/main.rs](../cli/src/main.rs), `list`.

**What works today.** The listing is readable and correct: size, protection bits, datestamp, name, optional comment, aligned in columns.

**Why it could be better.** The columns are whitespace-separated and two fields contain spaces — the datestamp (`1990-09-20 17:10:20`) and, more awkwardly, Amiga filenames, which routinely contain spaces. Splitting on whitespace therefore cannot recover the name. I hit this writing a validation script and misread 4283 files as extraction failures before realising the parser, not the tool, was wrong.

F-015 commits to "a documented, versioned CLI and library binding suitable for automation, with stable exit codes and **structured output**". The exit codes are done; the structured output is not.

**Suggested fix.** A `--format` flag offering at least a machine-readable mode. Tab-separated with the name last is the cheap option; JSON Lines is the more useful one for F-014's batch reporting, and would suit `ade info` equally, whose evidence list and fault list are currently prose.

**Trade-offs.** A second output path is a second thing to keep correct, and committing to a schema early risks having to break it — which for a stability promise is worse than not having made it. Against that: the schema is easier to design now, while the only consumers are ADE's own tests, than after F-014 has scripts depending on the human format. Doing nothing means the scriptable surface F-015 promises does not exist, and anyone automating ADE will parse the human output and be broken by the first column change.

**Related.** F-014 (batch reporting needs machine-readable summaries), F-015 (the promise itself).

**Applied 2026-08-22.** A `--format=json` flag on `ade info` and `ade ls`, with `--format=text` remaining the default and explicitly documented as unstable.

`ls` emits JSON Lines — one object per entry, so a large directory streams rather than buffering. `info` emits a single object carrying its evidence and fault lists as real arrays instead of prose. Names are written as `\uXXXX` escapes, so output is **pure ASCII** and a Latin-1 filename round-trips losslessly: a name is a sequence of bytes on the disk, not text, and a consumer must be able to get the original bytes back.

Written with an internal `ade_core::json` writer rather than `serde`, keeping the workspace at zero dependencies. The scope is narrow — ADE emits JSON and never parses it — so the whole escaping surface is one function.

**Two things came out of doing it.**

Fault computation moved from the CLI into `Inspection::faults()`, with a typed `Fault` carrying a **stable code** alongside its message. The message is for people and may be reworded; the code is the contract. That also means the human and JSON outputs cannot drift apart about what is wrong with an image, and a future GUI inherits the same list.

It exposed a genuine bug: `ade ls --format=json disk.adf | head` **panicked**, because `println!` panics on a closed pipe. For a tool designed to be piped that is unacceptable. Restoring the default `SIGPIPE` disposition needs a `libc` call behind `unsafe`, which the workspace forbids, so all output now goes through an `emit` helper that treats a closed pipe as the ordinary end of a command.

**Verified** across all 4288 corpus images: 4288 `info` objects and 68,961 JSON Lines, every one parsing, zero non-ASCII bytes, zero malformed output.


## Declined

_None._

## Deferred

_None._
