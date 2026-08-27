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
    check(ade_image_open(NULL, &err) == NULL, "opening NULL returns NULL");
    check(err == ADE_NULL_ARGUMENT, "and reports a null argument");

    err = ADE_INTERNAL;
    check(ade_image_open("/nonexistent/nope.adf", &err) == NULL, "missing file returns NULL");
    check(err == ADE_IO, "and reports IO");

    /* Every accessor must tolerate NULL rather than crash. */
    check(ade_image_container(NULL) == NULL, "container(NULL) is NULL");
    check(ade_image_size(NULL) == 0, "size(NULL) is 0");
    check(ade_image_has_volume(NULL) == false, "has_volume(NULL) is false");
    check(ade_image_volume_name(NULL).len == 0, "volume_name(NULL) is empty");
    check(ade_dir_open(NULL, 880) == NULL, "dir_open(NULL) is NULL");
    check(ade_listing_count(NULL) == 0, "listing_count(NULL) is 0");
    check(ade_file_read(NULL, 880) == NULL, "file_read(NULL) is NULL");
    ade_image_free(NULL);      /* must not crash */
    ade_listing_free(NULL);
    ade_buffer_free(NULL);
    check(1, "freeing NULL is harmless");

    /* Now the real image. */
    err = ADE_INTERNAL;
    AdeImage *image = ade_image_open(argv[1], &err);
    check(image != NULL, "opened the image");
    check(err == ADE_OK, "and reported success");
    if (!image) return 1;

    printf("container: %s\n", ade_image_container(image));
    printf("size:      %llu\n", (unsigned long long)ade_image_size(image));
    check(ade_image_size(image) > 0, "size is non-zero");

    if (!ade_image_has_volume(image)) {
        printf("no volume: %s\n", ade_image_volume_absent(image));
        ade_image_free(image);
        return failures ? 1 : 0;
    }

    AdeBytes vol = ade_image_volume_name(image);
    printf("volume:    \""); print_name(vol); printf("\"\n");
    check(vol.len > 0, "volume name is not empty");

    uint32_t root = ade_image_root_block(image);
    check(root > 0, "root block is not zero");
    printf("findings:  %zu\n", ade_image_finding_count(image));

    AdeListing *listing = ade_dir_open(image, root);
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
            AdeBuffer *buf = ade_file_read(image, file_block);
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

    ade_image_free(image);
    printf("\n%s\n", failures ? "FAILURES" : "all checks passed");
    return failures ? 1 : 0;
}
