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
    ADE_INTERNAL      = 6
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
typedef struct AdeCatalogue  AdeCatalogue;  /* a loaded dataset      */

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
