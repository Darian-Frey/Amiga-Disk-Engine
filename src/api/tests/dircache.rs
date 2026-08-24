//! Directory cache blocks (`DOS\4`, `DOS\5`) — Phase 2.
//!
//! A dircache duplicates what the hash chains already hold. That makes it two
//! things at once: a set of blocks a reader must account for, and a second
//! opinion about the directory that can be checked against the first.
//!
//! Both are tested here, and the first is not cosmetic. Before the chain was
//! walked, every `DOS\5` disk in the corpus reported its own cache blocks as
//! orphaned — the health report called space lost that was not lost.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "tests over data they construct"
)]

use std::sync::atomic::{AtomicUsize, Ordering};

use ade_core::layers::endian::put_u32;
use ade_core::layers::filesystem::{
    dircache::{self, Disagreement, T_DIRCACHE},
    entry::Entry,
};
use ade_core::{Image, Severity, examine};
use ade_fixtures::{Volume as Fixture, corrupt};

/// A cached volume with a handful of entries, one of them accented.
fn cached(dostype: u8) -> Vec<u8> {
    let mut v = Fixture::dd(dostype).named("Cached");
    v.add_file("startup", b"hello");
    v.add_file("\u{e4}pfel", b"umlaut");
    v.add_dir("Tools");
    v.add_file("plain", b"x");
    v.build()
}

/// Write a fixture to a uniquely named temporary file.
///
/// The counter matters: these tests run in parallel threads of one process, so
/// the pid alone does not distinguish them and two tests sharing a path race.
fn write_temp(bytes: &[u8], name: &str) -> std::path::PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("ade-dc-{name}-{}-{n}.adf", std::process::id()));
    std::fs::write(&p, bytes).expect("write fixture");
    p
}

#[test]
fn both_dircache_dostypes_carry_a_cache() {
    // `DOS\4` is OFS-with-dircache and `DOS\5` FFS-with-dircache. Neither sets
    // the INTL bit, yet both are international (C-006) — which is exactly why
    // the accented name is in the fixture.
    for dostype in [4u8, 5] {
        let path = write_temp(&cached(dostype), &format!("both{dostype}"));
        let image = Image::open(&path).unwrap();
        let volume = image.volume().unwrap();
        let _ = std::fs::remove_file(&path);

        assert!(
            volume.has_dircache(),
            "DOS\\{dostype} must be recognised as a dircache volume"
        );
        let chain = volume.dircache(volume.root()).unwrap();
        assert!(chain.faults.is_empty(), "{:?}", chain.faults);
        assert_eq!(chain.records().len(), 4, "DOS\\{dostype}");
    }
}

#[test]
fn a_volume_without_the_bit_is_not_searched_for_a_cache() {
    // On a plain volume the `extension` field means something else entirely.
    // Reading it as a cache pointer would walk whatever happens to be there.
    let path = write_temp(&cached(1), "plainvol");
    let image = Image::open(&path).unwrap();
    let volume = image.volume().unwrap();
    let _ = std::fs::remove_file(&path);

    assert!(!volume.has_dircache());
}

#[test]
fn the_cache_agrees_with_the_directory() {
    let path = write_temp(&cached(5), "agree");
    let image = Image::open(&path).unwrap();
    let volume = image.volume().unwrap();

    let chain = volume.dircache(volume.root()).unwrap();
    let entries = volume.list(volume.root()).unwrap().entries;
    let differences = dircache::compare(&chain.records(), &entries);
    let _ = std::fs::remove_file(&path);

    assert!(
        differences.is_empty(),
        "a freshly built cache must match its directory: {differences:?}"
    );
    // Compared against the directory's own names rather than against literals:
    // what matters is that both descriptions carry the same bytes, not which
    // encoding the fixture happened to store them in.
    let mut cached_names: Vec<String> = chain.records().iter().map(|r| r.name_lossy()).collect();
    let mut entry_names: Vec<String> = entries.iter().map(Entry::name_lossy).collect();
    cached_names.sort();
    entry_names.sort();
    assert_eq!(cached_names, entry_names);
    assert!(
        cached_names.iter().any(|n| n.ends_with("pfel")),
        "the accented entry should be cached too: {cached_names:?}"
    );
}

#[test]
fn a_subdirectory_has_its_own_cache() {
    let mut v = Fixture::dd(5).named("Nested");
    let sub = v.add_dir("Tools");
    v.add_file("top", b"a");
    let path = write_temp(&v.build(), "nested");
    let image = Image::open(&path).unwrap();
    let volume = image.volume().unwrap();
    let _ = std::fs::remove_file(&path);

    // `add_dir` only populates the root, so the subdirectory is empty — and an
    // empty directory needs no cache. That is not a fault.
    let chain = volume.dircache(sub).unwrap();
    assert!(chain.faults.is_empty(), "{:?}", chain.faults);
    assert!(chain.blocks.is_empty(), "an empty directory caches nothing");
}

#[test]
fn a_cache_spanning_several_blocks_is_followed() {
    // One 512-byte block holds roughly a dozen records, so this needs the
    // chain rather than a single block. Subwar 2050 in the corpus needs 14.
    let mut v = Fixture::dd(5).named("Many");
    for i in 0..60 {
        v.add_file(&format!("file{i:03}"), b"x");
    }
    let path = write_temp(&v.build(), "many");
    let image = Image::open(&path).unwrap();
    let volume = image.volume().unwrap();

    let chain = volume.dircache(volume.root()).unwrap();
    let entries = volume.list(volume.root()).unwrap().entries;
    let _ = std::fs::remove_file(&path);

    assert!(
        chain.blocks.len() > 1,
        "60 entries need several cache blocks"
    );
    assert_eq!(chain.records().len(), 60);
    assert_eq!(entries.len(), 60);
    assert!(dircache::compare(&chain.records(), &entries).is_empty());

    // Each block should declare what it actually holds.
    for block in &chain.blocks {
        assert_eq!(block.declared_records as usize, block.records.len());
        assert!(block.checksum_valid, "block {} checksum", block.block);
        assert_eq!(block.own_key, block.block, "self pointer");
    }
}

#[test]
fn cache_blocks_are_not_reported_as_orphans() {
    // The regression this whole feature exists for. The cache blocks are
    // marked used in the bitmap; if the walk does not reach them, they are
    // counted as lost space.
    let health = examine(cached(5));

    assert!(
        !health.findings.iter().any(|f| f.code == "orphaned-blocks"),
        "cache blocks must be reachable: {:?}",
        health.findings
    );
    let bitmap = health.bitmap.expect("bitmap");
    assert_eq!(
        bitmap.marked_used, bitmap.actually_used,
        "every used block should be reachable"
    );
    assert_eq!(bitmap.orphaned, 0);
}

#[test]
fn a_stale_size_in_the_cache_is_reported() {
    // Both blocks still checksum. Only comparing the two descriptions finds
    // this, which is why SPEC asks for the comparison rather than a preference.
    let mut bytes = cached(5);
    let cache_block = first_cache_block(&bytes);
    corrupt::dircache_size(&mut bytes, cache_block, 0, 999_999);

    let health = examine(bytes);
    let stale: Vec<&str> = health
        .findings
        .iter()
        .filter(|f| f.code == "dircache-disagrees")
        .map(|f| f.message.as_str())
        .collect();

    assert_eq!(stale.len(), 1, "{:?}", health.findings);
    assert!(stale[0].contains("999999"), "{}", stale[0]);
    assert!(stale[0].contains("size"), "{}", stale[0]);
    assert_eq!(health.worst(), Some(Severity::Warning));
}

#[test]
fn a_renamed_cache_record_is_reported_as_a_name_difference() {
    let mut bytes = cached(5);
    let cache_block = first_cache_block(&bytes);
    // Same length, so the record does not shift: "plain" -> "wrong".
    let record = record_named(&bytes, cache_block, b"plain");
    corrupt::dircache_name(&mut bytes, cache_block, record, b"wrong");

    let health = examine(bytes);
    let messages: Vec<&str> = health
        .findings
        .iter()
        .filter(|f| f.code == "dircache-disagrees")
        .map(|f| f.message.as_str())
        .collect();

    assert_eq!(messages.len(), 1, "{:?}", health.findings);
    assert!(messages[0].contains("wrong"), "{}", messages[0]);
    assert!(messages[0].contains("plain"), "{}", messages[0]);
}

#[test]
fn an_entry_missing_from_the_cache_is_reported() {
    let mut bytes = cached(5);
    let cache_block = first_cache_block(&bytes);
    corrupt::dircache_drop_last(&mut bytes, cache_block);

    let health = examine(bytes);
    let messages: Vec<&str> = health
        .findings
        .iter()
        .filter(|f| f.code == "dircache-disagrees")
        .map(|f| f.message.as_str())
        .collect();

    assert_eq!(messages.len(), 1, "{:?}", health.findings);
    assert!(
        messages[0].contains("omits"),
        "the directory holds an entry the cache does not: {}",
        messages[0]
    );
}

#[test]
fn a_looping_cache_chain_terminates_and_is_reported() {
    // AV-001 again, on a chain that did not exist until now.
    let mut bytes = cached(5);
    let cache_block = first_cache_block(&bytes);
    corrupt::dircache_chain_loop(&mut bytes, cache_block);

    let health = examine(bytes);

    assert!(
        health
            .findings
            .iter()
            .any(|f| f.code == "dircache-chain-broken"),
        "{:?}",
        health.findings
    );
}

#[test]
fn a_record_that_would_run_past_the_block_ends_it() {
    // The lengths are read off the disk, so a lying one must not be believed.
    // A 512-byte buffer whose single record claims a 30-byte name starting at
    // offset 500 cannot fit; the parser must stop rather than read on.
    let mut block = vec![0u8; 512];
    put_u32(&mut block, 0, T_DIRCACHE).unwrap();
    put_u32(&mut block, 12, 3).unwrap(); // claims three records
    // A first record whose name length runs off the end.
    block[24 + 23] = 30;
    block[511] = 22; // a comment length beyond the block

    let parsed = dircache::parse(&block, 7).expect("header parses");

    assert_eq!(parsed.declared_records, 3, "the claim is preserved");
    assert!(
        parsed.records.len() < 3,
        "a record that does not fit must not be invented"
    );
}

#[test]
fn a_block_of_the_wrong_type_is_refused() {
    let mut block = vec![0u8; 512];
    put_u32(&mut block, 0, 2u32).unwrap(); // T_HEADER, not T_DIRCACHE

    assert!(dircache::parse(&block, 7).is_err());
}

#[test]
fn a_zero_length_name_ends_the_block() {
    // Zero is not a legal name length (SPEC says 1–30). Accepting it would
    // make a record of fixed size and march through the rest of the block
    // producing nonsense entries.
    let mut block = vec![0u8; 512];
    put_u32(&mut block, 0, T_DIRCACHE).unwrap();
    put_u32(&mut block, 12, 5).unwrap();

    let parsed = dircache::parse(&block, 7).unwrap();

    assert!(parsed.records.is_empty(), "{:?}", parsed.records);
}

#[test]
fn comparison_reports_a_cached_entry_the_directory_lacks() {
    // The other direction from `an_entry_missing_from_the_cache`: a record
    // pointing at a block the hash chains never reach.
    let path = write_temp(&cached(5), "phantom");
    let image = Image::open(&path).unwrap();
    let volume = image.volume().unwrap();
    let chain = volume.dircache(volume.root()).unwrap();
    let records = chain.records();
    let _ = std::fs::remove_file(&path);

    // Compare against an empty directory: every record is then unmatched.
    let differences = dircache::compare(&records, &[]);

    assert_eq!(differences.len(), records.len());
    assert!(
        differences
            .iter()
            .all(|d| matches!(d, Disagreement::NotInDirectory { .. })),
        "{differences:?}"
    );
}

/// The first cache block of a built image, found through its rootblock.
fn first_cache_block(bytes: &[u8]) -> u32 {
    let path = write_temp(bytes, "locate");
    let image = Image::open(&path).unwrap();
    let volume = image.volume().unwrap();
    let block = volume.rootblock().dircache;
    let _ = std::fs::remove_file(&path);
    assert_ne!(block, 0, "the fixture should have built a cache");
    block
}

/// Index of the record with the given name, within a cache block.
fn record_named(bytes: &[u8], block: u32, name: &[u8]) -> usize {
    let parsed = dircache::parse(
        &bytes[block as usize * 512..(block as usize + 1) * 512],
        block,
    )
    .expect("parse cache block");
    parsed
        .records
        .iter()
        .position(|r| r.name == name)
        .expect("record present")
}
