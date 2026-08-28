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
| **[MDFS]** | *Amiga disk format*, mdfs.net — <https://mdfs.net/Docs/Comp/Disk/Format/Amiga>. MFM sector layout. |
| **[SCP]** | Jim Drew, *SuperCard Pro Image File Specification v2.5* — <https://www.cbmstuff.com/downloads/scp/scp_image_specs.txt> |
| **[AMIGAWIKI]** | *Filesystem*, amiga-wiki — <https://www.amigawiki.org/doku.php?id=en:system:filesystem>. Dostype table. |
| **[DISKTYPE]** | `disktype`, `amiga.c` — the filesystem-identifier table of a working detector, consulted as a second opinion on the registry. Read as data, never as an implementation to copy (D-002 applies to it as it does to ADFlib). |
| **[HYPERION-FS]** | *list of all FS indentificators*, Hyperion Entertainment forums — <https://forum.hyperion-entertainment.com/viewtopic.php?t=2343>. Community-assembled; treated as a lead, not as ground truth. |
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
| `DOS\6` | OFS-LNFS | OFS | yes | no | unsupported |
| `DOS\7` | FFS-LNFS | FFS | yes | no | unsupported |

Support column from [AFFS], which documents only `DOS\0`–`DOS\5`.

**Bold INTL means international hashing applies while the stored bit is clear** — true of `DOS\4` and `DOS\5` only. `DOS\6` and `DOS\7` are international *and* carry the bit (6 is `0b110`, 7 is `0b111`), so reading the bit alone gets the hash right on LNFS by luck and wrong on dircache by design. Both are still misclassified by a bit-pattern decoder, which is the separate trap below.

### The wider dostype registry

*Surveyed 2026-08-24 across [AMIGAWIKI], [DISKTYPE], [HYPERION-FS] and [AFFS]; cross-checked against the corpus.*

Two conclusions, and the first closes a question rather than opening one.

**`DOS\0`–`DOS\7` is the complete AmigaDOS set.** No source consulted knows a `DOS\8` or `DOS\9`. The eight above are all of them, so ADE's enumeration is complete and a ninth need not be designed for. One loose end: [HYPERION-FS] lists an `ID_VP255_DOS_DISK` labelled "FFS9", and no source found gives its byte value or says what it is. It is recorded here as a lead, not as a fact — if a `DOS\`-prefixed value outside 0–7 ever turns up, this is the thread to pull.

**AmigaDOS is one family among many, and the rest must be identified and declined.** A dostype is a 4-byte tag, and the AmigaDOS test is `dostype & 0xFFFFFF00 == 0x444F5300` — everything below fails it and none of it is ADE's to mount:

| Family | Values | Filesystem |
|---|---|---|
| muFS | `muFS`, `muF\0`–`muF\5` | MultiUser FFS — the OFS/FFS matrix again, with ownership |
| PFS | `PFS\0`–`PFS\3`, `PDS\2`, `PDS\3`, `muPF` (`0x6D755046`) | Professional File System; `PDS` is the direct-SCSI build |
| SFS | `SFS\0`, `SFS\2`, `SFS\3` | Smart File System. `SFS\1` was a beta and is **format-incompatible** with `SFS\2` |
| AFS | `AFS\0`, `AFS\1`, `AFS\2`, `muAF` (`0x6D754146`) | Ami-File-Safe; `AFS\1` Pro, `AFS\2` User |
| Other Amiga | `CFS\0`, `JXF\4`, `BOX\0` | CFS, JXFS, BoxFS |
| AmigaOS 4 era | `NTFS`, `FAT2`, `FATX`, `EXT\2`, `HFS\0` | host filesystems reached through OS4 handlers |
| Unix | `UNI\0`–`UNI\2` (Amix), `NBR\7`, `NBS\1`, `NBU\7` (NetBSD), `LNX\0`, `MNX\0` | |
| Swap | `SWAP`, `SWP\0` | not a filesystem at all |
| Foreign | `MAC\0`, `MSD\0`, `MSH\0` | HFS, MS-DOS |
| Optical | `CD00`, `CD01`, `CDDA`, `CDFS` | |
| Bootblock only | `KICK`, `BOOU` | a Kickstart disk and a generic boot disk, not filesystems |

`resv` also appears in RDB partition tables, marking space reserved rather than formatted.

**None of this is in the corpus.** Of 4652 images, 4341 begin `DOS` and **not one** carries any other filesystem identifier — the remaining 311 are custom bootblocks (`RNC` copylock loaders, `ATN!`, `NDOS`, and a long tail of one-offs) plus 11 extended ADFs and 100 all-zero blocks. So foreign-dostype handling has no corpus material either. It matters chiefly on RDB devices, where a partition may legitimately carry any of the above and mounting it as AmigaDOS would be actively wrong; `Partition::claims_amigados` is the guard, and SPEC §Partition block already says so.

**Where a bit-pattern decoder goes wrong is not here.** These are distinct 4-byte tags, easy to tell apart. The trap is *within* the `DOS\` family, where the last byte is flags-shaped but is not purely flags — which is the next two sections.

### The dircache trap (C-006)

> "If the dircache is enabled, its flag is set (bit #2), and the international mode is also enabled, but the related flag (bit #1) will stay cleared." — [FAQ §4.1]

So `DOS\4` and `DOS\5` carry the dircache bit with the INTL bit **clear**, yet international hashing applies. Reading bit 1 alone to decide the hash function produces lookups that fail on exactly those disks — and fail *quietly*, as a "file not found" rather than an error. [AOS-LNFS] states LNFS likewise "always uses the international directory entry name hashing operation", so `DOS\6` and `DOS\7` are international too.

**International hashing applies when bit 1 OR bit 2 is set, or when the dostype is `DOS\6`/`DOS\7`.**

### LNFS (`DOS\6`, `DOS\7`)

A later extension from Olaf Barthel's FFS reimplementation (AmigaOS 4 era), raising the 30-character name limit to ~106. Structurally: the separate name and comment fields are replaced by a 112-byte **NaC** ("name and comment") array holding both as consecutive BCPL strings; if they do not fit, the comment moves to a dedicated `TYPE_COMMENT` block referenced by a new `CommentBlock` field. The rootblock gains `NumBlocksUsed` and `FileSystemType`. There is no DCFS variant of LNFS. [AOS-LNFS]

#### Field-level pass

*Done 2026-08-24 against [AOS-LNFS], discharging the open question of the same name.*

[AOS-LNFS] declares its structures in C rather than as an offset table. The offsets below are the declaration order laid out, which is safe here because the structures contain no padding — every member is naturally aligned — and because **both sum to exactly 512 bytes**, which they could not do if a field were missing or misread. Every field from 0x1F0 onward matches the classic layout exactly, which is the second check.

Entry block — the long-name form of the canonical file/directory header:

| offset | type | name | meaning |
|---|---|---|---|
| 0x000 | ulong | Type | `T_HEADER` = 2 |
| 0x004 | ulong | OwnKey | self pointer |
| 0x008 | ulong[3] | Spare1 | must be 0 (a **file** uses these for `HighSeq`, `DataSize`, `FirstData`) |
| 0x014 | ulong | Checksum | normal algorithm |
| 0x018 | ulong[72] | HashTable | hash table, or the data-block table on a file |
| 0x138 | ulong | Spare2 | must be 0 |
| 0x13c | uword | OwnerID | **same as classic** |
| 0x13e | uword | GroupID | **same as classic** |
| 0x140 | ulong | Protection | **same as classic** |
| 0x144 | ulong | Spare3 | must be 0 (a **file** uses this for `ByteSize`, as classic does) |
| 0x148 | char[112] | **NaC** | name and comment, as two consecutive BCPL strings |
| 0x1b8 | ulong | CommentBlock | `TYPE_COMMENT` block, when the comment does not fit |
| 0x1bc | ulong[2] | Spare4 | must be 0 |
| 0x1c4 | ulong[3] | Created | datestamp — **moved** from the classic 0x1a4 |
| 0x1d0 | ulong[2] | Spare5 | must be 0 |
| 0x1d8 | ulong | FirstLink | same offset as the classic `real_entry` |
| 0x1dc | ulong[5] | Spare6 | must be 0 |
| 0x1f0 | ulong | HashChain | **same as classic** |
| 0x1f4 | ulong | Parent | **same as classic** |
| 0x1f8 | ulong | DirList | **same as classic** (the `extension` field) |
| 0x1fc | long | SecondaryType | **same as classic** |

The `NaC` array holds the name then the comment, each a BCPL string — one length byte followed by that many characters, no terminator. `[6]barney[4]fred` is a complete pair; a nameless comment is `[5]wilma[0]`. 112 bytes covers both, so the practical name limit is about 106 characters. When the pair will not fit, the comment moves to a `TYPE_COMMENT` block and `CommentBlock` names it:

| offset | type | name | meaning |
|---|---|---|---|
| 0x00 | long | Type | `TYPE_COMMENT` = **64** |
| 0x04 | ulong | OwnKey | self pointer |
| 0x08 | ulong | HeaderKey | the entry block this comment belongs to |
| 0x0c | ulong[2] | Spare1 | must be 0 |
| 0x14 | long | Checksum | normal algorithm |
| 0x18 | char[80] | Comment | BCPL string, 80 bytes including the length byte |
| 0x68 | ulong[102] | Spare2 | must be 0 |

Root block — identical to the classic one but for two fields:

| offset | type | name | meaning |
|---|---|---|---|
| 0x1d4 | ulong | NumBlocksUsed | blocks allocated; **valid only when `BitmapFlag` is -1** |
| 0x1f0 | ulong | FileSystemType | the dostype signature again, or 0 for a non-LNFS filesystem |

Everything else — the hash table, `BitmapFlag` at 0x138, the 25 bitmap keys, `BitmapExtend`, all three datestamps, the 30-character volume name — sits exactly where the classic rootblock puts it. The root directory's name is still capped at 30 characters even on LNFS.

**Hashing does not change.** [AOS-LNFS] is explicit: "No change is made to the hashing algorithm. It is the same that is being used for international mode, only that the names of the files can be longer than 30 characters." So C-006 covers LNFS, and the only difference is how many characters go into the hash.

#### Why the two layouts are dangerous together

The classic and LNFS entry blocks share a primary type, a secondary type, a checksum algorithm and their whole tail. Nothing in the block itself says which layout it is — **only the volume's dostype does**. Read one as the other and:

- The classic **name** field at 0x1b0 lands inside LNFS's `Spare6`, so a classic reader finds an empty name on every LNFS entry.
- The classic **datestamp** at 0x1a4–0x1b0 lands inside the `NaC` array, so a classic reader parses name characters as a date.
- An LNFS reader pointed at a classic block reads the comment field as the name.

Both blocks checksum correctly either way, because the checksum covers the bytes and not their meaning. This is the same class of trap as C-006 and BUG-001, and it is why the dostype must decide the layout before any field is read.

#### The oracle cannot check LNFS — it does not implement it

*Measured 2026-08-24.*

ADFlib decodes the dostype **by bit pattern**, which is exactly the trap C-006 and BUG-001 describe. `DOS\6` is `0b110` and `DOS\7` is `0b111`, so it reports them as `OFS INTL DIRCACHE` and `FFS INTL DIRCACHE` — as classic dircache volumes, which they are not. There is no LNFS mode in it at all.

It acts on that misreading. Asked to list a `DOS\7` volume from its dircache, `unadf -c` looks for cache blocks that LNFS never had:

```
Warning <adfReadDirCBlock : invalid checksum>
Warning <adfReadDirCBlock : T_DIRC not found>
Warning <adfReadDirCBlock : headerKey!=nSect>
Volume : Floppy 880 KBytes, "LongNames" ... FFS INTL DIRCACHE . Filled at 0.4%.

Using dir cache blocks.
```

— and then prints **nothing, exiting 0**. An empty listing reported as success is the worst shape a failure can take in a preservation tool: a caller scripting against the exit code concludes the disk is empty.

Two consequences follow, and the first is the harder one:

- **LNFS has no oracle.** It is absent from the corpus (§Open questions) and unimplemented in ADFlib, so it is the first structure in ADE with *neither* external check. D-010's amendment lets a generated fixture stand in for a real disk because ADFlib validates it independently; here there is nothing to do the validating. Any LNFS reader would be checked against SPEC and against itself, which is the situation D-002 gave up ADFlib's accumulated knowledge to avoid.
- **ADE must not repeat the mistake.** It does not: `Dostype::mode()` matches `6 | 7 => LongNames` *before* testing the dircache bit, so `has_dircache()` is false on an LNFS volume and no cache is looked for. That ordering is BUG-001's fix, and `dostype_lnfs.rs` pins it, because the natural way to write that function is the way ADFlib wrote it.

#### The file header is inferred, not documented

[AOS-LNFS] declares `LongNameUserDirectoryBlock`, the `CommentBlock` and the root block. **It declares no long-name file header.** The table above marks the three file-only fields (`HighSeq`/`DataSize`/`FirstData` at 0x008, `ByteSize` at 0x144) in their classic positions on the reasoning that AmigaDOS has always used one canonical 512-byte shape for both files and directories — they differ only in which overlapping fields are live — and that Hyperion describes the change as "changing the layout of the canonical file/directory header block", singular.

That reasoning is sound but it is **not a citation**. Anything built on it is built on inference, and this paragraph exists so that a future failure is diagnosed in one step rather than rediscovered.

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
| ADZ / HDZ | `1F 8B` (gzip) | 0 | [RFC 1952], implemented |
| SCP | `SCP` + version byte | 0 | [SCP] |
| DMS | `DMS!` *(unverified)* | 0 | Phase 3 |
| IPF | `CAPS` *(unverified)* | 0 | Phase 4 |
| FDI | `FDI` *(unverified)* | 0 | not scheduled — see below |

### ADZ and HDZ (gzip)

An ADZ is a gzip-wrapped ADF and an HDZ a gzip-wrapped HDF. There is no Amiga-specific structure at all: the wrapper is ordinary gzip ([RFC 1952]) around ordinary DEFLATE ([RFC 1951]), and what comes out is the image.

| offset | field | meaning |
|---|---|---|
| 0x00 | ID1, ID2 | `1F 8B` |
| 0x02 | CM | compression method; **8** is DEFLATE and the only one gzip defines |
| 0x03 | FLG | bit 1 FHCRC, bit 2 FEXTRA, bit 3 FNAME, bit 4 FCOMMENT |
| 0x04 | MTIME | modification time, little-endian |
| 0x08 | XFL, OS | compression hint and source OS |
| 0x0a | *optional fields* | FEXTRA (length-prefixed), then FNAME and FCOMMENT (NUL-terminated), then FHCRC |
| … | DEFLATE stream | |
| end−8 | CRC32 | of the **uncompressed** data |
| end−4 | ISIZE | uncompressed length, modulo 2³² |

Note gzip's framing is **little-endian**, unlike everything on an Amiga disk. C-001 governs disk data and routes it through `ade-endian`; the gzip header is not disk data and is read directly, which is a deliberate exception rather than an oversight.

**The trailer is a verification, not a size hint.** `ISIZE` states the decompressed length, and reserving it before decompressing would be BUG-003 again with the attacker holding the pen. Both fields are checked *after* the fact: a caller gets bytes that provably match what was compressed, or an error.

### Decompression is the AV-005 surface

DEFLATE is where a few kilobytes of input legitimately becomes gigabytes of output — that is the format working, not a corruption. Three rules follow, and they are the reason the inflater is written the way it is:

- **The output cap is checked before every write, never after.** A limit tested afterwards has already allocated. Measured: a 970 KB stream expanding to 1 GB is refused with a peak RSS of 515 MB against a 512 MiB cap, rather than being decompressed and then complained about.
- **Nothing is sized from a declared length.** Every length in the stream is the input's, including `ISIZE`.
- **A back-reference before the start of the output is an error**, not a wrap or a clamp. This is AV-004's equivalent inside the decompressor, and it is what stops a crafted stream reading memory it should not. Measured on a corrupted real ADZ: `back-reference 27392 bytes back, with only 6569 bytes decompressed`.

The cap is **policy, not format**: `MAX_DECOMPRESSED` is 512 MiB because everything here reads whole images into memory, so an unbounded expansion is an OOM kill rather than a slow read. A legitimate image larger than that would be refused, which is why the error names the limit rather than calling the file corrupt.

### The oracle for gzip is stronger than the one for ADF

D-002's ADF oracle compares two *interpretations* of a disk, so a disagreement needs adjudicating (D-012). gzip has no such gap: the system `gzip` compresses, ADE decompresses, and the result is either byte-identical to the input or wrong. Every image in the corpus is therefore a test case with an unambiguous answer, and `compressed.rs` runs a deterministic sample of them.

That also makes the *absence* of DMS material sting less than it might: ADZ/HDZ is the compressed path that can be verified completely, and it is done. DMS remains blocked on test data (D-009).

[RFC 1951]: https://www.rfc-editor.org/rfc/rfc1951
[RFC 1952]: https://www.rfc-editor.org/rfc/rfc1952

### Conversion is mostly a question about loss

Containers divide into two kinds, and the division is what makes a conversion matrix worth having.

**Sector containers** — ADF, HDF, a whole-device image — are the same thing: a flat run of sectors distinguished by naming convention rather than structure (§A raw volume has no geometry). Converting between them is a byte copy and can lose nothing. ADZ and HDZ are these wrapped in gzip, so unwrapping is also lossless, and provably so.

**Flux and raw-MFM containers** — extended ADF, SCP, IPF — hold what a sector image cannot: track timings, weak bits, and the deliberate irregularities that constitute copy protection. Flattening one into a sector image is not a conversion so much as a discard, and it is silent: the output is a perfectly valid ADF, and nothing about it records that the protection is gone.

That silence is what F-016 addresses. ADE's matrix gives every pair an answer with a reason attached, and separates two things that look alike from outside:

- **Refused** is a decision that does not expire. IPF output is refused because authoring is SPS-only (C-003), and no amount of future work changes that.
- **Not implemented** is a gap with a cause. DMS input waits on test material (D-009), flux writing on Phase 4 (D-005).

Reporting them identically would tell a user to wait for something that is never coming, or to give up on something that is merely pending.

**Lossy conversions are refused rather than warned about.** The loss is not recoverable and a warning nobody reads is precisely how it occurs.

### FDI is the licence-free flux format

*Found 2026-08-24 surveying formats.*

FDI ("Formatted Disk Image", Vincent Joguin, 2000) stores raw low-level track data of the kind copy protection needs, and — unlike IPF — **the specification is public and the access tools are open source**. Most Amiga emulators read it.

That matters because C-003 gates IPF behind a licence and forbids ADE from ever *emitting* it, which leaves Phase 4 able to read the dominant preservation format but not write it. FDI and SCP are the two flux formats ADE could support without a licensing constraint, and SCP (D-007) is already the chosen capture format. FDI is worth considering as a read/write interchange target alongside it; it is not currently on the roadmap, and this note exists so the option is visible when Phase 4 is planned rather than discovered afterwards.

Not verified: the magic bytes above are asserted, not confirmed against a file or the specification. Do that before writing a sniffer arm.

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

Carries raw MFM track data for non-standard and protected disks, which plain ADF cannot. Layout below was **derived empirically** from the eleven extended ADFs in the corpus — there is no published specification — and re-verified by implementation on 2026-08-25.

**Correction (2026-08-25).** This section previously claimed that `12 + tracks × 12 + Σ space` equals the file size for all eleven. It does not: it holds for ten, and `Demolition.adf` is **25,732 bytes short**, ending inside track 163 of a declared 166. The arithmetic is still the right check — it is what found the truncation — but it is a test the file can fail, not an invariant. A reader must treat a short file as a fault and keep the tracks that are present; 163 good tracks are not worth discarding.

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

**`space` is the file allocation; `length` is the meaningful extent, in bits.** Measured across all 428 type-0 tracks in the corpus, `length` is **45056 in every one** — 5632 bytes, one standard DD track of 11 × 512. `space` for those same tracks is 5632, 12650 or 12668 depending on the writer, so a reader that takes `space` as the data size reads up to seven kilobytes of padding as sectors.

That correction matters more than it sounds: the earlier note here said "a type-0 track reads `space = 5632`", which is true of only one writer in three. Advance through the file by `space`; read data by `length`.

**A track may hold nothing.** 154 observed type-1 tracks have `space` and `length` both zero — an unformatted or never-captured track, which is a fact about the disk rather than damage to the file. `Terrorpods` has 69 such tracks and `Deep Space` 55.

Mixed track types within one image are normal and are the signature of copy protection: *Deep Space* carries one type-0 track and 110 raw MFM ones. A reader that assumes a uniform track type across the image will mis-parse the common case.

### The sector tracks really are sectors

*Verified 2026-08-25.* Assembling the type-0 tracks of an extended ADF into a plain ADF, at their track positions with the rest zeroed, produces a mountable volume where enough of the disk is ordinary:

| Image | type-0 tracks | reconstructed volume |
|---|---|---|
| `Demolition` | 160 of 166 | mounts as `Demolition` |
| `Realm of the Trolls` | 80 of 166 | mounts as `REALM OF THE TROLLS` |
| `Champ, The` | 159 of 160 | no volume — the one raw track is where the rootblock would be |
| `Deep Space` | 1 of 166 | no volume, as expected |

This is what confirms the layout rather than merely fitting it: the arithmetic could be satisfied by a wrong reading, but a wrong reading does not produce a rootblock with a legible name at block 880. It also shows what the type mix means in practice — `Champ` is a disk protected by making exactly the track that matters unreadable to a plain reader.

## Flux formats

- **Extended-ADF** — see above.
- **SCP** — the open, documented flux container and ADE's write target (D-007). Header magic `SCP` at 0x00 with a version byte at 0x03; track data headers carry `TRK`. [SCP] Confirmed 2026-08-27: a generated file opens `53 43 50 00`, and ADE's sniffer already identifies it. Read since 2026-08-28 — see §SCP structure and §SCP has material and an oracle.
- **IPF** — stores flux-transition timings. Reading requires the closed CAPS library; creation is SPS-only. Read-only, optional, licence-gated; ADE **cannot emit IPF** (C-003). Independently corroborated 2026-08-27: Greaseweazle, which writes SCP and HFE happily, refuses IPF as an output format.

### SCP structure

*Written from [SCP] on 2026-08-28 and verified field by field against a Greaseweazle-generated capture of `1000cc Turbo.adf`.*

An SCP holds neither sectors nor bits. It holds **intervals between magnetic flux transitions** — one list per revolution, per track. Everything above that is inferred by deciding where the bit cells fall, which is what makes flux a preservation format and a sector image a summary of one.

#### File header, 16 bytes

| Offset | Size | Field | Notes |
|---|---|---|---|
| 0x00 | 3 | `SCP` | the signature |
| 0x03 | 1 | version | `(version << 4) \| revision`; 0x25 is v2.5. **Greaseweazle writes 0x00** |
| 0x04 | 1 | disk type | manufacturer in the upper nibble, subclass below |
| 0x05 | 1 | revolutions | stored per track, 1–5 |
| 0x06 | 1 | start track | first slot used |
| 0x07 | 1 | end track | last slot used |
| 0x08 | 1 | FLAGS | see below |
| 0x09 | 1 | bit-cell width | 0 means the default of 16 bits |
| 0x0A | 1 | heads | 0 = both, 1 = side 0, 2 = side 1 |
| 0x0B | 1 | resolution | multiplier of 25 ns; 0 *is* 25 ns, not zero |
| 0x0C | 4 | checksum | over everything from 0x10 to EOF |

FLAGS, bit 0 upward: index-aligned, 96 TPI, 360 RPM, normalised, read/write, footer present, extended mode, foreign creator. The generated capture reads `0b0010_0011` — index-aligned, 96 TPI, footer present.

#### Track offset table

168 little-endian longwords at **0x10** (at 0x80 when the extended-mode flag is set), each an **absolute file offset** to a track data header, or zero for a track that is absent. The slot index is the *physical* track: cylinder doubled, plus the head.

#### Track data header

`TRK`, then the track number as one byte, then twelve bytes per stored revolution: duration in ticks (index to index), the number of flux values, and the offset to them **relative to the track data header**, not to the file. The generated capture's first track has two revolutions at 8,000,000 ticks each — 200 ms, which is 300 RPM — of 41,975 flux values, the first starting 28 bytes into the header, exactly after two revolution entries.

#### Flux values, and the trap

Each value is a **16-bit big-endian** count of ticks — in a file whose every other field is little-endian.

This is the one thing a reader must not get wrong, and getting it wrong does not fail: the capture's first values are `009e`, which is **158** read big-endian and **40448** read little-endian. 158 ticks is 3,950 ns, one 4 µs MFM interval at 250 kbit/s. 40448 ticks is a millisecond, which is not an interval any drive produces — but nothing would report an error, and the decode would simply find no sectors anywhere.

A value of **zero is not an interval**. It means no transition occurred within the 16-bit range, so 65,536 ticks are accumulated and the next value continues the same interval. Treating zero as a transition manufactures a stream of impossible ones; the long gaps it stands for are how an erased or unformatted region reads.

#### The disk-type byte cannot be used for detection

The specification assigns 0x04 to Commodore Amiga. **Greaseweazle writes 0x80 — "other" — for a disk it has just encoded as AmigaDOS MFM.** A reader dispatching on this byte would refuse a file the standard tool had made. ADE reports it and decides nothing from it, which is the same lesson C-008 taught about the `DOS` prefix: what a container *says* it holds is evidence, never a verdict.

#### What ADE does not parse

The extension footer (creator strings, drive model, timestamps) and the file checksum. The footer is provenance metadata with no bearing on what the disk says; the checksum covers everything from 0x10 to EOF, so verifying it means reading 30 MB to answer a question no caller has asked. Both are recorded rather than silently skipped.

### Flux is not bits: the cell-width problem

A flux image says *when* the magnetisation reversed. Recovering bits means deciding how many bit cells each interval spans, and no field in the file states the cell width — the drive that wrote the disk and the drive that read it did not agree on speed, and neither ran at exactly its nominal rate.

At 250 kbit/s a cell is 2 µs, which is 80 ticks. Legal MFM intervals are two, three or four cells; a transition is a `1` and the cells that pass without one are zeros, so an interval of *n* cells decodes to a `1` followed by *n−1* zeros.

**A fixed divisor is not enough.** Motor speed drifts by a percent or two within a single revolution, and a fixed divisor accumulates that drift until intervals land on the boundary between two and three cells, where one rounding error becomes a wrong bit and every sector after it is lost. So the cell width is tracked: each interval is measured against the current estimate and the estimate is nudged toward what the interval implies, slowly, because a fast loop chases noise and one weak bit would drag it off the data rate entirely.

Two findings from implementing that, both from tests rather than from reasoning:

**The correction must be carried in fixed point.** A drive running 2% slow implies a cell one tick wider than nominal; one sixteenth of one tick is zero in integer arithmetic, so the loop never moves. It does not look broken — the drift stays at zero, which a naive check reads as a *perfect* lock. The estimate is therefore carried at 64× so sub-tick corrections accumulate.

**A lock check must count what it rejected.** Flux at a data rate nothing like MFM produces intervals that are all out of range, so the estimate is never corrected, never drifts, and a drift-only check again pronounces total failure a perfect lock. A loop that never ran is not a loop that succeeded, so "locked" also requires that under 5% of intervals fell outside two-to-four cells.

### Reading SCP, measured

*2026-08-28.*

| check | result |
|---|---|
| Generated fixture → SCP → decoded, byte-for-byte | identical |
| 20 corpus disks → SCP → decoded, listings | 20 of 20 identical |
| 3 corpus disks → SCP → decoded, byte-for-byte | 3 of 3 identical |
| Sectors recovered per disk | **1760 of 1760, every disk** |
| Intervals rejected as out of range | 1 per track — the partial cell after the index pulse |
| `ade info` on a 30 MB capture | 0.4 s |

**Several revolutions are merged, not chosen between.** An SCP normally stores two or more revolutions of each track, and they are not duplicates: a marginal or weak-bit region reads differently each time, which is *why* the format stores several. Each is decoded, and any sound sector still missing is taken from it. That is F-008's merge applied within one file, and it is the concrete reason reading flux beats reading a sector image of the same disk — the sector image already discarded the second opinion.

**What this does not establish.** Everything flux exists for. `gw` encodes an ordinary AmigaDOS disk, so these captures hold no weak bits, no long tracks and no deliberate illegality. "ADE reads a clean capture correctly" is necessary and nowhere near sufficient; only a real protected disk closes that gap, and that needs the hardware F-006 waits on.

### SCP has material and an oracle

*Established 2026-08-27, and it reverses an earlier judgement.*

SCP was recorded as blocked on material, on the grounds that not one `.scp` file existed here. That is no longer true. The **Greaseweazle host tools** (`gw`, installed from the project's own releases — it is not on PyPI) convert a plain sector image into real SCP, so every one of the 4652 corpus ADFs is potential material.

More usefully, `gw` is an **independent implementation**, which makes it an oracle in exactly the sense D-002 and D-010 mean: run as a separate binary, source not read, disagreements adjudicated rather than assumed. Measured:

| check | result |
|---|---|
| ADF → SCP → ADF, five varied disks | **5 of 5 byte-identical** |
| ADF → HFE → ADF | byte-identical |
| IPF as an output format | refused |
| ADE's sniffer on a generated SCP | identified as `SCP flux` |

That is the same footing gzip put ADZ on, and it is what made ADZ shippable.

#### Two caveats that matter

**SCP generation is not deterministic.** Converting one ADF twice produces two different SCP files: flux timings carry jitter, which is the point of the format. So an SCP fixture cannot be committed and compared byte for byte — it must be generated at test time, and the invariant to assert is the **round trip**, not the bytes. This suits D-010 anyway, which commits no binaries.

**`gw` does not understand extended ADF.** Given `Wings of Death_DiskB.adf` — a 2,004,560-byte `UAE-1ADF` container — it read the first 901,120 bytes of the *file* as though they were sectors, header included, and reported "Found 1760 sectors of 1760 (100%)". The output decodes back to exactly those bytes, so the round trip is self-consistent and the input reading is nonsense.

That is worth knowing before trusting it: `gw` is an oracle for **plain sector images only**. Pointed at a raw-track container it will silently agree with itself about the wrong data, and its confidence percentage describes its own encode rather than anything about the disk.

## MFM

*Written 2026-08-25, implementing it. Derived from [FAQ §2] and [MDFS], then **confirmed against the corpus's 1235 raw tracks** — see §The decode is self-evidencing.*

A raw track is what the drive actually read: a continuous **bit** stream carrying data bits interleaved with clock bits, punctuated by sync marks. Decoding it is what turns a protected disk's capture back into data, and — just as usefully — shows where it cannot be turned back, because that is where the protection is.

### Sector layout

One sector is **1088 MFM bytes**. Eleven of them plus a gap is 12668 bytes, which is exactly the per-track allocation observed in the corpus (§Extended-ADF) — a satisfying independent check on both structures.

| offset | MFM bytes | decoded | field |
|---|---|---|---|
| 0x000 | 4 | 2 | two sync words, `0x4489 0x4489` |
| 0x004 | 8 | 4 | info: `0xFF`, track, sector, sectors-to-gap |
| 0x00c | 32 | 16 | sector label — zero in practice |
| 0x02c | 8 | 4 | header checksum |
| 0x034 | 8 | 4 | data checksum |
| 0x03c | 1024 | 512 | data |

Offsets are from the start of the sync words. The header checksum covers the 40 MFM bytes of info and label; the data checksum covers the 1024 MFM bytes of data. Both are the **XOR of the big-endian longs, masked with `0x55555555`** — computed over the *encoded* bytes, with clock bits excluded by the mask rather than by decoding first.

### The odd/even split

Each MFM byte carries four data bits in its even positions. A field of *n* decoded bytes occupies *2n* MFM bytes: **the odd half first, then the even half**.

```
decoded[i] = ((odd[i] & 0x55) << 1) | (even[i] & 0x55)
```

**Which half comes first is not agreed between sources.** [MDFS] says even first; other descriptions say odd. Rather than choose, both were tried against a real track and only one produced agreeing checksums — odd first. This is recorded because the wrong choice does not fail loudly: it yields plausible-looking bytes and a silently wrong disk.

### A track is a bit stream, not a byte stream

**Sectors do not begin on byte boundaries.** There is no reason they should: the Amiga writes a track continuously, and where a sync happens to land in the file that stores it is arbitrary. Measured across the corpus, most do not — in one `Realm of the Trolls` track every sync sits at bit offset ≡ 7 (mod 8).

A byte-aligned scan therefore finds nothing at all on most tracks. The first implementation here did exactly that and decoded 8% of the corpus's sectors, which looked like a disappointing disk rather than a broken reader. Sync must be searched at **bit** granularity and the sector read from that bit offset.

### Sync marks are not sector marks

Two sync words is the norm; **three occurs throughout the corpus**, and the body begins after the last of them. Miscounting lands in the gap, which decodes to a header claiming format `0xAA` and track 168 — nonsense that a reader must not report as a sector.

More importantly, **a sync mark need not have a sector behind it**. Writing sync marks into the gap is how a custom loader finds its own data: the hardware syncs to them and a standard reader finds nothing. Every raw track in `Wings of Death` and `Realm of the Trolls` is like this — three sync words followed by gap.

A run of gap decodes to all zeros, and **zero satisfies its own checksum**, so a checksum test cannot distinguish a gap from a sector. The format byte `0xFF` is the only reliable marker, and is what ADE requires before reporting a sector at all.

### The decode is self-evidencing

Every sector carries two checksums of its own, so a correct decode needs no oracle: it produces sectors whose own arithmetic agrees, and an incorrect one does not. Measured across the corpus's 1235 raw tracks:

| | count |
|---|---|
| sectors decoding with **both** checksums agreeing | 2095 |
| raw tracks that are fully ordinary (11 sectors, 0–10) | 95 |
| sync marks leading to no sector | 38573 |

Two thousand sectors agreeing on two independent checksums is not something a wrong decoder reaches by accident. The reading was confirmed a third way as well: `Superman - The Man of Steel_Disk2` track 80 decodes as ten sectors that all say **track 80**, matching their physical position.

That third check matters because two disks do *not* do this — every raw track in `Deep Space` and `Terrorpods` claims to be track 0, with checksums fully valid. That is a property of those disks, not a decoding error, and it is why reconstructing an ADF by physical position fails on them.

### Clock bits and the encoding rule

*Verified 2026-08-25.* In MFM each data bit is preceded by a clock bit, and the clock is set **only when both the previous and the current data bit are zero** — its job is to keep a run of zeros from losing its timing. In the Amiga's byte layout the clock bits are the odd positions (`0xAA`) and the data bits the even ones (`0x55`).

Measured on a real Terrorpods sector, the rule holds exactly:

| region | clock bits wrong |
|---|---|
| data (1024 MFM bytes) | **0** of 4095 |
| header (40 MFM bytes) | **0** of 1279 |
| the two sync words | **2** of 191 |

The last row is the point. `0x4489` is *deliberately* illegal MFM — one violation per sync word — and that is exactly what makes it findable in a stream where ordinary data cannot produce it.

**The clock/data phase is only knowable from a sync word.** A track's byte boundaries say nothing about where its bit pairs begin, so a track with no sync at all cannot be checked; ADE reports that rather than guessing a phase.

Across the corpus, **every one of the 2095 sound sectors is legally encoded**. There is no illegal MFM inside sectors in this material: these disks protect themselves structurally — sync marks in the gaps, non-standard sector layouts — rather than by encoding bytes a drive could not write.

### A violation count is not a protection score

It is tempting to subtract the sync-word count from a track's total violations and call the remainder deliberate illegal MFM. **That was tried and it does not work.** A sync word does not contribute exactly one violation: the transitions into and out of a sync region contribute their own, so a known-good Terrorpods track has 22 sync words and 27 violations.

Counting the baseline two different ways produced two contradictory pictures of which disks were protected — the first made the *most* heavily protected ones look cleanest. ADE therefore reports the raw counts, which are facts, and derives no score from their difference. Isolating deliberate illegal MFM needs the sync boundaries modelled properly, and that is not done.

### What this decoder still does not check

Track length and bit-cell timing are not considered. Long tracks, variable-rate tracks and weak bits are all protection techniques that live in the *timing* rather than in the data, and reading them needs flux rather than MFM — which is what SCP and IPF carry and extended ADF does not.

## Corpus observations

Everything in this section is **measurement, not specification**: a survey of TOSEC Amiga ADF images, first taken on 2026-08-22 over 4288 images and recounted on 2026-08-24 over 4652. It is recorded here because the gap between the documented format and real images is precisely what D-002 gave up when it declined to inherit ADFlib's accumulated knowledge, and measuring it back is how that knowledge is recovered.

### Leading magic

*Recounted 2026-08-24 over 4652 images; the 2026-08-22 figures for 4288 are in brackets where they differ.*

| Count | Leading bytes |
|---|---|
| 4025 [3794] | `DOS\0` |
| 300 | *no recognised magic* |
| 212 [139] | `DOS\1` |
| 79 [20] | `DOS\3` |
| 21 [20] | `DOS\5` |
| 11 | `UAE-1ADF` (extended-ADF) |
| 3 | `DOS` + `0x32` |
| 1 | `DOS\2` |

Every one of the 364 images added since the first survey is `DOS`-prefixed: the unrecognised count and the extended-ADF count are both unchanged. The proportion of AmigaDOS disks rose from 93% to 93.3%, which is to say the shape of the corpus did not move.

Absent entirely: `DOS\4`, `DOS\6`, `DOS\7`. Their absence from one corpus is not evidence they do not matter — `DOS\5` appears twenty-one times and is the case that exposed BUG-001 — but it does mean this corpus cannot validate LNFS handling. `DOS\4` is now covered by generated fixtures under the oracle (§Confirmed against real disks); LNFS is not, which is D-013.

**No corpus image carries a non-AmigaDOS filesystem.** Checked 2026-08-24 against the full registry in §The wider dostype registry — no muFS, PFS, SFS, AFS, CFS, JXFS, `KICK`, or any Unix, swap or CD type. The 300 unrecognised images are custom bootblocks, not other filesystems: `RNC` (Rob Northen copylock), `ATN!`, `NDOS`, `DSK`, and a long tail of one-offs, plus 100 images whose block 0 is entirely zero. So the foreign-dostype path has no corpus material; it matters on RDB devices rather than floppies.

The three `DOS` + `0x32` images — `Shadowlands_Disk2`, `Shadoworlds_Disk2`, `F.1 Manager_Disk2` — carry a flags byte outside the documented three bits (`0x32` = `0b0011_0010`, and in ASCII the bootblock simply reads `DOS2`). ADE decodes what it can — bit 1 set, so international — and reports `0x30` as unrecognised rather than discarding it, then finds no rootblock at 880. Reporting the bits and declining to guess is the correct outcome, and is what C-008 asks for.

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
- ~~**LNFS block layout**~~ — **done 2026-08-24** (§Field-level pass). The entry block, `TYPE_COMMENT` block and root block are now at field level. What replaces this question is narrower and harder: [AOS-LNFS] declares **no long-name file header**, so the three file-only fields are placed by inference from the canonical block shape (§The file header is inferred, not documented), and LNFS has no oracle to check that inference against (§The oracle cannot check LNFS).
- ~~**MFM track and sector framing**~~ — **done 2026-08-25** (§MFM), from [FAQ §2] and [MDFS] and confirmed against 1235 real raw tracks. [RKRM] Appendix C was not needed and remains unconsulted. What replaces this question is narrower: **clock bits are masked off, not verified**, so MFM encoding violations — a real protection technique — cannot currently be detected.
- **muFS (MultiUser FS) variants** — [AFFS] says they are supported by the Linux driver; ADE's position is undecided. The `protect` field's bits 8–15 and 31 are muFS-related. The 2026-08-24 survey pinned down the identifiers (`muFS`, `muF\0`–`muF\5` — the OFS/FFS matrix again with ownership added), so what remains undecided is scope, not identification. None occur in the corpus.
- **5.25" DD geometry** — named in ROADMAP Phase 2; not covered by [FAQ §3], which documents only 3.5" DD and HD.
- **`DOS\6` and `DOS\7` fixtures.** The survey contains neither, so both LNFS variants cannot be validated against real material. `DOS\5` appears twenty-one times and was the case that exposed BUG-001, so the absent ones are not safely assumed unimportant — they need sourcing separately (D-010). **`DOS\4` is resolved rather than sourced**: it is absent from the corpus too, but the generator builds it and `unadf -c` validates it (§Confirmed against real disks), which is the shape D-010's amendment describes.
- **The 750 `DOS`-magic images with no rootblock at 880.** Custom formats wearing an AmigaDOS bootblock, unexamined so far. Worth a pass: some may place a rootblock elsewhere, and the distribution of what they *do* contain would sharpen the F-003 cascade.
- **Non-`DOS` prefixes** — `PFS`, `SFS`, `KICK`. Detection and honest reporting are in scope; mounting is not, for v1.
