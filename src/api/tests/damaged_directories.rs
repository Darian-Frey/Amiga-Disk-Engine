//! A directory whose hash table cannot be listed in full (BUG-016).
//!
//! `ade ls` printed these faults and `ade check` walked past them, so a disk
//! with entries missing read as whole. Measured 2026-10-26 over 3,606 corpus
//! volumes: 29 damaged directories on 22 disks. The corpus half is local only
//! and skips without the corpus; the built cases run in CI.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    reason = "test scaffolding"
)]

use std::path::PathBuf;

use ade_fixtures::{Volume as Fixture, corrupt};

fn damaged(img: Vec<u8>) -> Vec<ade_core::Finding> {
    ade_core::health::examine(img)
        .findings
        .into_iter()
        .filter(|f| f.code == "directory-damaged")
        .collect()
}

#[test]
fn a_sound_disk_has_no_damaged_directory() {
    let mut f = Fixture::dd(1);
    let _ = f.add_dir("Tools");
    let _ = f.add_file("top", b"t");
    assert_eq!(damaged(f.build()), Vec::new());
}

#[test]
fn a_pointer_to_nothing_names_the_directory_and_what_it_points_at() {
    let mut f = Fixture::dd(1);
    let tools = f.add_dir("Tools");
    let mut img = f.build();
    corrupt::hash_slot_out_of_range(&mut img, tools, 3);

    let found = damaged(img);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].severity, ade_core::Severity::Warning);
    assert_eq!(found[0].block, Some(tools));
    assert!(
        found[0]
            .message
            .starts_with("Tools: 1 hash-table pointer leads to no entry"),
        "{}",
        found[0].message
    );
    assert!(
        found[0].message.contains("points outside the volume"),
        "{}",
        found[0].message
    );
}

#[test]
fn a_loop_says_amigados_would_never_finish_listing() {
    // The crack-group case: a README whose chain points at itself.
    let mut f = Fixture::dd(1);
    let a = f.add_file("readme", b"r");
    let root = f.root();
    let mut img = f.build();
    corrupt::hash_chain_loop(&mut img, a);

    let found = damaged(img);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].block, Some(root));
    assert!(
        found[0]
            .message
            .starts_with("the root directory: a hash chain loops")
            && found[0].message.contains("never finishes"),
        "{}",
        found[0].message
    );
}

#[test]
fn a_merge_is_reported_as_a_merge_not_a_loop() {
    let mut f = Fixture::dd(1);
    let a = f.add_file("alpha", b"a");
    let b = f.add_file("beta", b"b");
    let mut img = f.build();
    corrupt::hash_chain_merge(&mut img, a, b);

    let found = damaged(img);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].message,
        format!(
            "the root directory: two hash chains merge at block {b}, which AmigaDOS never writes"
        )
    );
}

#[test]
fn damaged_directories_across_the_corpus() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let (mut volumes, mut disks, mut dirs, mut loops) = (0usize, 0usize, 0usize, 0usize);
    for path in &paths {
        let Ok(image) = ade_core::Image::open(path) else {
            continue;
        };
        let Ok(volume) = image.volume() else { continue };
        let Ok(walk) = volume.walk(volume.root()) else {
            continue;
        };
        volumes += 1;
        // Every directory the walk found damaged is one the report names.
        let bytes = std::fs::read(path).unwrap();
        let reported = damaged(bytes);
        assert_eq!(
            reported.len(),
            walk.damaged.len(),
            "{}: {reported:?}",
            path.display()
        );
        if !walk.damaged.is_empty() {
            disks += 1;
            dirs += walk.damaged.len();
            loops += walk.damaged.iter().filter(|d| !d.cycles.is_empty()).count();
        }
    }
    eprintln!("{volumes} volumes: {dirs} damaged directories on {disks} disks, {loops} looping");
    assert!(volumes > 3000, "the corpus mounted: {volumes}");
    // Measured 2026-10-26: 29 on 22, 7 looping — crack-group directory art
    // on Bomb Busters, Elf, Return to Genesis and ThunderCats.
    assert!((20..40).contains(&dirs), "{dirs}");
    assert_eq!(loops, 7);
}

#[test]
fn two_slots_pointing_outside_the_volume_are_two_faults_not_a_merge() {
    // Xenon 2's root: several slots holding the same out-of-range number.
    // Checked for merges before range, the second read as two chains
    // joining at a block that is not there.
    let f = Fixture::dd(1);
    let root = f.root();
    let mut img = f.build();
    corrupt::hash_slot_out_of_range(&mut img, root, 3);
    corrupt::hash_slot_out_of_range(&mut img, root, 9);

    let found = damaged(img);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0]
            .message
            .starts_with("the root directory: 2 hash-table pointers lead to no entry")
            && !found[0].message.contains("merge"),
        "{}",
        found[0].message
    );
}

#[test]
fn a_directory_with_merged_chains_is_not_written_into() {
    // Inserting into a shared tail would put the entry in both chains.
    // Refused before BUG-016 too, when merges were miscounted as loops.
    use ade_core::edit::{EditError, Session};
    use ade_core::layers::filesystem::edit::EditError as FsEditError;
    let mut f = Fixture::dd(1);
    let a = f.add_file("alpha", b"a");
    let b = f.add_file("beta", b"b");
    let mut img = f.build();
    corrupt::hash_chain_merge(&mut img, a, b);

    let mut s = Session::from_bytes(img).unwrap();
    let root = s.root();
    match s.make_dir(root, b"new") {
        Err(EditError::Refused(FsEditError::Damaged(why))) => {
            assert!(why.contains("merge"), "{why}");
        }
        other => panic!("{other:?}"),
    }
}
