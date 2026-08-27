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
    uint32_t     block;
    uint32_t     size;
    AdeEntryKind kind;
    uint32_t     protection;
    uint32_t     days;
    uint32_t     mins;
    uint32_t     ticks;
} AdeEntry;

typedef struct AdeImage   AdeImage;   /* an open image   */
typedef struct AdeListing AdeListing; /* a directory listing */
typedef struct AdeBuffer  AdeBuffer;  /* a file's contents  */

/* ADE's version. Static; never freed. */
const char *ade_version(void);

/* Open an image. Returns NULL on failure and writes the reason to `out_err`
 * unless that is NULL. Free with ade_image_free. */
AdeImage *ade_image_open(const char *path, AdeResult *out_err);
void      ade_image_free(AdeImage *image);

/* Borrowed from the image; valid until it is freed. */
const char *ade_image_container(const AdeImage *image);
/* Why there is no volume, or NULL if there is one. Borrowed. */
const char *ade_image_volume_absent(const AdeImage *image);

uint64_t ade_image_size(const AdeImage *image);
bool     ade_image_has_volume(const AdeImage *image);
/* Latin-1, borrowed from the image. Empty when there is no volume. */
AdeBytes ade_image_volume_name(const AdeImage *image);
/* The root directory's block, for ade_dir_open. Zero when there is no volume. */
uint32_t ade_image_root_block(const AdeImage *image);
/* How many findings a health check reports. */
size_t   ade_image_finding_count(const AdeImage *image);

/* List a directory. `block` is a root block or an entry's block. Returns NULL
 * if there is no volume or the block is not a directory. Free with
 * ade_listing_free. */
AdeListing *ade_dir_open(const AdeImage *image, uint32_t block);
size_t      ade_listing_count(const AdeListing *listing);
/* Copies entry `index` into `*out`. ADE_NOT_FOUND past the end. The name in
 * the entry borrows from the listing. */
AdeResult   ade_listing_entry(const AdeListing *listing, size_t index, AdeEntry *out);
void        ade_listing_free(AdeListing *listing);

/* Read a file by its entry block. NULL if it is not a readable file. Free with
 * ade_buffer_free. */
AdeBuffer *ade_file_read(const AdeImage *image, uint32_t block);
/* Borrowed from the buffer; valid until it is freed. */
AdeBytes   ade_buffer_bytes(const AdeBuffer *buffer);
void       ade_buffer_free(AdeBuffer *buffer);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* ADE_H */
