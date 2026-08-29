# Vocabulary

> **Status:** Active
> **Provenance:** Claude (drafted 2026-08-29 from ManifeST's own headers)
> **Last reviewed:** 2026-08-29
> **Why this status:** The contract F-013 deferred. ManifeST's `DiskRecord.hpp` and `DiskReader.hpp` define what a cataloguer needs from an engine; this maps ADE's terms onto it and names what is missing.

What ADE calls things, and how those names line up with a ManifeST-style catalogue. Two audiences: someone reading ADE's output who needs to know what a word means, and someone writing a cataloguer against it who needs to know which field to read.

This file was deferred from the start of the project because the contract was undefined. It is defined now: [ManifeST](https://github.com/Darian-Frey) is a batch disk-image cataloguer for Atari ST collections, and its `include/manifest/DiskRecord.hpp` is the shape an engine fills. ADE is the Amiga counterpart of the engine ManifeST already vendors, so this document is written against that struct rather than against an idea of one.

## The words ADE uses

Ordered from the physical outward, which is the order the pipeline reads in (D-003).

| Term | What it means in ADE |
|---|---|
| **flux** | Intervals between magnetic transitions, as a drive's head saw them. Carries what a sector image cannot: weak bits, long tracks, deliberate irregularity. SCP holds this. |
| **track** | One circular path under one head, as a **bit** stream. Not byte-aligned: a sector's sync can and does begin at any bit offset. |
| **cylinder** | The same track position across both heads. A DD floppy has 80; 81–83 occur. |
| **head** / **side** | Which surface. ADE says *head*; ManifeST says *sides* and means the count. |
| **sector** | 512 bytes of decoded data with its own two checksums. Eleven per track on DD, twenty-two on HD. |
| **block** | A sector addressed by number rather than by position — what the filesystem layer sees. `block_size` is 512 in every image ADE has met. |
| **container** | The file wrapper: ADF, ADZ, HDF, HDZ, extended ADF, SCP, DMS, IPF. Decided by content, never by extension (C-008). |
| **device** | An image holding a partition table (RDB) and **no volume of its own**. Every volume is inside a partition. |
| **partition** | A bounded window onto a device, with its own block size and reserved count. Its rootblock is computed from both — never from an offset. |
| **volume** | A mounted AmigaDOS filesystem: rootblock, bitmap, directory tree. What `ls` lists. |
| **dostype** | The four bytes at the start of the bootblock, `DOS\0`…`DOS\7`, decoding to OFS/FFS plus the international and directory-cache flags. |
| **entry** | A file, directory, or link in a directory. |
| **finding** | Something a health check noticed, carrying a **stable code** and a severity. The message is prose and may be reworded; the code may not (F-015). |
| **assembly** / **reconstruction** | A volume rebuilt from raw tracks or flux. Undecodable sectors are zeros, so a reconstruction always reports how much of it is real. |
| **identification** | A name from a dataset, matched by content hash. Not the same as the filename, which may be anything. |

Two distinctions ADE insists on because getting them wrong is silent:

**Container is not filesystem.** A `DOS` prefix does not imply a mountable volume — 19% of real ones are not — and its absence does not imply an unmountable one. They are reported as two independent facts.

**Agreement is not correctness.** Where several dumps of a disk are compared, ADE reports what they agree on and never calls a plurality the truth.

## Mapping onto a ManifeST-style catalogue

`DiskRecord` fields against ADE's JSON (schema 1.2). Field names below are the JSON keys; the command that produces them is in brackets.

| ManifeST `DiskRecord` | ADE | Notes |
|---|---|---|
| `path`, `filename` | `path` [batch] | The caller's own, and ADE echoes it back. |
| `image_hash` | `sha1` [batch `--hash`] | SHA-1 of the file as it sits on disk. **Opt-in** — see below. |
| `format` | `container_code` [batch, info] | `adf`, `extended-adf`, `rdb`, `hardfile`, `gzip`, `dms`, `scp`, `ipf`, `unknown`. Use this, not `container`, which is a sentence. |
| `volume_label` | `volume` [batch], `volume.name` [info] | Latin-1 off the disk, escaped to ASCII in JSON. |
| `sides` | `geometry.heads` [info] | Same number, different word. |
| `tracks` | `geometry.cylinders` [info] | ManifeST's "tracks" is ADE's *cylinders*. Reading one as the other halves or doubles the figure. |
| `sectors_per_track` | `geometry.sectors` [info] | |
| `bytes_per_sector` | `geometry.block_size` [info] | |
| `oem_name` | — | No Amiga equivalent. The Atari BPB carries a creator string; an Amiga bootblock does not. The nearest fact is `bootblock.dostype.label`, and it is not the same thing. |
| `identified_title`, `publisher`, `year` | `matches[].name` [identify] | ADE returns the dataset's **full name** — `Title (Year)(Publisher)[flags].adf` — and does not split it. Parsing the TOSEC convention is the cataloguer's job; ManifeST already does it, and two implementations would drift. |
| `notes` | — | The cataloguer's, not the engine's. |
| `files[].filename` | `name` [ls] | |
| `files[].extension` | — | Split it from `name`; ADE does not, because an Amiga filename has no required extension. |
| `files[].size_bytes` | `size` [ls] | |
| `files[].start_cluster` | `block` [ls] | The file header block. An Amiga file has no cluster chain; the header is the equivalent handle. |
| `files[].file_hash` | `sha1` [ls `--hash`] | SHA-1 of the file's contents. **Opt-in.** |
| `files[].is_launcher` | — | Atari heuristic (the lone `.PRG`/`.APP`/`.TOS`). The Amiga analogue is `s/startup-sequence`, which is a path rather than a guess — a cataloguer can look for it directly. |
| `tags` | `findings[].code` [check], and the dataset name's own flags | ADE emits fault codes, not tags. `[cr ...]`, `[t ...]`, `[m ...]` live in the identified name. |
| `text_fragments` source `"boot"` | `boot_text` [info] | A direct match. ADE extracts printable runs from boot code, filtered so the result is prose rather than 68k opcodes, and **draws no conclusion from them** (D-014). |
| `text_fragments` source `"deep"` | — | Whole-image string scanning is not something ADE does. |
| `menu_games`, `detected_games` | — | Cracker-menu-disk concepts from the ST scene. The Amiga equivalent would be a different dataset, and none is in hand. |
| `file_mtime`, `file_size` | `size` [batch], and the caller's `stat` | |

### Hashing is opt-in, and that is the contract

`--hash` adds `sha1` to a batch record and to each file in a listing. Without it there is no hash anywhere, and the field is `null` rather than absent.

The reason is cost, measured end to end rather than estimated: over the 4,652-image corpus, `ade batch` takes **5.81 s** and `ade --hash batch` takes **18.53 s**. SHA-1 runs at 349 MB/s and the corpus is 4.2 GB, so the arithmetic and the stopwatch agree. A cataloguer wants the hash — it is the key duplicates are found with, and the key ScreenScraper-style enrichment looks up. A health run has no use for it at all. Making it a flag lets both have what they need, and mirrors the quick/deep split ManifeST already draws for the same reason.

`container_code` is **not** behind the flag, because it costs nothing.

### What ADE will not do for a cataloguer

- **Parse dataset names into title/publisher/year.** The convention belongs to TOSEC, the cataloguer already implements it, and a second implementation is a second thing to be wrong.
- **Guess a launcher.** ADE will tell you `s/startup-sequence` exists; what that means is a judgement.
- **Draw conclusions from boot text.** It reports what the bootblock says. Whether "NO VIRUS ON BOOTBLOCK!" is a reassurance or a lie is a question ADE declines to answer (D-014).

## Stability

Everything above is the **JSON** surface, versioned and enforced: field names and fault codes are a commitment, every document carries `schema`, and an inventory test fails on any change (D-015, F-015). Text output is for people, is explicitly not parseable, and may be reworded without notice.

A cataloguer should read `schema` and refuse a major version it does not know. Additions arrive as minor versions and are safe to ignore.
