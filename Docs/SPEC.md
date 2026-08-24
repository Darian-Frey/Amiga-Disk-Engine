> **Status:** Active
> **Provenance:** Claude (research pass against primary sources, 2026-08-22)
> **Last reviewed:** 2026-08-22
> **Why this status:** Filled in from primary documentation for Phase 1. Sections marked *Unverified* are deferred to the phase that needs them.

# Spec — Amiga disk & filesystem formats

The authoritative technical reference for the formats ADE reads and writes. This is the **how it works** (structures, constants, layouts); the **what it does** is [FEATURES.md](FEATURES.md) and the **why** is [DECISIONS.md](DECISIONS.md).

## Sources

Cited inline by key. These are ground truth; this document records ADE's working understanding of them.

| Key | Source |
|---|---|
| **[FAQ]** | Laurent Clévy, *The .ADF (Amiga Disk File) format FAQ* — <http://lclevy.free.fr/adflib/adf_info.html>. Section numbers refer to that document. |
| **[AFFS]** | Linux kernel AFFS driver documentation — <https://www.kernel.org/doc/html/latest/filesystems/affs.html> |
| **[AOS-LNFS]** | *DCFS and LNFS Low Level Data Structures*, AmigaOS Documentation Wiki — <https://wiki.amigaos.net/wiki/DCFS_and_LNFS_Low_Level_Data_Structures> |
| **[SCP]** | Jim Drew, *SuperCard Pro Image File Specification v2.5* — <https://www.cbmstuff.com/downloads/scp/scp_image_specs.txt> |
| **[RKRM]** | *Amiga ROM Kernel Reference Manual: Devices*, Appendix C — MFM track format. Not yet consulted; Phase 4. |
| **[CORPUS]** | Direct survey of 4288 TOSEC Amiga ADF images held locally, 2026-08-22. **Observation, not specification** — it records what real images do, which is frequently not what the documentation says. See §Corpus observations. |

**ADFlib's source is deliberately absent.** Under D-002 it is a black-box differential-test oracle, run as a separate binary and diffed against. Reading it to inform this document would muddy provenance and forfeit the licence freedom that decision preserves.

## Conventions

- **Endianness.** All multi-byte on-disk integers are big-endian (68k "Motorola order"). Values are unsigned unless noted. [FAQ §1.3]
- **`BSIZE`** is the block size in bytes — 512 for floppies, configurable for hard disks (512/1024/2048/4096) [AFFS]. Layouts below are given as `offset` from the block start, with negative offsets written as `BSIZE-n` because many fields are anchored to the block *end*.
- **Block pointers** are block numbers, counting from 0. *Logical* pointers are relative to the start of a volume, *physical* to the start of the media; for floppies they coincide. Within a partition, pointers are relative to the partition's first block. [FAQ §1.3, §6.3]
- A **word**/**short** is 2 bytes, a **long** is 4 bytes.

## Layers

ADE spans six format layers (see [ARCHITECTURE.md](ARCHITECTURE.md) for the module mapping):

1. **Flux** — SCP, extended-ADF (MFM track data), optional IPF-read.
2. **Track / MFM** — MFM encoding, sync words (0x4489), track gaps.
3. **Sector / block** — 512-byte blocks, checksums, the allocation bitmap.
4. **Filesystem** — OFS/FFS, dostypes, RDB partitioning.
5. **Object model** — files, directories, links, comments, metadata.
6. **Catalogue** — content hashes and dataset identity (not a disk format, but the terminal representation).

## Geometry

| | bytes/sector | sectors/track | tracks/cyl | cyls |
|---|---|---|---|---|
| DD floppy | 512 | 11 | 2 | 80 |
| HD floppy | 512 | 22 | 2 | 80 |

Block order is increasing sector, then side, then cylinder. A DD disk holds 1760 blocks (901,120 bytes — the canonical 880 KB ADF); an HD disk holds 3520 (1,802,240 bytes). [FAQ §3]

**80 cylinders is the norm, not the limit.** Drives could generally seek several tracks beyond cylinder 79, and images of 81, 82 and 83 cylinders occur in the wild — exactly `cylinders × 2 × 11 × 512` bytes, so 912,384 / 923,648 / 934,912. Extended-ADFs commonly declare 166 tracks (83 cylinders). Geometry handling must be parameterised on cylinder count rather than assuming 80. See §Corpus observations.

### Locating the rootblock

The rootblock sits at the volume midpoint, but **it must be computed, not read from the bootblock** — see C-007. [FAQ §4.2]

```
numCyls = highCyl - lowCyl + 1
highKey = numCyls * numSurfaces * numBlocksPerTrack - 1
rootKey = (numReserved + highKey) / 2        # integer division
```

For a DD floppy: `highKey = 1759`, `numReserved = 2`, so `rootKey = 880`. For HD: `rootKey = 1760`. Note that `(numReserved + highKey) / 2` coincides with `total_blocks / 2` only when `numReserved == 1`; for partitions with other reserved counts the two diverge, so the documented formula is the one to implement.

## Checksums

**Two different algorithms, at two different offsets.** Confusing them is a silent-corruption bug, not a loud one.

### Bootblock checksum — offset 4, add-with-carry then complement

Over the whole bootblock (1024 bytes on floppy; `Bootblocks * BSIZE` on hard disk), with the checksum field zeroed first: [FAQ §4.1]

```
sum = 0
for each big-endian u32 d in block:
    prev = sum
    sum = (sum + d) mod 2^32
    if sum < prev:          # carry out
        sum = sum + 1
sum = NOT sum               # one's complement
```

### Normal block checksum — offset 20, plain sum then negate

Used by every other block type — rootblock, directory, file header, file extension, OFS data, bitmap, dircache, and the RDB family. With the checksum field zeroed first: [FAQ §4.2.3]

```
sum = 0
for each big-endian u32 d in block:
    sum = (sum + d) mod 2^32
sum = -sum                  # two's complement negation
```

## Bootblock

Blocks 0–1 on a floppy. On a hard disk the count comes from `DosEnvVec->Bootblocks`. [FAQ §4.1]

| offset | type | count | name | meaning |
|---|---|---|---|---|
| 0x00 | char | 4 | DiskType | `'D' 'O' 'S'` + flags byte |
| 0x04 | ulong | 1 | Chksum | bootblock checksum (see above) |
| 0x08 | ulong | 1 | Rootblock | **unreliable — see C-007** |
| 0x0c | char | * | boot code | 1012 bytes on floppy |

AmigaDOS executes the boot code when the checksum and DiskType are valid. **ADE never executes it** (AV-002, D-006); the bootblock is parsed, checksummed, and virus-scanned only (F-011).

A bootblock beginning `PFS` indicates the Professional File System, not AmigaDOS. [FAQ §4.1] Other non-`DOS` prefixes exist (`SFS`, `KICK`); ADE reports them rather than guessing.

## Dostypes

The flags byte after the `DOS` prefix. [FAQ §4.1]

| Bit | Set | Clear |
|---|---|---|
| 0 | FFS | OFS |
| 1 | INTL only | no INTL |
| 2 | DIRC **and** INTL | no DIRC, no INTL |

| Value | Name | Filesystem | INTL | DIRC | Linux AFFS |
|---|---|---|---|---|---|
| `DOS\0` | OFS | OFS | no | no | read/write |
| `DOS\1` | FFS | FFS | no | no | read/write |
| `DOS\2` | OFS-INTL | OFS | yes | no | read/write |
| `DOS\3` | FFS-INTL | FFS | yes | no | read/write |
| `DOS\4` | OFS-DC | OFS | **yes** | yes | read-only |
| `DOS\5` | FFS-DC | FFS | **yes** | yes | read-only |
| `DOS\6` | OFS-LNFS | OFS | **yes** | no | unsupported |
| `DOS\7` | FFS-LNFS | FFS | **yes** | no | unsupported |

Support column from [AFFS], which documents only `DOS\0`–`DOS\5`.

### The dircache trap (C-006)

> "If the dircache is enabled, its flag is set (bit #2), and the international mode is also enabled, but the related flag (bit #1) will stay cleared." — [FAQ §4.1]

So `DOS\4` and `DOS\5` carry the dircache bit with the INTL bit **clear**, yet international hashing applies. Reading bit 1 alone to decide the hash function produces lookups that fail on exactly those disks — and fail *quietly*, as a "file not found" rather than an error. [AOS-LNFS] states LNFS likewise "always uses the international directory entry name hashing operation", so `DOS\6` and `DOS\7` are international too.

**International hashing applies when bit 1 OR bit 2 is set, or when the dostype is `DOS\6`/`DOS\7`.**

### LNFS (`DOS\6`, `DOS\7`)

A later extension from Olaf Barthel's FFS reimplementation (AmigaOS 4 era), raising the 30-character name limit to ~106. Structurally: the separate name and comment fields are replaced by a 112-byte **NaC** ("name and comment") array holding both as consecutive BCPL strings; if they do not fit, the comment moves to a dedicated `TYPE_COMMENT` block referenced by a new `CommentBlock` field. The rootblock gains `NumBlocksUsed` and `FileSystemType`. There is no DCFS variant of LNFS. [AOS-LNFS]

Phase 2 work. Detail above is summary-level and needs a second pass before implementation.

## Rootblock

`BSIZE` bytes at the computed midpoint. [FAQ §4.2]

| offset | type | count | name | meaning |
|---|---|---|---|---|
| 0x00 | ulong | 1 | type | `T_HEADER` = 2 |
| 0x04 | ulong | 1 | header_key | unused (0) |
| 0x08 | ulong | 1 | high_seq | unused (0) |
| 0x0c | ulong | 1 | ht_size | hash table size in longs = `BSIZE/4 - 56` (72 for 512) |
| 0x10 | ulong | 1 | first_data | unused (0) |
| 0x14 | ulong | 1 | chksum | normal checksum |
| 0x18 | ulong | ht_size | ht[] | hash table — entry block numbers |
| BSIZE-200 | ulong | 1 | bm_flag | **-1 means valid** |
| BSIZE-196 | ulong | 25 | bm_pages[] | bitmap block pointers |
| BSIZE-96 | ulong | 1 | bm_ext | first bitmap extension block (hard disk) |
| BSIZE-92 | ulong | 1 | r_days | last root change — days since 1978-01-01 |
| BSIZE-88 | ulong | 1 | r_mins | minutes past midnight |
| BSIZE-84 | ulong | 1 | r_ticks | ticks (1/50 s) past the minute |
| BSIZE-80 | char | 1 | name_len | volume name length |
| BSIZE-79 | char | 30 | diskname[] | volume name |
| BSIZE-40 | ulong | 1 | v_days | last volume change |
| BSIZE-36 | ulong | 1 | v_mins | |
| BSIZE-32 | ulong | 1 | v_ticks | |
| BSIZE-28 | ulong | 1 | c_days | volume creation (format) date |
| BSIZE-24 | ulong | 1 | c_mins | |
| BSIZE-20 | ulong | 1 | c_ticks | |
| BSIZE-8 | ulong | 1 | extension | first dircache block, else 0 |
| BSIZE-4 | ulong | 1 | sec_type | `ST_ROOT` = 1 |

### Datestamps

Days since 1978-01-01, minutes past midnight, ticks at 1/50 s. Constraints: `0 <= mins < 1440`, `0 <= ticks < 3000`. A `days` value of zero is treated as illegal by most Amiga software. [FAQ §4.2] ADE surfaces out-of-range datestamps rather than normalising them.

`r_*` tracks the root directory's last change, `v_*` any change to the volume, `c_*` the format date and never changes afterwards.

## Directory hashing

```c
int HashName(unsigned char *name) {
    unsigned long hash, l;
    hash = l = strlen(name);
    for (int i = 0; i < l; i++) {
        hash = hash * 13;
        hash = hash + toupper(name[i]);
        hash = hash & 0x7ff;
    }
    return hash % ((BSIZE/4) - 56);
}
```

[FAQ §4.2.1] Names are case-insensitive. The `& 0x7ff` inside the loop is load-bearing, not an optimisation.

### The international variant

`toupper` is the *only* difference between international and non-international volumes, and the reason a wrong INTL determination breaks lookups silently: [FAQ §5.4]

```c
int intl_toupper(int c) {
    return (c >= 'a' && c <= 'z') || (c >= 224 && c <= 254 && c != 247)
         ? c - ('a' - 'A') : c;
}
```

The Amiga character set is ISO 8859-1. Codes 192–222 are uppercase accented, 224–254 their lowercase forms; 215 and 247 are the multiply and divide signs and are excluded. Old AmigaDOS `toupper` mishandled codes above 128, which is the bug international mode exists to fix.

### Collision chains

`ht[hash]` holds the block number of the first entry. Further entries with the same hash form a linked list through each entry block's `hash_chain` field (`BSIZE-16`). Under FFS the chain is sorted by block number. [FAQ §4.2.1, §8]

Traversal must detect cycles (AV-001) — and see the note under **Links** below, because cycles are reachable through legitimate on-disk structures, not only corruption.

## User directory block

As the rootblock, minus the bitmap and volume fields, plus a comment. [FAQ §4.5]

| offset | type | name | meaning |
|---|---|---|---|
| 0x00 | ulong | type | `T_HEADER` = 2 |
| 0x04 | ulong | header_key | self pointer |
| 0x14 | ulong | chksum | normal checksum |
| 0x18 | ulong[] | ht[] | hash table, `BSIZE/4 - 56` entries |
| BSIZE-196 | ushort | UID | |
| BSIZE-194 | ushort | GID | |
| BSIZE-192 | ulong | protect | protection flags (below) |
| BSIZE-184 | char | comm_len | comment length |
| BSIZE-183 | char[79] | comment[] | |
| BSIZE-92 | ulong | days | last access |
| BSIZE-88 | ulong | mins | |
| BSIZE-84 | ulong | ticks | |
| BSIZE-80 | char | name_len | |
| BSIZE-79 | char[30] | dirname[] | |
| BSIZE-40 | ulong | next_link | hardlink chain (FFS) |
| BSIZE-16 | ulong | hash_chain | next entry with the same hash |
| BSIZE-12 | ulong | parent | parent directory |
| BSIZE-8 | ulong | extension | first dircache block (FFS) |
| BSIZE-4 | ulong | sec_type | `ST_USERDIR` = 2 |

### Protection flags

Bits 0–3 are **inverted** — set means *forbidden*: [FAQ §4.4]

| Bit | Meaning when set |
|---|---|
| 0 | delete forbidden (D) |
| 1 | not executable (E) |
| 2 | not writable (W) |
| 3 | not readable (R) |
| 4 | archived (A) |
| 5 | pure / re-entrant (P) |
| 6 | script (S) |
| 7 | hold (H) |
| 8–11 | group D/E/W/R — set means *permitted* |
| 12–15 | other D/E/W/R — set means *permitted* |
| 31 | SUID (MultiUser FS only) |

The owner bits and the group/other bits have opposite polarity. Rendering them uniformly is wrong.

## Files

### File header block

[FAQ §4.4]

| offset | type | name | meaning |
|---|---|---|---|
| 0x00 | ulong | type | `T_HEADER` = 2 |
| 0x04 | ulong | header_key | self pointer |
| 0x08 | ulong | high_seq | number of data-block pointers here |
| 0x0c | ulong | data_size | unused (0) |
| 0x10 | ulong | first_data | first data block pointer |
| 0x14 | ulong | chksum | normal checksum |
| 0x18 | ulong[] | data_blocks[] | `BSIZE/4 - 56` entries, **stored in reverse** |
| BSIZE-196 | ushort | UID | |
| BSIZE-194 | ushort | GID | |
| BSIZE-192 | ulong | protect | as above |
| BSIZE-188 | ulong | byte_size | file size in bytes |
| BSIZE-184 | char | comm_len | |
| BSIZE-183 | char[79] | comment[] | |
| BSIZE-92 | ulong | days | last change |
| BSIZE-88 | ulong | mins | |
| BSIZE-84 | ulong | ticks | |
| BSIZE-80 | char | name_len | |
| BSIZE-79 | char[30] | filename[] | |
| BSIZE-40 | ulong | next_link | hardlink chain |
| BSIZE-16 | ulong | hash_chain | |
| BSIZE-12 | ulong | parent | |
| BSIZE-8 | ulong | extension | first file extension block |
| BSIZE-4 | ulong | sec_type | `ST_FILE` = **-3** |

**`data_blocks[]` runs backwards.** For a 7-block file the first data block is at `data_blocks[71]` and the last at `data_blocks[65]`, with `high_seq == 7`. Iterating forwards reads the file in reverse. [FAQ §4.4]

An empty file is a header alone, `byte_size == 0`, no data-block pointers.

### File extension block

| offset | type | name | meaning |
|---|---|---|---|
| 0x00 | ulong | type | `T_LIST` = 16 |
| 0x04 | ulong | header_key | self pointer |
| 0x08 | ulong | high_seq | pointers stored here |
| 0x14 | ulong | chksum | normal checksum |
| 0x18 | ulong[] | data_blocks[] | as above, reversed |
| BSIZE-12 | ulong | parent | file header block |
| BSIZE-8 | ulong | extension | next extension block, 0 for last |
| BSIZE-4 | ulong | sec_type | `ST_FILE` = -3 |

### Data blocks

**OFS** — 24-byte header, `BSIZE-24` payload (488 at BSIZE 512):

| offset | type | name | meaning |
|---|---|---|---|
| 0x00 | ulong | type | `T_DATA` = 8 |
| 0x04 | ulong | header_key | pointer to the file header |
| 0x08 | ulong | seq_num | data block number, **first is 1** |
| 0x0c | ulong | data_size | `<= BSIZE-24` |
| 0x10 | ulong | next_data | next data block, 0 for last |
| 0x14 | ulong | chksum | normal checksum |
| 0x18 | uchar[] | data[] | payload |

**FFS** — the full `BSIZE` bytes are payload. No header, no checksum, no chain.

This is C-005, and it has a forensic consequence beyond capacity: **OFS offers two independent paths to a file's contents** — the `data_blocks[]` table, and the `first_data`/`next_data` chain — whereas FFS offers only the table. Under FFS, an unreadable header or extension block orphans its data blocks irrecoverably. OFS is materially more salvageable, which matters to F-012. [FAQ §4.4]

### Deletion

Deleting a file clears only its pointer from the directory hash chain and updates the bitmap. Header, extension, and data blocks are left intact, so undelete is straightforward until the blocks are reused. Reuse is not random: allocation resumes from the first free block searched from the rootblock outward, so freed blocks near the rootblock are reclaimed early. [FAQ §4.4, §8] Salvage tooling should therefore treat recency of deletion as a strong recoverability signal (F-012).

## Bitmap

**A set bit means the block is FREE; a cleared bit means allocated.** [FAQ §4.3] This is the opposite of the common convention and worth a test of its own.

| offset | type | count | name |
|---|---|---|---|
| 0x00 | ulong | 1 | checksum (normal algorithm) |
| 0x04 | ulong | BSIZE/4 - 1 | map |

Each long describes 32 blocks, bit 0 being the lowest-numbered. **The map starts at block 2**, not block 0 — the boot blocks are excluded (or `DosEnvVec->Bootblocks` on a hard disk), so `bit_index = block_number - reserved_blocks`. For a DD floppy the map is 1758 bits: 54 full longs plus 30 bits of the 55th. [FAQ §4.3]

`bm_flag` in the rootblock is -1 when valid. [AFFS] warns it "may not be accurate when the system crashes while an affs partition is mounted" — which is AV-003 confirmed by a second source. ADE treats the flag as advisory and can rebuild the bitmap defensively.

If 25 bitmap pointers are insufficient (hard disks above roughly 50 MB), further pointers live in bitmap extension blocks forming a linked list from `bm_ext`:

| offset | type | count | name |
|---|---|---|---|
| 0x00 | ulong | BSIZE/4 - 1 | bitmap block pointers |
| BSIZE-4 | ulong | 1 | next (0 for last) |

Note this block has **no checksum**.

## Directory cache blocks

Chained from the `extension` field of a root or directory block. [FAQ §4.7]

| offset | type | name | meaning |
|---|---|---|---|
| 0x00 | ulong | type | `DIRCACHE` = 33 (0x21) |
| 0x04 | ulong | header_key | self pointer |
| 0x08 | ulong | parent | parent directory |
| 0x0c | ulong | records_nb | records in this block |
| 0x10 | ulong | next_dirc | next dircache block |
| 0x14 | ulong | chksum | normal checksum |
| 0x18 | uchar[] | records[] | `BSIZE-24` bytes of records |

Record layout, 26–77 bytes, **word-aligned with an optional trailing pad byte**:

| offset | type | name |
|---|---|---|
| 0 | ulong | header — entry block pointer |
| 4 | ulong | size (0 for directory or link) |
| 8 | ulong | protect |
| 12 | ushort | UID |
| 14 | ushort | GID |
| 16 | short | days |
| 18 | short | mins |
| 20 | short | ticks |
| 22 | char | type — secondary type |
| 23 | char | name_len (1–30) |
| 24 | char[] | name |
| 24+nl | char | comm_len (0–22) |
| 25+nl | char[] | comment |
| 25+nl+cl | char | optional padding |

The dircache is a *cache*: it duplicates information held in the entry blocks. ADE should treat disagreement between the two as a health finding (F-010), not silently prefer one.

### Confirmed against real disks

*Verified 2026-08-24 across the corpus's 21 `DOS\5` images — 1252 records in 133 cache blocks over 66 cached directories.*

The layout above reads correctly with no amendment needed, including the awkward parts: records are variable-length, the trailing pad byte appears only when the record length is odd, and `days`/`mins`/`ticks` really are 16-bit here against the entry block's 32-bit fields. Every record matched the entry block it describes on name, size, protection, comment and secondary type — **zero disagreements across all 21 disks**. A byte-level error anywhere in the record walk would have desynchronised the rest of the block and shown up immediately, so this is a strong check on the layout as documented.

Two details worth stating because they are easy to get wrong:

- The **secondary type is one signed byte** here where the entry block holds a 32-bit word. `-3` becomes `0xFFFFFFFD` on widening; comparing the two without sign-extending reports every file on the disk as a mismatch.
- A record **never spans two blocks**. There is no continuation mechanism, so a block simply ends early when the next record will not fit.

### A cache block is a block something must reach

Cache blocks are marked used in the bitmap. A reader that does not follow the chain therefore does not merely miss a feature — it reports the blocks as orphaned, and calls space lost that is not lost. This was live in ADE until 2026-08-24: 19 false orphans on the Workbench 3.1 install disk, 14 on Subwar 2050.

Note also that **`DOS\4` occurs nowhere in the corpus** and `DOS\6`/`DOS\7` do not either (§Open questions). Only `DOS\5` has real material, so OFS-with-dircache rests on generated fixtures and the oracle — `unadf -c` lists from the cache rather than the hash chains, which is what makes it an independent check on a cache ADE wrote.

## Links

Hard links are FFS-only and chain through `next_link`. Soft links were broken and support was removed in AmigaDOS 3.0. [FAQ §4.6]

> "Hard links are seen as files, and hard links to directories are allowed, which opens the way to endless recursion..." — [FAQ §4.6]

**This extends AV-001.** Directory traversal can cycle on a structurally valid, non-corrupt disk, because AmigaDOS permits hard links to directories. Cycle detection is therefore a correctness requirement for ordinary images, not merely a defence against hostile input — a visited-set over block numbers, not a depth limit.

### Link block layout

A link block holds **no data of its own**. It is shaped like a file header, but the fields that would describe content are unused and `real_entry` names the block it stands for.

| offset | field | meaning |
|---|---|---|
| BSIZE-44 | real_entry | the file or directory this link points at |
| BSIZE-40 | next_link | on a target, the newest link pointing at it; on a link, the next in that chain |
| BSIZE-4 | sec_type | `ST_LINKFILE` (-4) or `ST_LINKDIR` (4) |

Reading a link's own data-block table therefore yields nothing — the mistake ADE made until BUG-005. A reader must follow `real_entry`, bounds-checking it like any other pointer off the disk (AV-004) and carrying a visited set, since a looping `real_entry` would otherwise not terminate.

### The oracle cannot check links

`unadf` **omits link entries from its listings entirely**: given a generated volume with four entries of which two are links, it reports two. The link blocks match the layout above field for field, so this is a limitation of ADFlib — the FAQ calls the whole link implementation "a mess" — not a fault in the fixtures.

Link support is therefore validated against this specification alone. There is no independent implementation checking it and no link anywhere in the 4652-image corpus, which makes it the one area of Phase 2 where D-010's two mechanisms both come up empty.

## Hard disks — RDB

The Rigid Disk Block must lie within the first 16 blocks of the media. It is 256 bytes regardless of `BSIZE`. [FAQ §6.1]

| offset | type | name | meaning |
|---|---|---|---|
| 0x00 | char[4] | id | `'RDSK'` |
| 0x04 | ulong | size | in longs, == 64 |
| 0x08 | long | checksum | normal algorithm |
| 0x0c | ulong | hostID | SCSI target (7 for IDE/ZIP) |
| 0x10 | ulong | block size | typically 512, any power of 2 |
| 0x14 | ulong | flags | |
| 0x18 | ulong | BadBlockList | block pointer, -1 = none |
| 0x1c | ulong | PartitionList | block pointer, -1 = none |
| 0x20 | ulong | FileSysHdrList | block pointer, -1 = none |
| 0x24 | ulong | DriveInit | optional init code |
| 0x40 | ulong | cylinders | |
| 0x44 | ulong | sectors | per track |
| 0x48 | ulong | heads | |
| 0x80 | ulong | RDB_BlockLo | reserved range low |
| 0x84 | ulong | RDB_BlockHi | reserved range high |
| 0x88 | ulong | LoCylinder | partitionable area low |
| 0x8c | ulong | HiCylinder | partitionable area high |
| 0x90 | ulong | CylBlocks | == heads × sectors |
| 0x98 | ulong | HighRSDKBlock | highest block used by RDSK |
| 0xa0 | char[8] | DiskVendor | |
| 0xa8 | char[16] | DiskProduct | |
| 0xb8 | char[4] | DiskRevision | |

Note the list terminator is **-1, not 0** — the opposite of the filesystem's linked lists.

### Partition block

256 bytes, chained from `PartitionList`. [FAQ §6.3]

| offset | type | name | meaning |
|---|---|---|---|
| 0x00 | char[4] | id | `'PART'` |
| 0x04 | ulong | size | in longs, == 64 |
| 0x08 | ulong | checksum | normal algorithm |
| 0x0c | ulong | hostID | |
| 0x10 | ulong | next | next partition block |
| 0x14 | ulong | Flags | bit 0 bootable, bit 1 no automount |
| 0x24 | char | DriveName len | |
| 0x25 | char[31] | DriveName | e.g. `DH0` |
| | | *DOSEnvVec* | |
| 0x80 | ulong | size of vector | == 16 longs, minimum 11 |
| 0x84 | ulong | SizeBlock | block size **in longs** (128 for BSIZE 512) |
| 0x8c | ulong | Surfaces | heads |
| 0x90 | ulong | sectors/block | == 1 |
| 0x94 | ulong | blocks/track | |
| 0x98 | ulong | Reserved | DOS reserved blocks at partition start, usually 2 |
| 0x9c | ulong | PreAlloc | reserved at end, normally 0 |
| 0xa4 | ulong | LowCyl | first cylinder, inclusive |
| 0xa8 | ulong | HighCyl | last cylinder, inclusive |
| 0xb4 | ulong | Mask | often 0xFFFFFFFE |
| 0xbc | ulong | BootPri | |
| 0xc0 | char[4] | DosType | `DOS` + flags; also `UNI\0`, `UNI\1`, `UNI\2`, `resv` |
| 0xcc | ulong | Bootblocks | boot blocks to load (KS 2.0+) |

`SizeBlock` is in **longs**, not bytes — multiply by 4. `LowCyl`/`HighCyl` are inclusive.

> "The first two blocks of a partition contain a Bootblock. You have to use it to determine the correct file system... **Don't rely only on the PART and FSHD 'DosType' field.**" — [FAQ §6.3]

That is a direct instruction: mount from the partition's own bootblock, cross-check against the RDB's claim, and report disagreement as a health finding.

Non-AmigaDOS dostypes (`UNI\*`, `resv`) must be recognised and skipped rather than mounted.

### Filesystem header block

256 bytes, chained from `FileSysHdrList`, id `'FSHD'`, with `DosType` at 0x20 and `Version` at 0x24 (e.g. 0x0027001b == 39.27). Carries the filesystem driver itself in `LSEG` blocks. ADE parses these for reporting; it does not execute them. [FAQ §6.4, §6.5]

### Bad block block

Id `'BADB'`, chained from `BadBlockList`, holding block-remap pairs. [FAQ §6.2]

### The oracle is stricter than the format here

*Observed 2026-08-24, building the RDB fixture generator.*

FAQ §6.5 says the `LSEG` chain "isn't needed to reach partitions", and it is not: the partition list is reachable from `PartitionList` alone, and ADE mounts a device that has no `FileSysHdrList` at all. **ADFlib will not.** A device whose `FileSysHdrList` is `-1`, or whose `FSHD` names no valid `LSEG`, is rejected with a repeated `ReadLSEGblock : LSEG id not found` and no volume is reported.

This is a divergence in *strictness*, not in reading — ADFlib demands a driver it never runs. It matters twice over:

- A fixture built to the specification alone cannot be checked against the oracle, so the generator emits a minimal `FSHD` + `LSEG` pair. `Device::without_filesystem_driver()` builds the spec-legal-but-ADFlib-unmountable shape deliberately, so the difference stays visible rather than being quietly designed around.
- A real device written by a tool that omits the driver would read in ADE and not in ADFlib. Which behaviour is *right* is not ours to settle; per D-012 the disagreement is recorded, not resolved in the oracle's favour.

A second, narrower divergence: on a partition whose bitmap spans more than one block, ADFlib reports a fill percentage that direct byte inspection contradicts (94% for a partition with five blocks in use). Listing and extraction are unaffected, so this appears to be a display artefact in ADFlib rather than a disagreement about the disk. Recorded because it will otherwise be rediscovered as a suspected ADE bug.

## Hardfiles (HDF)

A hardfile is a raw volume dump with no RDB — bootblock, rootblock, bitmap, exactly like a floppy but larger. Its first three bytes are therefore `'DOS'`. Typical UAE geometry is `heads = 1, sectors = 32`, cylinders derived from size. [FAQ §7]

So **HDF covers two distinct layouts**: RDB-partitioned whole-disk images, and unpartitioned single-volume dumps. The container layer must distinguish them by looking for `RDSK` within the first 16 blocks and falling back to a bootblock at block 0.

### A raw volume has no geometry

An unpartitioned hardfile records no cylinders, heads or sectors anywhere. Those are a convention of whatever created it, not a property of the bytes, and nothing above the block layer depends on them: what a reader needs is the **block count**, because that is what fixes the rootblock's position (C-007).

ADFlib takes exactly this view, reporting an 8 MB hardfile as "Cylinders = 16384, Heads = 1, Sectors = 1" — a shape invented to reach the right total. ADE does the same, and the two agree on the resulting volume.

Large volumes also need **more than one bitmap block**: a 512-byte bitmap block covers `(512/4 - 1) × 32 = 4064` blocks, so an 8 MB hardfile needs five, their pointers filling the rootblock's `bm_pages` and, past 25, a `bm_ext` chain.

## Containers & compression

| Format | Magic | Offset | Verified |
|---|---|---|---|
| ADF | **none** | — | [FAQ §3] |
| Extended-ADF | `UAE-1ADF` | 0 | [CORPUS] |
| HDF (unpartitioned) | `DOS` | 0 | [FAQ §7] |
| HDF (RDB) | `RDSK` | within first 16 blocks | [FAQ §6.1] |
| ADZ / HDZ | `1F 8B` (gzip) | 0 | — |
| SCP | `SCP` + version byte | 0 | [SCP] |
| DMS | `DMS!` *(unverified)* | 0 | Phase 3 |
| IPF | `CAPS` *(unverified)* | 0 | Phase 4 |

### The sniffing problem (F-003)

F-003 commits to dispatch "by content sniffing, not extension". **A plain ADF has no magic number** — it is raw block data beginning at the bootblock. Neither does an unpartitioned HDF, beyond the `DOS` prefix it shares with every ADF.

Content sniffing must therefore be a cascade, not a lookup:

1. Test the unambiguous magics: gzip, `SCP`, `DMS!`, `CAPS`.
2. Test `RDSK` within the first 16 blocks → RDB-partitioned image.
3. Test for `DOS`/`PFS` at offset 0 → a raw volume. Distinguish ADF from HDF by size against known floppy geometries (901,120 / 1,802,240), treating anything else as a hardfile.
4. Validate the bootblock checksum and rootblock plausibility as corroboration, **not** as an accept/reject test — measured against 4288 images, only 74% of `DOS` images have a valid bootblock checksum, 19% of them have no rootblock at all, and some non-`DOS` images mount perfectly. See §Corpus observations and C-008.

Step 3 also cannot require a `DOS` prefix: 7% of the survey corpus begins with something else — 144 distinct custom bootloaders — and ten of those still hold mountable volumes. The prefix is evidence, not a gate.

Size alone cannot be decisive either: a truncated or padded ADF is exactly the sort of thing ADE must diagnose rather than misclassify. The open path should report *what it decided and why*, so a misidentification is visible in the health report (F-010).

## Extended-ADF (`UAE-1ADF`)

Carries raw MFM track data for non-standard and protected disks, which plain ADF cannot. Layout below was **derived empirically** from eleven images in the survey corpus and then checked arithmetically: for all eleven, `12 + tracks × 12 + Σ space` equals the file size exactly.

| offset | type | count | name | meaning |
|---|---|---|---|---|
| 0x00 | char | 8 | magic | `UAE-1ADF` |
| 0x08 | ushort | 1 | reserved | 0 in every observed image |
| 0x0a | ushort | 1 | tracks | number of track entries following |

Then `tracks` track headers of 12 bytes each, from 0x0c:

| offset | type | name | meaning |
|---|---|---|---|
| +0 | ushort | reserved | 0 |
| +2 | ushort | type | **0** = standard AmigaDOS sector data, **1** = raw MFM |
| +4 | ulong | space | bytes allocated for this track in the file |
| +8 | ulong | length | track length in **bits** |

Track data follows the header array in track order, each occupying its `space` bytes.

`length` is in bits, not bytes: a type-0 track reads `space = 5632` (11 × 512, one DD track) with `length = 45056`, exactly 5632 × 8. Type-1 tracks in the same image read `space = 12768` with `length ≈ 102138` bits, which is under `space × 8` — so `space` is the allocation and `length` the meaningful extent.

Mixed track types within one image are normal and are the signature of copy protection: *Deep Space* carries track 0 as type 0 (a standard, mountable bootblock track) and the remaining 165 as raw MFM. A reader that assumes a uniform track type across the image will mis-parse the common case.

## Flux formats

- **Extended-ADF** — see above.
- **SCP** — the open, documented flux container and ADE's write target (D-007). Header magic `SCP` at 0x00 with a version byte at 0x03; track data headers carry `TRK`. [SCP]
- **IPF** — stores flux-transition timings. Reading requires the closed CAPS library; creation is SPS-only. Read-only, optional, licence-gated; ADE **cannot emit IPF** (C-003).

MFM encoding, sync words (0x4489), and track/sector framing are Phase 4 and not yet written up — [RKRM] Appendix C is the source to work from. [FAQ §2] also covers it and should be revisited then.

## Corpus observations

Everything in this section is **measurement, not specification**: a survey of 4288 TOSEC Amiga ADF images on 2026-08-22. It is recorded here because the gap between the documented format and real images is precisely what D-002 gave up when it declined to inherit ADFlib's accumulated knowledge, and measuring it back is how that knowledge is recovered.

### Leading magic

| Count | Leading bytes |
|---|---|
| 3794 | `DOS\0` |
| 300 | *no recognised magic* |
| 139 | `DOS\1` |
| 20 | `DOS\3` |
| 20 | `DOS\5` |
| 11 | `UAE-1ADF` (extended-ADF) |
| 3 | `DOS` + `0x32` |
| 1 | `DOS\2` |

Absent entirely: `DOS\4`, `DOS\6`, `DOS\7`. Their absence from one corpus is not evidence they do not matter — `DOS\5` appears twenty times and is the case that exposed BUG-001 — but it does mean this corpus cannot validate LNFS handling. Fixtures for those need sourcing separately.

The three `DOS` + `0x32` images carry a flags byte outside the documented three bits (`0x32` = `0b0011_0010`). ADE decodes what it can — bit 1 set, so international — and reports `0x30` as unrecognised rather than discarding it.

### 7% of images have no `DOS` bootblock

The 300 unrecognised images span **144 distinct leading words**, i.e. custom bootloaders rather than any one alternative format:

| Count | Leading bytes | |
|---|---|---|
| 97 | `00000000` | zeroed first block |
| 10 | `ATN!` | |
| 7 | `NDOS` | |
| 6 | `RNC\x01` | RNC ProPack compressed |
| 5 | `FORM` | IFF |
| 4 | `trak` | |
| 3 | `LDOS` | |

A sniffing cascade that required a `DOS` prefix would reject one image in fourteen.

### The bootblock is a poor witness in both directions

| Measurement | Result |
|---|---|
| `DOS` images with a **valid bootblock checksum** | 2947 / 3976 — **74.1%** |
| `DOS` images with a **valid rootblock** at block 880 | 3226 / 3976 — 81% |
| **non-`DOS`** images with a valid rootblock at 880 | **10 / 300** |

Three findings follow, all of which bear on F-003 and F-010:

1. **A bootblock checksum cannot gate acceptance.** A quarter of perfectly ordinary images fail it, because only bootable disks need a valid one. It is a signal for the health report, never an accept/reject test.
2. **A `DOS` magic does not imply a mountable filesystem.** 750 images carry one yet have no rootblock at 880 — and this is not an artefact of a strict test: 658 have neither the right primary nor secondary type there, 28 have a zeroed block, and only **3** fail on checksum alone. These are custom-format disks wearing a partial AmigaDOS bootblock.
3. **A non-`DOS` magic does not imply an unmountable one.** Ten of the 300 mount cleanly, with volume names intact — several bearing cracker-group signatures such as `CHAMBER OF SHAOLIN - QUARTEX` and `ACCESSION!`.

Bootblock and filesystem must therefore be probed **independently**, and the result reported as two facts rather than collapsed into one verdict.

### Plain ADFs are not a fixed size

| Count | Size | Interpretation |
|---|---|---|
| 4270 | 901,120 | canonical, 80 cylinders |
| 3 | 923,648 | 82 cylinders |
| 1 | 912,384 | 81 cylinders |
| 1 | 934,912 | 83 cylinders |
| 1 | 901,121 | canonical **+ 1 byte** |
| 1 | 90,112 | 8 cylinders — truncated |

Extra-cylinder images are exact: 81, 82 and 83 cylinders land precisely on `cylinders × 2 × 11 × 512`. Drives could usually seek a few tracks past 80, and both protection schemes and ordinary "extra capacity" formats used them. Ten of the eleven extended-ADFs likewise declare 166 tracks — 83 cylinders.

So the ADF size test is **`size % 11264 == 0` with a plausible cylinder count**, not `size == 901120`. The 901,121-byte image is a byte of trailing junk on an otherwise canonical image; the 90,112-byte one is a truncation. Both should be diagnosed and reported, not silently accepted or rejected — a truncated image is exactly the condition a forensic tool exists to name.

### No corpus image carries an RDB

*Measured 2026-08-24 across 4652 images.*

The `RDSK` signature appears in the first 16 blocks of **0** of them. Every image held is a floppy, which is what makes the RDB search cheap — it never matches, and it never has to read past block 15 to find that out — and also what makes the RDB path the one area of Phase 2 with **no corpus material at all**. Its only external check is the D-002 oracle over generated devices (`oracle_fixtures.rs`), which is exactly the situation D-010's amendment was written to cover.

The practical consequence: RDB reading is verified against the specification and against ADFlib, not against reality. Where real devices differ from both — as real floppies differ from the FAQ in seven documented ways above — that difference has not been measured and cannot be, until there is a device to measure.

### The reference implementation is not a safety benchmark

Running ADFlib's `unadf` over all 4288 images, each capped at 512 MB and 10 s:

| | |
|---|---|
| extracted normally | 3210 |
| declined to mount | 1063 |
| **crashed (SIGSEGV / SIGABRT)** | **15** |

Uncapped, `Bomb Busters_Disk1.adf` drove it to **29 GB** and the kernel OOM killer took down the whole session. That disk is unremarkable: 901,120 bytes, `DOS\0`, valid bootblock checksum, a volume named `BOMBER` that ADE mounts and lists without complaint in 2.8 MB of memory.

ADE crashed on none of the 4288.

This is the concrete form of what F-001 set as its bar and what D-002 predicted: a fault inside wrapped C is not a `Result` and cannot be caught. It is also why the differential oracle runs under hard resource caps — see D-002 and AV-005.

## Format constraints (C-NNN)

Stable, append-only IDs; referenced from [ARCHITECTURE.md](ARCHITECTURE.md), [DECISIONS.md](DECISIONS.md), and [ATTACK_VECTORS.md](ATTACK_VECTORS.md).

- **C-001 — Endianness.** All on-disk data is 68k **big-endian**; the host is little-endian. Every conversion routes through one byte-order module. (ARCHITECTURE invariant 2.)
- **C-002 — FFS 32-bit limit.** FFS addresses ~4 GB max; TD64 and NSD are mutually incompatible 64-bit extensions — NSD being the official one and TD64 a third-party hack [FAQ §3]. HDF handling must detect which scheme an image uses.
- **C-003 — No IPF creation.** IPF authoring is closed (SPS-only) and the CAPS read library is restrictively licensed. ADE reads IPF (optional) but never writes it. (D-007.)
- **C-004 — DMS is buggy.** Some DMS images will not round-trip; over 200 TOSEC entries are tagged `errdms`. ADE surfaces this honestly rather than producing a silently-bad ADF.
- **C-005 — OFS/FFS payload difference.** OFS data blocks carry 488 usable bytes (24-byte header including a checksum and chain pointer); FFS uses the full 512. The block layer is parameterised on this. The OFS header also provides a second, independent recovery path that FFS lacks.
- **C-006 — Dircache and LNFS imply international hashing.** `DOS\4` and `DOS\5` set the dircache bit and leave the INTL bit clear, but are international; `DOS\6` and `DOS\7` are always international. Deciding the hash function from bit 1 alone breaks directory lookup on those volumes, and breaks it *silently* — as "not found" rather than as an error. Recorded 2026-08-22 from [FAQ §4.1] and [AOS-LNFS].
- **C-007 — The bootblock rootblock pointer is unreliable.** The bootblock's `Rootblock` field at offset 8 "is 880 for DD **and HD**" [FAQ §4.1], while an HD volume's rootblock is actually at 1760. The field must not be trusted; the rootblock location is computed as `(numReserved + highKey) / 2` [FAQ §4.2]. Recorded 2026-08-22.

- **C-008 — ADF identification is heuristic and must be reported as such.** A plain ADF has no magic number, is not a fixed size (81–83-cylinder images occur), need not carry a valid bootblock checksum (26% of surveyed images do not), and a `DOS` prefix neither implies a mountable filesystem (19% of surveyed `DOS` images have no rootblock) nor is required for one (10 surveyed non-`DOS` images mount). Format detection is therefore a cascade of weighted evidence, not a test. ADE must record *what it decided and why* so a misidentification is visible in the health report (F-010) rather than silently wrong. Recorded 2026-08-22 from [CORPUS].

## Open questions

Deliberately unresolved, each deferred to the phase that needs it:

- **DMS and IPF magic bytes** — asserted from memory in the table above, not yet confirmed against a primary source. Phase 3 and Phase 4 respectively.
- **LNFS block layout** — [AOS-LNFS] summarised above; needs a full field-level pass before implementation. Phase 2.
- **MFM track and sector framing** — [RKRM] Appendix C not yet consulted. Phase 4.
- **muFS (MultiUser FS) variants** — [AFFS] says they are supported by the Linux driver; ADE's position is undecided. The `protect` field's bits 8–15 and 31 are muFS-related.
- **5.25" DD geometry** — named in ROADMAP Phase 2; not covered by [FAQ §3], which documents only 3.5" DD and HD.
- **`DOS\6` and `DOS\7` fixtures.** The survey contains neither, so both LNFS variants cannot be validated against real material. `DOS\5` appears twenty-one times and was the case that exposed BUG-001, so the absent ones are not safely assumed unimportant — they need sourcing separately (D-010). **`DOS\4` is resolved rather than sourced**: it is absent from the corpus too, but the generator builds it and `unadf -c` validates it (§Confirmed against real disks), which is the shape D-010's amendment describes.
- **The 750 `DOS`-magic images with no rootblock at 880.** Custom formats wearing an AmigaDOS bootblock, unexamined so far. Worth a pass: some may place a rootblock elsewhere, and the distribution of what they *do* contain would sharpen the F-003 cascade.
- **Non-`DOS` prefixes** — `PFS`, `SFS`, `KICK`. Detection and honest reporting are in scope; mounting is not, for v1.
