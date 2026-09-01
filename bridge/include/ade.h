/*
 * ade.h — the C ABI over the Amiga Disk Engine core.
 *
 * Written by hand rather than generated. ADE has no dependencies (D-001), and
 * a header is the contract with every future caller: worth reading, worth
 * reviewing, and small enough that generating it would buy nothing.
 *
 * Three things to know before using it:
 *
 *  - Names that came off a disk are AdeBytes, not char*. Amiga filenames are
 *    Latin-1 and routinely hold bytes above 0x7F, so they are handed over as
 *    pointer-and-length and you decide how to decode them. Only ADE's own
 *    diagnostics, which are ASCII by construction, are char*.
 *
 *  - Every pointer is either borrowed from a live handle and valid until that
 *    handle is freed, or owned by you and freed by a named function. Each one
 *    below says which.
 *
 *  - Nothing here panics or unwinds into your stack. Failure is a null
 *    pointer, a zero, or an AdeResult.
 *
 * Licence: Apache-2.0, as the rest of ADE (D-011).
 */

#ifndef ADE_H
#define ADE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* How a call turned out. Part of the ABI once released. */
typedef enum {
    ADE_OK            = 0,
    ADE_NULL_ARGUMENT = 1,
    ADE_IO            = 2,
    ADE_NO_VOLUME     = 3,
    ADE_BAD_ENCODING  = 4,
    ADE_NOT_FOUND     = 5,
    ADE_INTERNAL      = 6,
    ADE_ALREADY_EXISTS = 7
} AdeResult;

/* A borrowed run of bytes with no encoding claimed. `data` is NULL when
 * `len` is zero. */
typedef struct {
    const uint8_t *data;
    size_t         len;
} AdeBytes;

/* What a directory entry is. */
typedef enum {
    ADE_ENTRY_FILE      = 0,
    ADE_ENTRY_DIRECTORY = 1,
    ADE_ENTRY_LINK_FILE = 2,
    ADE_ENTRY_LINK_DIR  = 3,
    ADE_ENTRY_SOFT_LINK = 4,
    ADE_ENTRY_UNKNOWN   = 5
} AdeEntryKind;

/* One directory entry. `name` borrows from the listing it came from and is
 * Latin-1, not UTF-8. `days`/`mins`/`ticks` are an Amiga datestamp: days since
 * 1978-01-01, minutes past midnight, and ticks at 50 Hz. */
typedef struct {
    AdeBytes     name;
    /* Full path from the volume root, for entries from ade_walk_open. Empty
     * for entries from ade_dir_open, which are already relative to the
     * directory that was asked for. */
    AdeBytes     path;
    uint32_t     block;
    uint32_t     size;
    AdeEntryKind kind;
    uint32_t     protection;
    uint32_t     days;
    uint32_t     mins;
    uint32_t     ticks;
} AdeEntry;

typedef struct AdeImage      AdeImage;      /* an open image        */
typedef struct AdeListing    AdeListing;    /* a directory listing  */
typedef struct AdeBuffer     AdeBuffer;     /* a file's contents    */
typedef struct AdePartitions AdePartitions; /* a device's partitions */
typedef struct AdeLayout     AdeLayout;     /* a map of a whole disk */
typedef struct AdeSearch     AdeSearch;     /* a content search      */
typedef struct AdeSpecs      AdeSpecs;      /* what a disk needs     */
typedef struct AdeCatalogue  AdeCatalogue;  /* a loaded dataset      */
typedef struct AdeCarve      AdeCarve;      /* files nothing claims  */

/* Pass as `partition` to mean "the image's own volume, not a partition".
 *
 * A floppy has one volume and no partition table; a hard disk has a partition
 * table and no volume of its own. One selector covers both, which is why the
 * reading calls take it rather than coming in two families. */
#define ADE_WHOLE_IMAGE ((uint32_t)0xFFFFFFFFu)

/* One partition of a device. Names borrow from the AdePartitions they came
 * from and are valid until it is freed. */
typedef struct {
    AdeBytes name;        /* the drive name, "DH0", Latin-1            */
    AdeBytes volume_name; /* the volume's own name; empty if unmounted */
    uint32_t dostype;
    uint32_t first_block; /* on the device                             */
    uint32_t blocks;
    uint32_t block_size;  /* usually 512, and not always               */
    uint32_t reserved;    /* blocks at the front; fixes the rootblock  */
    uint32_t root_block;  /* relative to the partition; 0 if unmounted */
    bool     bootable;    /* flagged bootable                          */
    /* Whether an AmigaDOS volume actually mounts. Worth more than `bootable`:
     * a partition can be flagged bootable and hold nothing, or hold a good
     * volume and not be bootable, or be a PFS/SFS partition ADE cannot read. */
    bool     mounts;
} AdePartition;

/* ADE's version. Static; never freed. */
const char *ade_version(void);

/* A dataset of TOSEC-style datfiles, loaded once and used for every image
 * opened afterwards — 88,921 entries take about 140 ms to load, so a front end
 * pays that at startup rather than per disk. NULL if the directory holds none.
 * Free with ade_catalogue_free. */
AdeCatalogue *ade_catalogue_open(const char *dir);
size_t        ade_catalogue_count(const AdeCatalogue *catalogue);
void          ade_catalogue_free(AdeCatalogue *catalogue);

/* Where a dataset lives when the front end was not told: $ADE_DATFILES, then
 * the conventional data directory. NULL when neither exists, which is the
 * ordinary case and not an error. Free the result with ade_string_free. */
char *ade_datfiles_location(void);
void  ade_string_free(char *text);

/* Open an image. Returns NULL on failure and writes the reason to `out_err`
 * unless that is NULL. Free with ade_image_free.
 *
 * `catalogue` may be NULL. When it is not, the image is identified **as it is
 * opened** — the bytes are in hand exactly once, and the handle keeps a
 * mounted image rather than the file afterwards, so it cannot hash itself
 * later. */
AdeImage *ade_image_open(const char *path, const AdeCatalogue *catalogue,
                         AdeResult *out_err);
void      ade_image_free(AdeImage *image);

/* Borrowed from the image; valid until it is freed. */
const char *ade_image_container(const AdeImage *image);
/* Why there is no volume, or NULL if there is one. Borrowed. */
const char *ade_image_volume_absent(const AdeImage *image);

uint64_t ade_image_size(const AdeImage *image);
bool     ade_image_has_volume(const AdeImage *image);
/* Latin-1, borrowed from the image. Empty when there is no volume. */
AdeBytes ade_image_volume_name(const AdeImage *image);
/* The root directory's block, for ade_dir_open with ADE_WHOLE_IMAGE. Zero when
 * there is no volume — which is every hard disk, whose volumes are inside its
 * partitions. */
uint32_t ade_image_root_block(const AdeImage *image);
/* What the dataset called this image, or empty. Borrowed from the handle. */
AdeBytes ade_image_identified(const AdeImage *image);
/* How many findings a health check reports. */
size_t   ade_image_finding_count(const AdeImage *image);

/* A device's partition table, or NULL if it has no Rigid Disk Block — which is
 * most images, and not a fault. Free with ade_partitions_free.
 *
 * Do not take `first_block` and read from there yourself. A partition carries
 * its own block size and its own reserved-block count, and the rootblock is
 * computed from both: a partition with four reserved blocks instead of two
 * puts its rootblock where a caller assuming the usual layout will not find
 * it. Pass the partition's index to the reading calls instead. */
AdePartitions *ade_partitions_open(const AdeImage *image);
size_t         ade_partitions_count(const AdePartitions *partitions);
/* Copies partition `index` into `*out`. ADE_NOT_FOUND past the end. */
AdeResult      ade_partitions_entry(const AdePartitions *partitions, size_t index,
                                    AdePartition *out);
void           ade_partitions_free(AdePartitions *partitions);

/* List a directory. `block` is a root block or an entry's block; `partition`
 * is an index from ade_partitions_open, or ADE_WHOLE_IMAGE for an image that
 * holds its own volume. Returns NULL if there is no such volume or the block
 * is not a directory. Free with ade_listing_free. */
AdeListing *ade_dir_open(const AdeImage *image, uint32_t partition, uint32_t block);

/* Every entry on the volume, flattened, with full paths. Use this rather than
 * recursing through ade_dir_open yourself: walking an Amiga volume safely is
 * engine logic, not UI logic. A hard link to a directory makes cycles
 * reachable on an uncorrupted disk, and the engine's walk carries a visited
 * set and a depth bound — a cycle grows the path strings without bound even
 * while the entry count stays inside its cap. Free with ade_listing_free. */
AdeListing *ade_walk_open(const AdeImage *image, uint32_t partition);
size_t      ade_listing_count(const AdeListing *listing);
/* Copies entry `index` into `*out`. ADE_NOT_FOUND past the end. The name in
 * the entry borrows from the listing. */
AdeResult   ade_listing_entry(const AdeListing *listing, size_t index, AdeEntry *out);
void        ade_listing_free(AdeListing *listing);

/* What occupies a part of a disk (F-022). Codes, not strings, because a front
 * end colours by them and a switch on an integer is what that wants. */
typedef enum {
    ADE_REGION_BOOTBLOCK = 0, /* boot code and the dostype                */
    ADE_REGION_ROOTBLOCK = 1, /* the volume's name, datestamps, hash table */
    ADE_REGION_BITMAP    = 2, /* which blocks are free                     */
    ADE_REGION_DIRECTORY = 3, /* a directory header, holding its name      */
    ADE_REGION_FILE      = 4, /* a file's header or its data               */
    ADE_REGION_UNCLAIMED = 5  /* nothing points here                       */
} AdeRegion;

/* A run of consecutive blocks that are all the same thing. Owner borrows from
 * the AdeLayout it came from and is valid until that is freed; it is empty for
 * a region no directory entry owns. */
typedef struct {
    uint64_t  offset;   /* first byte                                     */
    uint64_t  length;   /* how many bytes                                 */
    uint64_t  block;    /* first block                                    */
    uint64_t  blocks;   /* how many blocks                                */
    AdeRegion region;
    AdeBytes  owner;    /* the owning path, Latin-1; empty if none        */
    /* The block of the directory entry that owns this run, or 0 for none.
     * Zero is safe as "none": block 0 is a bootblock and never an entry.
     *
     * The path names the owner for a person; this identifies it for a
     * program. A front end showing the disk in a tree already has each
     * entry's block and can match on it exactly, where comparing Latin-1
     * path strings is a comparison that can go wrong in ways a block cannot. */
    uint32_t  owner_block;
} AdeSpan;

/* Map what occupies every block of an image (F-022).
 *
 * The spans tile the image with no gaps and no overlaps, ADE_REGION_UNCLAIMED
 * where nothing else applies, so a front end can colour a whole disk without
 * deciding what to do about a byte the map forgot. Runs rather than blocks: an
 * 880 KB floppy has 1,760 blocks and about ninety spans, and the largest in a
 * 4,652-image corpus has about seven hundred.
 *
 * Works on an image with no mountable volume, which is a quarter of real ones
 * — everything past the reserved blocks is then unclaimed, and the bootblock
 * is still named, because C-008 keeps those two facts separate.
 *
 * Only ADE_WHOLE_IMAGE is mapped today: pass a partition index and this
 * returns NULL. A device's map would have to place several volumes, each with
 * its own block size, at absolute offsets on the device — and no image in the
 * 4,652-image corpus carries an RDB, so there is nothing to verify such a map
 * against. Refused rather than guessed.
 *
 * Free with ade_layout_free. */
AdeLayout *ade_layout_open(const AdeImage *image, uint32_t partition);
size_t     ade_layout_count(const AdeLayout *layout);
/* Copies span `index` into `*out`. ADE_NOT_FOUND past the end. */
AdeResult  ade_layout_span(const AdeLayout *layout, size_t index, AdeSpan *out);
void       ade_layout_free(AdeLayout *layout);

/* A region's short name and its one-line description, for a legend. Static;
 * never freed; empty for a code this build does not know. */
const char *ade_region_name(AdeRegion region);
const char *ade_region_describes(AdeRegion region);

/* What came off one track of the medium (F-029). */
typedef struct {
    uint32_t track;    /* 0-159: cylinder * 2 + head        */
    uint32_t cylinder;
    uint32_t head;
    uint32_t sectors;  /* actually recovered                */
    uint32_t expected; /* a whole track holds this many     */
    uint32_t source;   /* AdeTrackSource                    */
} AdeTrack;

/* Where a track's sectors came from. */
typedef enum {
    ADE_TRACK_SECTORS = 0, /* stored already decoded            */
    ADE_TRACK_RAW_MFM = 1, /* decoded here, out of raw MFM      */
    ADE_TRACK_ABSENT  = 2  /* the container carried nothing     */
} AdeTrackSource;

/* Read what came off each track, for a surface view (F-029).
 *
 * Returns 0 for a container that carries no track-level information — a plain
 * ADF, an ADZ, a hardfile. That is not a failure: those are already sectors,
 * nothing recorded how they were read, and reporting 160 whole tracks would
 * claim a measurement nobody made. Only an extended ADF or a flux capture
 * knows.
 *
 * Fills up to `count` entries and returns how many the disk has, which is 160
 * for a double-density floppy — so a caller can size a buffer by calling with
 * `out` NULL first. Every track is present, including those the container
 * never mentioned: "nothing was recovered here" and "nobody looked here" are
 * the same picture otherwise, and only one is a fact about the disk. */
size_t ade_surface_read(const AdeImage *image, AdeTrack *out, size_t count);

/* Files nothing points at any more, and how far to believe each one (F-030).
 *
 * An orphaned file header sitting in space no directory reaches: what a
 * deletion leaves behind, and what a damaged directory tree leaves behind for
 * everything below the damage.
 *
 * The grading is the point, not a decoration. An OFS data block carries the
 * block of the header that owns it, its sequence number and its own checksum,
 * so a carved file is confirmed by the disk rather than by ADE. An FFS data
 * block carries none of that and can never be better than ADE_EVIDENCE_HEADER
 * — the name and size are sound and nothing at all confirms the contents. A
 * front end that shows those three the same way has thrown away the only
 * reason this feature could be written honestly.
 *
 * Works on disks that do not mount, which are the ones worth carving.
 *
 * Free with ade_carve_free. */
AdeCarve *ade_carve_open(const AdeImage *image);
size_t    ade_carve_count(const AdeCarve *carve);
void      ade_carve_free(AdeCarve *carve);

/* How far the disk itself supports a carved file. */
typedef enum {
    ADE_EVIDENCE_SELF_EVIDENT = 0, /* every data block names this header back */
    ADE_EVIDENCE_PARTIAL      = 1, /* some agree; the file is partly overwritten */
    ADE_EVIDENCE_HEADER       = 2  /* nothing confirms the contents           */
} AdeEvidence;

/* One carved entry. `name` is Latin-1 and borrows from the AdeCarve; `size` is
 * what the header claims, which for a partial recovery is more than comes
 * back. Empty or zero past the end. */
AdeBytes    ade_carve_name(const AdeCarve *carve, size_t index);
uint32_t    ade_carve_block(const AdeCarve *carve, size_t index);
uint32_t    ade_carve_size(const AdeCarve *carve, size_t index);
uint32_t    ade_carve_confirmed(const AdeCarve *carve, size_t index); /* bytes */
AdeEvidence ade_carve_evidence(const AdeCarve *carve, size_t index);
bool        ade_carve_is_directory(const AdeCarve *carve, size_t index);

/* The filename this should be written under: the header's block, then the
 * Amiga name made safe for the host, then `.partial` if the recovery is
 * incomplete. Two lost files routinely share a name — a deleted file and the
 * one that replaced it usually do — and the block is what makes each answer
 * unique. Borrows from the AdeCarve. */
AdeBytes ade_carve_filename(const AdeCarve *carve, size_t index);

/* Write one carved file into `dir`, under ade_carve_filename.
 *
 * ADE_NOT_FOUND for a header-only carve: there is nothing confirmed to write,
 * and a file on disk with the right name and unconfirmed bytes is worse than
 * no file, because somebody will believe it. ADE_ALREADY_EXISTS rather than
 * overwriting. */
AdeResult ade_carve_write(const AdeImage *image, const AdeCarve *carve,
                          size_t index, const char *dir);

/* What a disk says it needs, with the evidence for each claim (F-028).
 *
 * Facts, not a verdict. An ADF carries no manifest, so most of what somebody
 * means by "the hardware this needs" — memory, processor, OCS against AGA,
 * PAL against NTSC — is not in the bytes and cannot be had without running or
 * disassembling the code, which ADE does not do. Every claim here is a lower
 * bound with its evidence named, and `ade_specs_unknowable_*` lists what is
 * missing and why, because a report that simply stops reads as "there is
 * nothing more to know".
 *
 * Free with ade_specs_free. */
AdeSpecs *ade_specs_open(const AdeImage *image);
size_t    ade_specs_count(const AdeSpecs *specs);
/* The claim, and the evidence for it. Both borrow from the AdeSpecs and are
 * valid until it is freed; empty past the end. */
AdeBytes  ade_specs_what(const AdeSpecs *specs, size_t index);
AdeBytes  ade_specs_because(const AdeSpecs *specs, size_t index);
void      ade_specs_free(AdeSpecs *specs);

/* What an Amiga disk image cannot tell anyone, and why not. Static; never
 * freed; empty past the end. */
size_t      ade_specs_unknowable_count(void);
const char *ade_specs_unknowable_what(size_t index);
const char *ade_specs_unknowable_why(size_t index);

/* Whether the container was recognised at all.
 *
 * False for a file that is no kind of disk image — an executable, a document,
 * anything. Distinct from having no volume: a DMS archive or an IPF is
 * recognised and holds no volume ADE can mount, and is worth opening to say
 * so. An unrecognised file is worth *declining*, which is what a front end
 * should do with one rather than showing a row that explains nothing.
 *
 * This exists because dragging files out of a disk and dropping them back on
 * the window opened three Amiga executables as damaged hard disks (BUG-010). */
bool ade_image_recognised(const AdeImage *image);

/* The shape of a disk to make (F-025). */
typedef enum {
    ADE_DISK_DD = 0,    /* 3.5" double density, 880 KB — the norm  */
    ADE_DISK_HD = 1,    /* 3.5" high density, 1.76 MB              */
    ADE_DISK_DD525 = 2, /* 5.25" double density, 440 KB            */
    ADE_DISK_HARD = 3   /* an unpartitioned hard disk; see megabytes */
} AdeDiskShape;

/* The filesystems ADE will write, for a front end to offer.
 *
 * Six, not eight: DOS and DOS (LNFS) are deferred by D-013 on
 * verifiability, and the forty-odd non-AmigaDOS tags are other people's
 * filesystems. Enumerated here rather than listed in a front end so that two
 * front ends cannot disagree with the engine about which disks exist.
 *
 * `ade_create_type_name` is what ade_create takes ("ffs-intl");
 * `ade_create_type_label` is what a person reads ("FFS, international
 * (DOS)"). Both are static and never freed; empty past the end. */
size_t      ade_create_type_count(void);
const char *ade_create_type_name(size_t index);
const char *ade_create_type_label(size_t index);

/* Make a blank disk at `path` (F-019, F-025).
 *
 * `type_name` is one of ade_create_type_name's; NULL means the default, which
 * is `ffs-intl` because everything since Workbench 2.0 writes the
 * international variant. `volume_name` NULL means ADE's default. `megabytes`
 * is read only for ADE_DISK_HARD.
 *
 * **Never overwrites.** ADE_ALREADY_EXISTS if something is already at `path`:
 * a blank disk is the safest write there is precisely because it makes a new
 * file, and giving that away would be a poor trade. */
AdeResult ade_create(const char *path, const char *type_name, const char *volume_name,
                     AdeDiskShape shape, uint32_t megabytes);

/* Write every file on the image into `dir`, creating it if needed (F-024).
 *
 * Names are mapped to what this host can hold: Latin-1 decoded to UTF-8, and
 * anything that cannot be a filename escaped as %XX of its original byte. The
 * mapping lives in the engine so a front end cannot invent its own — get it
 * wrong and a name is lost while appearing to be preserved.
 *
 * **Nothing is ever overwritten.** A target that already exists is skipped and
 * counted, never replaced. Returns the number of files written and sets
 * `*skipped` to how many were not, so a partial recovery is visible without
 * parsing anything. A run over a damaged disk does not stop at the first bad
 * file: one that did would recover nothing.
 *
 * Returns ADE_NO_VOLUME if the image holds none, ADE_IO if `dir` cannot be
 * made. `written` and `skipped` may be NULL. */
AdeResult ade_unpack(const AdeImage *image, uint32_t partition, const char *dir,
                     uint64_t *written, uint64_t *skipped);

/* One place a pattern was found. Owner borrows from the AdeSearch it came from
 * and is valid until that is freed. */
typedef struct {
    uint64_t  offset;      /* into the mounted image                        */
    uint64_t  block;       /* the block it falls in                         */
    AdeRegion region;      /* what that part of the disk is                 */
    AdeBytes  owner;       /* the owning path, Latin-1; empty if none       */
    uint32_t  owner_block; /* the owning entry's block, or 0 for none       */
} AdeMatch;

/* Search an image for text or hex (F-021).
 *
 * `pattern` is read as bytes when it is entirely hex digits and separators
 * pairing into whole bytes, and as Latin-1 text otherwise — so `60 1A` and
 * `deadbeef` are hex while `Copylock` is text. Pass `text` to force the text
 * reading of a pattern that looks like hex; `ade_find_was_hex` reports which
 * way it went, and a front end should show that, because the guess is only
 * safe while it is visible.
 *
 * **This one does not return NULL.** Every other opening call in this header
 * answers failure with a null pointer; this answers it with a handle carrying
 * the reason, because the reason is the useful part. A search that could not
 * run and a search that found nothing are different answers and must not look
 * alike: the first means "ask me again", the second means "it is not there" —
 * the distinction the command line draws with exit 2 against exit 1. A null
 * pointer would collapse them together and throw away the message.
 *
 * So always check `ade_find_error` first: non-empty means no search happened.
 *
 * Free with ade_find_free. */
AdeSearch *ade_find_open(const AdeImage *image, const char *pattern, bool text,
                         bool ignore_case);
size_t     ade_find_count(const AdeSearch *search);
/* Copies match `index` into `*out`. ADE_NOT_FOUND past the end. */
AdeResult  ade_find_match(const AdeSearch *search, size_t index, AdeMatch *out);
/* Whether the pattern was read as hex rather than as text. */
bool       ade_find_was_hex(const AdeSearch *search);
/* Why the pattern was refused, or an empty string if it was not. Borrows from
 * the search; valid until it is freed. */
AdeBytes   ade_find_error(const AdeSearch *search);
void       ade_find_free(AdeSearch *search);

/* Read raw bytes of the mounted image, for a hex view of the disk itself.
 *
 * Offsets are in the space `ade_layout_open` maps and `ade_image_size` counts:
 * the image as it mounts, not the file as it sits on disk. For an ADZ that is
 * the decompressed disk and for a flux capture the reconstruction, which is
 * the only space in which a span at offset 1024 means anything.
 *
 * A short read at the end is not an error — the returned buffer holds what
 * there was. Past the end returns an empty buffer, never NULL, so a caller
 * scrolling off the bottom gets nothing rather than a failure.
 *
 * Free with ade_buffer_free. */
AdeBuffer *ade_image_read(const AdeImage *image, uint64_t offset, uint64_t length);

/* Read a file by its entry block. NULL if it is not a readable file. Free with
 * ade_buffer_free. */
AdeBuffer *ade_file_read(const AdeImage *image, uint32_t partition, uint32_t block);
/* Borrowed from the buffer; valid until it is freed. */
AdeBytes   ade_buffer_bytes(const AdeBuffer *buffer);
void       ade_buffer_free(AdeBuffer *buffer);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* ADE_H */
