/*
 * A C program that exercises the whole ABI, because a C ABI that has only ever
 * been called from Rust has not been tested at all: the header could disagree
 * with the library and nothing in `cargo test` would notice.
 *
 * Build and run via bridge/tests/run.sh.
 */

#include "../include/ade.h"

#include <stdio.h>
#include <string.h>

static int failures = 0;

static void check(int condition, const char *what) {
    if (condition) {
        printf("  ok    %s\n", what);
    } else {
        printf("  FAIL  %s\n", what);
        failures++;
    }
}

/* Latin-1 bytes are printed as escapes: the point of AdeBytes is that ADE does
 * not claim an encoding, so neither does this. */
static void print_name(AdeBytes name) {
    for (size_t i = 0; i < name.len; i++) {
        unsigned char c = name.data[i];
        if (c >= 0x20 && c < 0x7F) putchar(c); else printf("\\x%02x", c);
    }
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s <image.adf>\n", argv[0]);
        return 2;
    }

    printf("ade_version: %s\n", ade_version());
    check(ade_version() != NULL, "version is not null");

    /* Null and error handling first: a C caller meets these before anything. */
    AdeResult err = ADE_INTERNAL;
    check(ade_image_open(NULL, NULL, &err) == NULL, "opening NULL returns NULL");
    check(err == ADE_NULL_ARGUMENT, "and reports a null argument");

    err = ADE_INTERNAL;
    check(ade_image_open("/nonexistent/nope.adf", NULL, &err) == NULL, "missing file returns NULL");
    check(err == ADE_IO, "and reports IO");

    /* Every accessor must tolerate NULL rather than crash. */
    check(ade_image_container(NULL) == NULL, "container(NULL) is NULL");
    check(ade_image_size(NULL) == 0, "size(NULL) is 0");
    check(ade_image_has_volume(NULL) == false, "has_volume(NULL) is false");
    check(ade_image_volume_name(NULL).len == 0, "volume_name(NULL) is empty");
    check(ade_dir_open(NULL, ADE_WHOLE_IMAGE, 880) == NULL, "dir_open(NULL) is NULL");
    check(ade_listing_count(NULL) == 0, "listing_count(NULL) is 0");
    check(ade_file_read(NULL, ADE_WHOLE_IMAGE, 880) == NULL, "file_read(NULL) is NULL");
    ade_image_free(NULL);      /* must not crash */
    ade_listing_free(NULL);
    ade_buffer_free(NULL);
    check(1, "freeing NULL is harmless");

    /* Now the real image. */
    err = ADE_INTERNAL;
    AdeImage *image = ade_image_open(argv[1], NULL, &err);
    check(image != NULL, "opened the image");
    check(err == ADE_OK, "and reported success");
    if (!image) return 1;

    printf("container: %s\n", ade_image_container(image));
    printf("size:      %llu\n", (unsigned long long)ade_image_size(image));
    check(ade_image_size(image) > 0, "size is non-zero");

    // The partition table, where there is one. A floppy has none, and null
    // here is the answer rather than a failure.
    AdePartitions *table = ade_partitions_open(image);
    if (table) {
        size_t n = ade_partitions_count(table);
        printf("partitions: %zu\n", n);
        for (size_t i = 0; i < n; i++) {
            AdePartition p;
            if (ade_partitions_entry(table, i, &p) != ADE_OK) continue;
            printf("  ");
            print_name(p.name);
            printf("  %u blocks of %u, root %u, %s\n", p.blocks, p.block_size,
                   p.root_block, p.mounts ? "mounts" : "no AmigaDOS volume");
            if (p.mounts) {
                AdeListing *inner = ade_dir_open(image, (uint32_t)i, p.root_block);
                check(inner != NULL, "a partition lists");
                ade_listing_free(inner);
            }
        }
        ade_partitions_free(table);
    } else {
        printf("partitions: none (not a device)\n");
    }
    check(ade_partitions_count(NULL) == 0, "partitions_count(NULL) is 0");
    ade_partitions_free(NULL);


    // A device holds no volume of its own — every volume is inside a
    // partition — so this check comes *after* the partition table, not before.
    // Bailing here first meant a hard disk was never exercised at all.
    if (!ade_image_has_volume(image)) {
        printf("no volume of its own (a device keeps its volumes in partitions)\n");
        ade_image_free(image);
        return failures ? 1 : 0;
    }

    AdeBytes vol = ade_image_volume_name(image);
    printf("volume:    \""); print_name(vol); printf("\"\n");
    check(vol.len > 0, "volume name is not empty");

    uint32_t root = ade_image_root_block(image);
    check(root > 0, "root block is not zero");
    printf("findings:  %zu\n", ade_image_finding_count(image));

    AdeListing *listing = ade_dir_open(image, ADE_WHOLE_IMAGE, root);
    check(listing != NULL, "listed the root directory");
    if (listing) {
        size_t n = ade_listing_count(listing);
        printf("entries:   %zu\n", n);
        check(n > 0, "root has entries");

        AdeEntry entry;
        check(ade_listing_entry(listing, n, &entry) == ADE_NOT_FOUND, "past the end is NOT_FOUND");
        check(ade_listing_entry(listing, 0, NULL) == ADE_NULL_ARGUMENT, "NULL out is rejected");

        uint32_t file_block = 0;
        for (size_t i = 0; i < n && i < 8; i++) {
            if (ade_listing_entry(listing, i, &entry) != ADE_OK) continue;
            printf("  %-10s %8u  ",
                   entry.kind == ADE_ENTRY_DIRECTORY ? "<dir>" : "file",
                   entry.size);
            print_name(entry.name);
            printf("\n");
            if (entry.kind == ADE_ENTRY_FILE && entry.size > 0 && !file_block) {
                file_block = entry.block;
            }
        }

        if (file_block) {
            AdeBuffer *buf = ade_file_read(image, ADE_WHOLE_IMAGE, file_block);
            check(buf != NULL, "read a file");
            if (buf) {
                AdeBytes bytes = ade_buffer_bytes(buf);
                printf("read %zu bytes from block %u\n", bytes.len, file_block);
                check(bytes.len > 0, "file has contents");
                check(bytes.data != NULL, "and a valid pointer");
                ade_buffer_free(buf);
            }
        }
        ade_listing_free(listing);
    }

    // The whole volume, flattened. This is what a front end searches, and the
    // reason it is here rather than in the front end: the traversal carries
    // the cycle detection (AV-001).
    AdeListing *walk = ade_walk_open(image, ADE_WHOLE_IMAGE);
    check(walk != NULL, "walked the volume");
    if (walk) {
        size_t n = ade_listing_count(walk);
        printf("walked:    %zu\n", n);
        check(n > 0, "the walk found entries");

        int with_path = 0;
        for (size_t i = 0; i < n; i++) {
            AdeEntry entry;
            if (ade_listing_entry(walk, i, &entry) != ADE_OK) continue;
            if (entry.path.len > 0 && entry.path.data != NULL) with_path++;
            if (i < 8) {
                printf("  ");
                print_name(entry.path);
                printf("\n");
            }
        }
        check((size_t)with_path == n, "every walked entry carries its path");
        ade_listing_free(walk);
    }

    /* The disk map (F-022). Read from C because only a C compiler checks that
       AdeSpan and AdeRegion in the header match what the library writes — a
       struct that disagrees by one field silently mis-colours a hex view. */
    AdeLayout *layout = ade_layout_open(image, ADE_WHOLE_IMAGE);
    if (layout == NULL) {
        printf("no layout (a device: only ADE_WHOLE_IMAGE is mapped)\n");
    } else {
        size_t n = ade_layout_count(layout);
        printf("layout: %zu spans\n", n);
        check(n > 1, "a formatted disk is more than one span");

        uint64_t at = 0;
        int gaps = 0, bootblocks = 0;
        for (size_t i = 0; i < n; i++) {
            AdeSpan span;
            if (ade_layout_span(layout, i, &span) != ADE_OK) continue;
            if (span.offset != at) gaps++;
            at += span.length;
            if (span.region == ADE_REGION_BOOTBLOCK) bootblocks++;
            if (i < 6) {
                printf("  %8llu +%-6llu %-10s ",
                       (unsigned long long)span.offset,
                       (unsigned long long)span.blocks,
                       ade_region_name(span.region));
                print_name(span.owner);
                printf("\n");
            }
        }
        check(gaps == 0, "the spans tile the image with no gaps");
        check(bootblocks > 0, "the bootblock is named");
        check(ade_region_name(99)[0] == '\0', "an unknown region is empty, not wrong");
        printf("  legend: %s = %s\n", ade_region_name(ADE_REGION_UNCLAIMED),
               ade_region_describes(ADE_REGION_UNCLAIMED));
        ade_layout_free(layout);
    }

    /* Extract everything (F-024), from C, so the header's out-params are
       checked by a compiler rather than by a Rust caller that shares the
       declarations. */
    if (argc > 2) {
        uint64_t written = 0, skipped = 0;
        AdeResult r = ade_unpack(image, ADE_WHOLE_IMAGE, argv[2], &written, &skipped);
        if (r == ADE_OK) {
            printf("unpacked: %llu written, %llu skipped\n",
                   (unsigned long long)written, (unsigned long long)skipped);
            check(written > 0, "a formatted disk yields files");
            check(skipped == 0, "into an empty folder nothing is skipped");
            /* Again, into the same folder: everything collides and nothing is
               overwritten, which is the whole promise. */
            uint64_t again = 0, collided = 0;
            check(ade_unpack(image, ADE_WHOLE_IMAGE, argv[2], &again, &collided) == ADE_OK,
                  "a second run still succeeds");
            check(again == 0, "and writes nothing");
            check(collided == written, "reporting every one as skipped");
        } else {
            printf("unpack: not a volume (%d)\n", (int)r);
        }
        check(ade_unpack(NULL, ADE_WHOLE_IMAGE, argv[2], NULL, NULL) != ADE_OK,
              "a null image is refused");
        check(ade_unpack(image, ADE_WHOLE_IMAGE, NULL, NULL, NULL) != ADE_OK,
              "a null folder is refused");
    }

    ade_image_free(image);
    printf("\n%s\n", failures ? "FAILURES" : "all checks passed");
    return failures ? 1 : 0;
}
