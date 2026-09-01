//! Assembling a raw-track container into a filesystem view (Phase 4, F-007).
//!
//! An extended ADF holds tracks, not a volume — but most of a protected disk is
//! usually ordinary, and that part is a perfectly good AmigaDOS volume nothing
//! was reading. Six of the corpus's eleven extended ADFs mount this way.
//!
//! What these tests mostly guard is honesty. The result is a **reconstruction**:
//! sectors that could not be decoded are zeros, so a listing can silently omit
//! half a disk. Every path that mounts one must report how much is real.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::panic,
    reason = "tests over data they construct"
)]

use ade_core::layers::container::extended::{ExtendedAdf, STANDARD_TRACK_BYTES};
use ade_core::layers::endian::{put_u16, put_u32};
use ade_core::{Image, assemble, examine, inspect_bytes};

/// Wrap a plain image's tracks as an extended ADF of type-0 tracks.
fn as_extended(image: &[u8], tracks: usize) -> Vec<u8> {
    let mut out = vec![0u8; 12];
    out[..8].copy_from_slice(b"UAE-1ADF");
    put_u16(&mut out, 10, tracks as u16).unwrap();
    for _ in 0..tracks {
        let at = out.len();
        out.extend_from_slice(&[0u8; 12]);
        put_u16(&mut out, at + 2, 0).unwrap();
        put_u32(&mut out, at + 4, STANDARD_TRACK_BYTES as u32).unwrap();
        put_u32(&mut out, at + 8, (STANDARD_TRACK_BYTES * 8) as u32).unwrap();
    }
    out.extend_from_slice(&image[..tracks * STANDARD_TRACK_BYTES]);
    out
}

#[test]
fn a_fully_ordinary_container_assembles_to_the_original() {
    // The base case: if every track is ordinary, the reconstruction is the
    // disk, byte for byte.
    let mut fixture = ade_fixtures::Volume::dd(1).named("Whole");
    fixture.add_file("startup", b"hello from a raw-track container");
    fixture.add_dir("Tools");
    let plain = fixture.build();
    let extended = as_extended(&plain, 160);

    let parsed = ExtendedAdf::parse(&extended).unwrap();
    let assembly = assemble(&parsed, &extended);

    assert_eq!(assembly.sectors_placed, 1760);
    assert_eq!(assembly.percent_complete(), 100);
    assert_eq!(
        assembly.bytes, plain,
        "a complete assembly is the disk itself"
    );
}

#[test]
fn a_partial_container_reports_how_much_is_real() {
    // Half a disk. The listing that follows is half fiction, so the count is
    // the thing a caller must not be able to miss.
    let plain = ade_fixtures::Volume::dd(1).named("Half").build();
    let extended = as_extended(&plain, 80);

    let parsed = ExtendedAdf::parse(&extended).unwrap();
    let assembly = assemble(&parsed, &extended);

    assert_eq!(assembly.sectors_placed, 880);
    assert_eq!(assembly.sectors_total, 1760);
    assert_eq!(assembly.percent_complete(), 50);
    assert_eq!(assembly.from_sector_tracks, 80);
    assert_eq!(assembly.from_raw_tracks, 0);
    // The missing half is zeros, not garbage.
    assert!(
        assembly.bytes[80 * STANDARD_TRACK_BYTES..]
            .iter()
            .all(|&b| b == 0)
    );
}

#[test]
fn an_assembled_volume_mounts_and_lists() {
    let mut fixture = ade_fixtures::Volume::dd(1).named("Mountable");
    fixture.add_file("readme", b"recovered from tracks");
    fixture.add_dir("Tools");
    let extended = as_extended(&fixture.build(), 160);

    let path = std::env::temp_dir().join(format!("ade-asm-{}.adf", std::process::id()));
    std::fs::write(&path, &extended).unwrap();
    let image = Image::open(&path).unwrap();
    let volume = image.volume().expect("assembled volume mounts");
    let listing = volume.list(volume.root()).unwrap();
    let _ = std::fs::remove_file(&path);

    assert_eq!(volume.rootblock().name_lossy(), "Mountable");
    assert_eq!(listing.entries.len(), 2);
}

#[test]
fn the_inspection_reports_the_container_not_the_reconstruction() {
    // A protected disk is an extended ADF that happens to be readable, not a
    // plain ADF. Reporting the assembled bytes' container would lose that.
    let plain = ade_fixtures::Volume::dd(1).named("Reported").build();
    let inspection = inspect_bytes(as_extended(&plain, 160));

    assert!(
        inspection
            .detection
            .kind
            .to_string()
            .contains("extended ADF"),
        "{}",
        inspection.detection.kind
    );
    assert!(
        inspection.tracks.is_some(),
        "the track table is still reported"
    );
    assert!(inspection.assembly.is_some(), "and so is the assembly");
    assert_eq!(
        inspection.volume.map(|v| v.rootblock.name_lossy()),
        Some("Reported".to_owned())
    );
}

#[test]
fn health_says_the_volume_was_reconstructed() {
    // "880 orphaned blocks" means something quite different on a disk half of
    // which could not be decoded: there the blocks are not lost, they were
    // never recovered. The report has to say so.
    // 120 of 160 tracks: enough to include the rootblock at block 880, which
    // is on track 80 — with fewer, nothing mounts and there is no volume to
    // describe.
    let plain = ade_fixtures::Volume::dd(1).named("Partial").build();
    let health = examine(as_extended(&plain, 120));

    assert!(health.examined.is_some(), "the volume should mount");
    let reconstructed = health
        .findings
        .iter()
        .find(|f| f.code == "volume-reconstructed")
        .expect("the reconstruction must be reported");
    assert!(
        reconstructed.message.contains("1320 of 1760"),
        "{}",
        reconstructed.message
    );
    assert!(
        reconstructed.message.contains("75%"),
        "{}",
        reconstructed.message
    );
}

#[test]
fn a_plain_image_is_never_reported_as_assembled() {
    let plain = ade_fixtures::Volume::dd(1).named("Plain").build();
    let inspection = inspect_bytes(plain.clone());

    assert!(inspection.assembly.is_none());
    assert!(inspection.tracks.is_none());
    assert!(
        !examine(plain)
            .findings
            .iter()
            .any(|f| f.code == "volume-reconstructed")
    );
}

#[test]
fn sectors_are_placed_by_position_not_by_what_they_claim() {
    // Two corpus disks label every raw track as track 0. Trusting that field
    // would pile the whole disk onto track 0; physical position is the only
    // usable placement (SPEC §MFM).
    let plain = ade_fixtures::Volume::dd(1).named("Positioned").build();
    let extended = as_extended(&plain, 160);
    let parsed = ExtendedAdf::parse(&extended).unwrap();

    let assembly = assemble(&parsed, &extended);

    // The rootblock is at block 880, which is track 80 — it can only be there
    // if placement followed position.
    assert_eq!(
        &assembly.bytes[880 * 512..880 * 512 + 4],
        &plain[880 * 512..880 * 512 + 4]
    );
}

#[test]
fn the_corpus_extended_adfs_mount_where_they_can() {
    // Six of eleven yield a volume. That is the whole point of F-007: the
    // ordinary part of a protected disk was previously unreachable.
    let corpus = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    if !corpus.is_dir() {
        eprintln!("no corpus — skipping");
        return;
    }

    let mut examined = 0usize;
    let mut mounted = 0usize;
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&corpus).expect("read corpus").flatten() {
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        if bytes.get(..8) != Some(b"UAE-1ADF") {
            continue;
        }
        examined += 1;
        let inspection = inspect_bytes(bytes);
        if let Some(volume) = inspection.volume {
            mounted += 1;
            names.push(volume.rootblock.name_lossy());
            // A mounted reconstruction must always carry its completeness.
            assert!(
                inspection.assembly.is_some(),
                "a volume from a raw-track container must report its assembly"
            );
        }
    }

    names.sort();
    eprintln!("extended ADFs: {mounted} of {examined} mount — {names:?}");
    assert!(examined >= 10);
    assert!(mounted >= 5, "expected around six to mount, got {mounted}");
}

#[test]
fn every_track_has_a_state_including_the_ones_nobody_looked_at() {
    // The surface view's data (F-029). A container that mentions 40 tracks
    // still yields 160 states: "nothing was recovered here" and "nobody looked
    // here" are the same picture on a surface view, and only one of them is a
    // fact about the disk.
    use ade_core::assemble::TrackSource;

    let mut fixture = ade_fixtures::Volume::dd(1).named("Partial");
    fixture.add_file("startup", b"half a disk");
    let plain = fixture.build();
    let extended = as_extended(&plain, 40);

    let parsed = ExtendedAdf::parse(&extended).unwrap();
    let assembly = assemble(&parsed, &extended);

    assert_eq!(assembly.tracks.len(), 160, "one per track, always");
    for (index, state) in assembly.tracks.iter().enumerate() {
        assert_eq!(state.index, index);
        assert_eq!(state.cylinder(), index / 2);
        assert_eq!(state.head(), index % 2);
        assert_eq!(state.expected, 11);
        if index < 40 {
            assert_eq!(state.source, TrackSource::Sectors, "track {index}");
            assert_eq!(state.sectors, 11, "track {index}");
        } else {
            assert_eq!(state.source, TrackSource::Absent, "track {index}");
            assert_eq!(state.sectors, 0, "track {index}");
        }
    }

    // And the states agree with the total, which is the sum they came from.
    let counted: usize = assembly.tracks.iter().map(|t| usize::from(t.sectors)).sum();
    assert_eq!(counted, assembly.sectors_placed);
}

#[test]
fn a_container_that_never_recorded_its_tracks_has_no_surface() {
    // A plain ADF is already sectors: every one is present by construction and
    // nothing recorded how it was read. Reporting 160 whole tracks would claim
    // a measurement nobody made, so there is no answer rather than a full one.
    let plain = ade_fixtures::Volume::dd(1).named("Plain").build();
    assert!(ade_core::assemble::of_bytes(&plain).is_none());

    // The same disk wrapped as raw tracks does have one.
    let extended = as_extended(&plain, 160);
    let surface = ade_core::assemble::of_bytes(&extended).expect("a raw-track container");
    assert_eq!(surface.tracks.len(), 160);
}
