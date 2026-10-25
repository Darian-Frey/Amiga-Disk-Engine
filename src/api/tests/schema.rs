//! The JSON surface, field by field (F-015, D-015).
//!
//! # What this is for
//!
//! F-015 promises that field names and fault codes are stable. A promise in a
//! document is not a mechanism: nothing stops a rename, and a rename does not
//! fail loudly downstream — a consumer reading a key that has vanished sees a
//! missing *value*, not an error, and carries on with a wrong answer.
//!
//! So every field ADE can emit is listed here. Any change to the output fails
//! this test, and the fix is to edit the inventory **and** move `json::SCHEMA`
//! in the same commit, where a reviewer sees both at once. That is the same
//! shape as `tools/check-layering.py`: the policy is not that the thing cannot
//! change, it is that it cannot change quietly.
//!
//! # Reading a failure
//!
//! - A field in the inventory but not the output → something was **removed or
//!   renamed**. Major version.
//! - A field in the output but not the inventory → something was **added**.
//!   Minor version.
//!
//! Nested objects are listed by path, `image.geometry.cylinders`, because a
//! field's meaning depends on where it sits. Arrays are traversed into their
//! first element: every element of an ADE array has the same shape.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test scaffolding: a failure to set up is a test failure"
)]

use std::collections::BTreeSet;

use ade_core::json::{SCHEMA, Value};

/// Every field path in a document, in sorted order.
fn paths(value: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    walk(value, "", &mut out);
    out
}

fn walk(value: &Value, prefix: &str, out: &mut BTreeSet<String>) {
    match value {
        Value::Obj(fields) => {
            for (name, child) in fields {
                let path = if prefix.is_empty() {
                    (*name).to_owned()
                } else {
                    format!("{prefix}.{name}")
                };
                out.insert(path.clone());
                walk(child, &path, out);
            }
        }
        Value::Arr(items) => {
            // The first element stands for all of them: an ADE array is
            // homogeneous by construction.
            if let Some(first) = items.first() {
                walk(first, &format!("{prefix}[]"), out);
            }
        }
        _ => {}
    }
}

/// Report what changed, in the terms the policy is written in.
fn compare(document: &str, actual: &BTreeSet<String>, expected: &[&str]) {
    let expected: BTreeSet<String> = expected.iter().map(|s| (*s).to_owned()).collect();
    let removed: Vec<_> = expected.difference(actual).collect();
    let added: Vec<_> = actual.difference(&expected).collect();
    assert!(
        removed.is_empty(),
        "{document}: fields removed or renamed — a MAJOR change under D-015: {removed:?}"
    );
    assert!(
        added.is_empty(),
        "{document}: fields added — a MINOR change under D-015: {added:?}"
    );
}

/// The same volume with its bitmap flag and bootblock checksum broken, so the
/// report has findings to describe.
fn damaged() -> Vec<u8> {
    let mut volume = ade_fixtures::Volume::dd(1).named("SCHEMA");
    volume.add_file("a", b"contents");
    let root = volume.root();
    let mut bytes = volume.build();
    ade_fixtures::corrupt::bitmap_flag_invalid(&mut bytes, root);
    ade_fixtures::corrupt::bootblock_checksum(&mut bytes);
    bytes
}

/// A partitioned device, for the fields only a hard disk produces.
fn partitioned() -> Vec<u8> {
    let mut device = ade_fixtures::device::Device::new(64, 4, 32);
    device.add_partition("DH0", 2, 30, 1, true, |v| {
        v.add_file("startup", b"hello");
    });
    device.build()
}

/// A small volume with one of everything the inspection can report on.
fn image() -> Vec<u8> {
    let mut volume = ade_fixtures::Volume::dd(1).named("SCHEMA");
    volume.add_file("a", b"contents");
    volume.add_dir("Tools");
    volume.build()
}

#[test]
fn the_version_is_the_first_field_of_every_document() {
    // First so it can be read without parsing the rest, which is the whole
    // point of putting it in the document rather than in the documentation.
    let json = ade_core::inspect_bytes(image()).to_json().versioned();
    assert!(
        json.to_json()
            .starts_with(&format!(r#"{{"schema":"{SCHEMA}","#))
    );
}

#[test]
fn info_emits_exactly_these_fields() {
    let json = ade_core::inspect_bytes(image()).to_json().versioned();
    compare(
        "info",
        &paths(&json),
        &[
            "schema",
            "container",
            "size",
            "evidence",
            "compression",
            "geometry",
            "geometry.block_size",
            "geometry.cylinders",
            "geometry.heads",
            "geometry.sectors",
            "geometry.total_blocks",
            "bootblock",
            "bootblock.checksum_valid",
            "bootblock.dostype",
            "bootblock.dostype.dircache",
            "bootblock.dostype.filesystem",
            "bootblock.dostype.flags",
            "bootblock.dostype.international",
            "bootblock.dostype.label",
            "bootblock.dostype.raw",
            "bootblock.dostype.unrecognised_flags",
            "bootblock.has_boot_code",
            "bootblock.is_dos",
            "bootblock.prefix",
            "bootblock.stored_rootblock",
            "volume",
            "volume.bitmap_flag_valid",
            "volume.checksum_valid",
            "volume.created",
            "volume.hash_table_size",
            "volume.modified",
            "volume.name",
            "volume.root_altered",
            "volume.rootblock",
            "volume_absent",
            "tracks",
            "flux",
            "assembly",
            "description",
            "boot_text",
            "rdb",
            "partitions",
            "partition_faults",
            "identified",
            "faults",
        ],
    );
}

#[test]
fn a_directory_entry_emits_exactly_these_fields() {
    let bytes = image();
    let img = ade_core::Image::from_bytes(bytes).unwrap();
    let volume = img.volume().unwrap();
    let root = volume.root();
    let listing = volume.list(root).unwrap();
    let entry = listing
        .entries
        .iter()
        .find(|e| e.kind.is_file())
        .expect("the fixture has a file");
    let json = ade_core::entry_to_json(entry, &volume.path_components(entry)).versioned();
    compare(
        "ls",
        &paths(&json),
        &[
            "schema",
            "name",
            "path",
            "kind",
            "size",
            "block",
            "parent",
            "protection",
            "protection_bits",
            "comment",
            "altered",
            "checksum_valid",
            "sha1",
        ],
    );
}

#[test]
fn check_emits_exactly_these_fields() {
    // A *damaged* image, because an array's element fields cannot be
    // inventoried from an empty array — a clean disk has no findings, and the
    // inventory would then quietly stop covering `findings[].code`. Every
    // optional part of the output needs an input that produces it.
    let json = ade_core::health::examine(damaged()).to_json().versioned();
    let actual = paths(&json);
    // `image` is the whole inspection, already pinned above; checking it again
    // here would fail twice for one change and say the same thing both times.
    let own: BTreeSet<String> = actual
        .iter()
        .filter(|p| !p.starts_with("image."))
        .cloned()
        .collect();
    compare(
        "check",
        &own,
        &[
            "schema",
            "image",
            "examined",
            "examined.partition",
            "examined.rootblock",
            "examined.volume",
            "tree",
            "tree.bytes_recovered",
            "tree.directories",
            "tree.files",
            "bitmap",
            "bitmap.actually_used",
            "bitmap.at_risk_blocks",
            "bitmap.covered",
            "bitmap.fill_percent",
            "bitmap.flagged_valid",
            "bitmap.marked_used",
            "bitmap.orphaned",
            "bitmap.referenced_but_free",
            "dircache",
            "findings",
            "findings[].block",
            "findings[].code",
            "findings[].message",
            "findings[].severity",
            "summary",
            "summary.error",
            "summary.info",
            "summary.warning",
            "summary.worst",
        ],
    );
}

#[test]
fn the_conversion_matrix_emits_exactly_these_fields() {
    let json = ade_core::convert::matrix_json().versioned();
    compare(
        "formats",
        &paths(&json),
        &[
            "schema",
            "conversions",
            "conversions[].from",
            "conversions[].to",
            "conversions[].from_label",
            "conversions[].to_label",
            "conversions[].conversion",
            "conversions[].conversion.kind",
            "conversions[].conversion.possible",
            "conversions[].conversion.reason",
        ],
    );
}

#[test]
fn diff_emits_exactly_these_fields() {
    let a = vec![0u8; 512 * 1760];
    let mut b = a.clone();
    b[512 * 13] = 0xFF;
    let json = ade_core::consolidate::diff(&a, &b)
        .unwrap()
        .to_json()
        .versioned();
    compare(
        "diff",
        &paths(&json),
        &[
            "schema",
            "identical",
            "sectors_total",
            "sectors_differing",
            "bytes_differing",
            "sectors",
            "tracks",
        ],
    );
}

#[test]
fn consolidate_emits_exactly_these_fields() {
    let a = vec![0u8; 512 * 1760];
    let mut b = a.clone();
    b[512 * 13] = 0xFF;
    let json = ade_core::consolidate::consolidate(&[a, b])
        .unwrap()
        .to_json()
        .versioned();
    compare(
        "consolidate",
        &paths(&json),
        &[
            "schema",
            "sources",
            "sectors_total",
            "unanimous",
            "agreed_sectors",
            "resolved_sectors",
            "unresolved_sectors",
            "can_vote",
            "tracks",
            "tracks[].track",
            "tracks[].disputed",
            "tracks[].unresolved",
        ],
    );
}

#[test]
fn identify_emits_exactly_these_fields() {
    let entries = ade_catalogue::parse(
        r#"<datafile><game name="g"><rom name="x.adf" size="880" crc="deadbeef"/></game></datafile>"#,
        "test.dat",
    );
    let refs: Vec<&ade_catalogue::Entry> = entries.iter().collect();
    let json = ade_core::batch::identification_json("disk.adf", &refs, ade_catalogue::Match::Named)
        .versioned();
    compare(
        "identify",
        &paths(&json),
        &[
            "schema",
            "path",
            "identified",
            "ambiguous",
            "match",
            "matches",
            "matches[].name",
            "matches[].source",
        ],
    );
}

#[test]
fn a_hard_disk_emits_exactly_these_extra_fields() {
    // `rdb` and `partitions` are null on a floppy, and a null object hides
    // every field beneath it. Without an input that produces them, the
    // inventory would cover the names and none of their contents.
    let json = ade_core::inspect_bytes(partitioned()).to_json().versioned();
    let device: Vec<String> = paths(&json)
        .into_iter()
        .filter(|p| p.starts_with("rdb.") || p.starts_with("partitions["))
        .collect();
    let expected = [
        "partitions[].block_size",
        "partitions[].blocks",
        "partitions[].bootable",
        "partitions[].checksum_valid",
        "partitions[].dostype",
        "partitions[].first_block",
        "partitions[].high_cylinder",
        "partitions[].low_cylinder",
        "partitions[].mount_error",
        "partitions[].name",
        "partitions[].reserved",
        "partitions[].volume_name",
        "rdb.block",
        "rdb.block_size",
        "rdb.checksum_valid",
        "rdb.cylinders",
        "rdb.heads",
        "rdb.high_rdsk_block",
        "rdb.product",
        "rdb.revision",
        "rdb.sectors",
        "rdb.vendor",
    ];
    compare("info (hard disk)", &device.into_iter().collect(), &expected);
}

#[test]
fn batch_emits_exactly_these_fields() {
    // The record shape, from a file that cannot be read — the cheapest input
    // that still produces a whole record, and the one a corpus run hits.
    let record = ade_core::batch::examine_one(std::path::Path::new("no-such-image.adf"));
    compare(
        "batch record",
        &paths(&record.to_json().versioned()),
        &[
            "schema",
            "path",
            "size",
            "container",
            "container_code",
            "sha1",
            "conversion",
            "volume",
            "files",
            "directories",
            "bytes_recovered",
            "findings",
            "worst",
            "unreadable",
            "identified",
        ],
    );
}

#[test]
fn a_scan_emits_exactly_these_fields() {
    // A hit is planted rather than hoped for: an empty `hits` array would show
    // none of its element fields, which is how an inventory quietly stops
    // covering something.
    let mut bytes = vec![0u8; 4096];
    bytes[512..516].copy_from_slice(b"PP20");
    let found = ade_core::scan::Scan::of(&bytes, 512);
    assert!(!found.is_empty(), "the fixture should be recognised");
    compare(
        "scan",
        &paths(&found.to_json().versioned()),
        &[
            "schema",
            "scanned",
            "found",
            "hits",
            "hits[].name",
            "hits[].category",
            "hits[].offset",
            "hits[].block",
            "hits[].blocks",
        ],
    );
}

#[test]
fn a_search_emits_exactly_these_fields() {
    // Over a mounted volume with a planted hit, so `matches[].file` is a
    // string rather than the null an unowned block gives — a null leaf and a
    // string leaf inventory the same, but only a real volume proves the owner
    // map is wired in at all.
    let mut v = ade_fixtures::Volume::dd(1).named("Schema");
    v.add_file("readme", b"FINDME");
    let pattern = ade_core::layers::object::find::Pattern::parse("FINDME", true, false).unwrap();
    let found = ade_core::find::Search::run(&v.build(), &pattern);
    assert!(!found.matches.is_empty(), "the fixture should be found");
    compare(
        "find",
        &paths(&found.to_json().versioned()),
        &[
            "schema",
            "scanned",
            "hex",
            "found",
            "matches",
            "matches[].offset",
            "matches[].block",
            "matches[].file",
            "matches[].file_block",
            "matches[].region",
        ],
    );
}

#[test]
fn a_carve_emits_exactly_these_fields() {
    // A volume whose root hash table has been cleared: the files are still on
    // the disk and nothing reaches them, which is what deletion looks like
    // from outside.
    let mut v = ade_fixtures::Volume::dd(0).named("Schema");
    v.add_file("lost", b"contents that outlive their directory entry");
    let mut bytes = v.build();
    let root = 880usize * 512;
    for slot in 0..72usize {
        ade_core::layers::endian::put_u32(&mut bytes, root + 24 + slot * 4, 0).unwrap();
    }
    let block = &mut bytes[root..root + 512];
    ade_core::layers::endian::put_u32(block, 20, 0).unwrap();
    let sum = ade_core::layers::block::checksum::normal_at(block, 20).unwrap();
    ade_core::layers::endian::put_u32(block, 20, sum).unwrap();

    let image = ade_core::Image::from_bytes(bytes).unwrap();
    let found = ade_core::carve::carve(&image);
    assert!(!found.is_empty(), "the lost file is findable");
    compare(
        "carve",
        &paths(&ade_core::carve::to_json(&found).versioned()),
        &[
            "schema",
            "found",
            "carved",
            "carved[].block",
            "carved[].name",
            "carved[].size",
            "carved[].kind",
            "carved[].evidence",
            "carved[].data_blocks",
        ],
    );
}

#[test]
fn a_layout_emits_exactly_these_fields() {
    let mut v = ade_fixtures::Volume::dd(1).named("Schema");
    v.add_file("readme", b"hello");
    let image = ade_core::Image::from_bytes(v.build()).unwrap();
    let map = ade_core::layout::Layout::of(&image);
    assert!(!map.spans.is_empty(), "a formatted disk has spans");
    compare(
        "layout",
        &paths(&map.to_json().versioned()),
        &[
            "schema",
            "block_size",
            "blocks",
            "mounted",
            "spans",
            "spans[].offset",
            "spans[].block",
            "spans[].blocks",
            "spans[].region",
            "spans[].file",
            "spans[].file_block",
        ],
    );
}

#[test]
fn the_batch_summary_emits_exactly_these_fields() {
    // Over a real image, and a damaged one: the arrays are the point here, and
    // an empty array shows none of its element fields. A summary of nothing
    // would inventory `containers` and `findings` as leaves and quietly stop
    // covering everything inside them.
    let dir = std::env::temp_dir().join(format!("ade-schema-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let good = dir.join("good.adf");
    let bad = dir.join("bad.adf");
    std::fs::write(&good, image()).unwrap();
    std::fs::write(&bad, damaged()).unwrap();

    // Through the real entry point, so the summary is the one a corpus run
    // produces rather than one assembled by the test — and **converting**, so
    // `conversions[]` is non-empty. An empty array shows none of its element
    // fields, which is how an inventory quietly stops covering them.
    let request = ade_core::batch::ConvertRequest {
        to: ade_core::layers::container::Kind::Hardfile,
        into: dir.join("converted"),
    };
    let summary =
        ade_core::batch::run_converting(&[good, bad], None, false, Some(&request), |_, _| {});
    let _ = std::fs::remove_dir_all(&dir);
    compare(
        "batch summary",
        &paths(&summary.to_json().versioned()),
        &[
            "schema",
            "examined",
            "unreadable",
            "mounted",
            "sound",
            "bytes_recovered",
            "identified",
            "containers",
            "containers[].name",
            "containers[].count",
            "findings",
            "findings[].code",
            "findings[].images",
            "conversions",
            "conversions[].code",
            "conversions[].images",
        ],
    );
}

// --- The three documents BUG-012 added (schema 1.10 → 1.11) ---------------

#[test]
fn create_emits_exactly_these_fields() {
    let json =
        ade_core::create::created_json(std::path::Path::new("x.adf"), "ffs-intl", "Empty", 901_120)
            .versioned();
    compare(
        "create",
        &paths(&json),
        &["schema", "written", "bytes", "type", "volume"],
    );
}

#[test]
fn convert_emits_exactly_these_fields() {
    use ade_core::layers::container::Kind;
    let json = ade_core::convert::converted_json(
        Kind::Adf {
            cylinders: 80,
            sectors: 11,
        },
        Kind::Hardfile,
        false,
        std::path::Path::new("x.hdf"),
        901_120,
    )
    .versioned();
    compare(
        "convert",
        &paths(&json),
        &[
            "schema",
            "from",
            "to",
            "from_label",
            "to_label",
            "conversion",
            "conversion.kind",
            "conversion.possible",
            "conversion.reason",
            "written",
            "bytes",
        ],
    );
}

#[test]
fn extract_emits_exactly_these_fields() {
    use ade_core::layers::filesystem::volume::{DataFault, DataFaultKind, FileContents};
    // A read with a fault in it, so the fault's fields are inventoried too.
    let contents = FileContents {
        bytes: vec![0; 10],
        declared_size: 20,
        short_by: 10,
        exceeded_volume: false,
        faults: vec![DataFault {
            kind: DataFaultKind::Zeroed,
            first_block: 900,
            first_index: 1,
            count: 1,
        }],
    };
    let json =
        ade_core::unpack::extracted_json("a", std::path::Path::new("out"), &contents).versioned();
    compare(
        "extract",
        &paths(&json),
        &[
            "schema",
            "path",
            "written",
            "bytes",
            "declared_size",
            "complete",
            "short_by",
            "exceeded_volume",
            "faults",
            "faults[].code",
            "faults[].first_block",
            "faults[].first_index",
            "faults[].count",
            "faults[].message",
        ],
    );
}

// The three documents IMP-011 moved out of the CLI, where nothing inventoried
// them: `extract --all`, `surface` and `specs`.

#[test]
fn an_unpack_emits_exactly_these_fields() {
    let mut v = ade_fixtures::Volume::dd(1).named("Schema");
    v.add_file("readme", b"hello");
    v.add_dir("Tools");
    let image = ade_core::Image::from_bytes(v.build()).unwrap();
    let volume = image.volume().unwrap();
    let dest = std::env::temp_dir().join(format!("ade-schema-unpack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);
    ade_core::unpack::unpack(&volume, &dest).unwrap();
    // The second run skips everything, so `skipped` has an element to read.
    let again = ade_core::unpack::unpack(&volume, &dest).unwrap();
    let _ = std::fs::remove_dir_all(&dest);
    assert!(!again.skipped.is_empty());
    compare(
        "unpack",
        &paths(&again.to_json().versioned()),
        &[
            "schema",
            "files",
            "directories",
            "bytes",
            "skipped",
            "skipped[].path",
            "skipped[].reason",
        ],
    );
}

#[test]
fn a_surface_emits_exactly_these_fields() {
    use ade_core::layers::container::extended::STANDARD_TRACK_BYTES;
    use ade_core::layers::endian::{put_u16, put_u32};
    // A plain image wrapped as an extended ADF of 160 ordinary tracks.
    let plain = ade_fixtures::Volume::dd(1).named("Schema").build();
    let mut extended = vec![0u8; 12];
    extended[..8].copy_from_slice(b"UAE-1ADF");
    put_u16(&mut extended, 10, 160).unwrap();
    for _ in 0..160 {
        let at = extended.len();
        extended.extend_from_slice(&[0u8; 12]);
        put_u32(
            &mut extended,
            at + 4,
            u32::try_from(STANDARD_TRACK_BYTES).unwrap(),
        )
        .unwrap();
        put_u32(
            &mut extended,
            at + 8,
            u32::try_from(STANDARD_TRACK_BYTES * 8).unwrap(),
        )
        .unwrap();
    }
    extended.extend_from_slice(&plain);
    let assembly = ade_core::assemble::of_bytes(&extended).unwrap();
    compare(
        "surface",
        &paths(&assembly.surface_json().versioned()),
        &[
            "schema",
            "sectors_placed",
            "sectors_total",
            "tracks",
            "tracks[].track",
            "tracks[].cylinder",
            "tracks[].head",
            "tracks[].sectors",
            "tracks[].expected",
            "tracks[].source",
        ],
    );
}

#[test]
fn a_specs_report_emits_exactly_these_fields() {
    // A file naming a Release 2 library, so `libraries` has an element.
    let mut v = ade_fixtures::Volume::dd(1).named("Schema");
    v.add_file("prog", b"\0\0opens asl.library here\0");
    let bytes = v.build();
    let image = ade_core::Image::from_bytes(bytes.clone()).unwrap();
    let specs = ade_core::specs::Specs::of(&image, &bytes);
    assert!(!specs.libraries.is_empty(), "{specs:?}");
    compare(
        "specs",
        &paths(&specs.to_json().versioned()),
        &[
            "schema",
            "facts",
            "facts[].what",
            "facts[].because",
            "libraries",
            "unknowable",
            "unknowable[].what",
            "unknowable[].why",
        ],
    );
}
