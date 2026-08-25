//! Bootblock text extraction (Phase 3, F-011) and the AV-002 guarantee.
//!
//! Seventy per cent of the corpus has printable text in its boot code, and it
//! is often the most human-legible thing on the disk: publisher banners, the
//! Copylock notice, cracker signatures, virus-protector menus.
//!
//! What these tests mostly pin down is the *filtering*, because 68k machine
//! code lands in the printable range constantly — `NqNqNq` is three `NOP`s
//! (0x4E71) — and an unfiltered extractor reports mostly opcodes.
//!
//! They also pin the thing AV-002 actually turns on: ADE never executes boot
//! code. That is structural rather than checked, and the test says so out loud
//! so it stays that way.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "tests over data they construct"
)]

use ade_core::inspect_bytes;
use ade_core::layers::filesystem::bootblock::Bootblock;
use ade_fixtures::Volume as Fixture;

/// A bootblock carrying `text` at `offset`, on an otherwise ordinary disk.
fn disk_with_boot_text(offset: usize, text: &[u8]) -> Vec<u8> {
    let mut image = Fixture::dd(1).named("Banner").build();
    image[offset..offset + text.len()].copy_from_slice(text);
    image
}

#[test]
fn a_banner_is_found() {
    let image = disk_with_boot_text(200, b"CRACKED BY SOMEONE 1991");
    let found = Bootblock::text(&image);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].text, "CRACKED BY SOMEONE 1991");
    assert_eq!(found[0].offset, 200);
}

#[test]
fn repeated_opcodes_are_not_text() {
    // 0x4E71 is `NOP`, and a run of them reads as "NqNqNqNq…". Bootblocks are
    // padded with these, so an unfiltered extractor reports them constantly.
    let image = disk_with_boot_text(200, &b"\x4E\x71".repeat(20));
    assert!(
        Bootblock::text(&image).is_empty(),
        "NOP padding is not text"
    );

    // 0x55 is also an instruction byte and pads the same way.
    let image = disk_with_boot_text(200, &b"U".repeat(30));
    assert!(Bootblock::text(&image).is_empty());
}

#[test]
fn a_longer_repeating_unit_is_kept() {
    // `(W)XCOPY(W)XCOPY…` is real — an XCOPY bootblock says so about itself.
    // The period filter must be short enough not to eat it.
    let image = disk_with_boot_text(200, &b"(W)XCOPY".repeat(6));
    let found = Bootblock::text(&image);

    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].text.starts_with("(W)XCOPY"));
}

#[test]
fn library_names_are_dropped() {
    // Every bootblock opens dos.library. Reporting it is noise on every disk
    // in the world, which is worse than useless — it drowns the signal.
    for name in [
        &b"dos.library"[..],
        b"graphics.library",
        b"trackdisk.device",
        b"disk.resource",
    ] {
        let image = disk_with_boot_text(200, name);
        assert!(
            Bootblock::text(&image).is_empty(),
            "{:?} should be dropped",
            String::from_utf8_lossy(name)
        );
    }
}

#[test]
fn a_length_prefix_byte_is_stripped_exactly() {
    // Several bootblocks store BCPL-style length-prefixed strings, and a length
    // in the printable range joins the run. `'` is 0x27 = 39, and the message
    // that follows it in the corpus is exactly 39 characters.
    let message = b"No COOLCAPTURE/KICKTAGPTR viruses found";
    assert_eq!(message.len(), 39);
    let mut payload = vec![39u8];
    payload.extend_from_slice(message);

    let image = disk_with_boot_text(200, &payload);
    let found = Bootblock::text(&image);

    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].text, "No COOLCAPTURE/KICKTAGPTR viruses found",
        "the length byte should be gone"
    );
}

#[test]
fn a_leading_byte_that_is_not_the_length_is_kept() {
    // The strip is exact, not a heuristic: a real character that happens to
    // precede the text must survive.
    let image = disk_with_boot_text(200, b"*STARRED MESSAGE HERE");
    let found = Bootblock::text(&image);

    assert_eq!(found[0].text, "*STARRED MESSAGE HERE");
}

#[test]
fn dense_punctuation_is_not_text() {
    let image = disk_with_boot_text(200, b"/\\|/\\|<>{}~/\\|<>");
    assert!(Bootblock::text(&image).is_empty());
}

#[test]
fn short_runs_are_ignored() {
    let image = disk_with_boot_text(200, b"HI THERE");
    assert!(
        Bootblock::text(&image).is_empty(),
        "8 chars is below the floor"
    );
}

#[test]
fn the_header_is_not_scanned() {
    // `DOS\1` plus the checksum and rootblock pointer are structure, not text,
    // and a checksum can be printable by chance on any disk.
    let image = Fixture::dd(1).named("Header").build();
    let found = Bootblock::text(&image);

    assert!(
        found.iter().all(|t| t.offset >= 12),
        "the 12-byte header should not be reported: {found:?}"
    );
}

#[test]
fn text_is_reported_through_the_inspection() {
    let image = disk_with_boot_text(300, b"SOME PUBLISHER 1990 ALL RIGHTS");
    let inspection = inspect_bytes(image);

    assert_eq!(inspection.boot_text.len(), 1);
    assert_eq!(
        inspection.boot_text[0].text,
        "SOME PUBLISHER 1990 ALL RIGHTS"
    );
}

#[test]
fn a_container_without_a_bootblock_yields_no_text() {
    // An extended ADF's first block is a UAE header, not boot code.
    let mut bytes = vec![0u8; 2048];
    bytes[..8].copy_from_slice(b"UAE-1ADF");
    bytes[200..230].copy_from_slice(b"THIS IS NOT BOOT CODE AT ALL!!");

    assert!(inspect_bytes(bytes).boot_text.is_empty());
}

#[test]
fn extraction_never_executes_and_never_panics() {
    // AV-002's actual guarantee. ADE has no interpreter and no execution path
    // of any kind: boot code is bytes to be read, hashed and displayed, and
    // nothing more. This test exists so that stays true by intent rather than
    // by accident — and to prove the reader survives boot code designed to be
    // hostile to a reader.
    let hostile: [&[u8]; 5] = [
        &[0xFF; 900],  // all bits set
        &[0x00; 900],  // all bits clear
        &[0x4E, 0x71], // NOP
        &[0x60, 0xFE], // BRA to self — an infinite loop, if run
        &[0x4E, 0x40], // TRAP #0
    ];
    for pattern in hostile {
        let mut image = Fixture::dd(1).named("Hostile").build();
        for (i, chunk) in image[12..1024].chunks_mut(pattern.len().max(1)).enumerate() {
            let _ = i;
            let n = chunk.len().min(pattern.len());
            chunk[..n].copy_from_slice(&pattern[..n]);
        }
        // Both of these must simply return.
        let _ = Bootblock::text(&image);
        let _ = inspect_bytes(image);
    }
}
