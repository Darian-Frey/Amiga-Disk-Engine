//! The generator must produce structurally valid volumes, or every fixture
//! built on it is worthless. These tests check its output against the format
//! description independently of any ADE parser.

#![allow(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "test code over data it constructs or has already length-checked"
)]

use ade_fixtures::{BSIZE, Volume, corrupt, get_u32, hash_name, normal_checksum};

fn block(img: &[u8], n: u32) -> &[u8] {
    &img[n as usize * BSIZE..(n as usize + 1) * BSIZE]
}

fn checksum_ok(img: &[u8], n: u32) -> bool {
    let b = block(img, n);
    get_u32(b, 20) == normal_checksum(b)
}

#[test]
fn dd_volume_has_canonical_geometry() {
    let v = Volume::dd(0);
    assert_eq!(v.total_blocks(), 1760);
    assert_eq!(v.root(), 880, "rootKey = (2 + 1759) / 2");
    assert_eq!(v.build().len(), 901_120);
}

#[test]
fn hd_volume_puts_the_rootblock_at_1760() {
    let v = Volume::hd(1);
    assert_eq!(v.root(), 1760);
    let img = v.build();
    assert_eq!(img.len(), 1_802_240);
    // ...while the bootblock still claims 880. C-007: the field lies, and the
    // fixture reproduces the lie faithfully so ADE can be tested against it.
    assert_eq!(get_u32(&img, 8), 880);
}

#[test]
fn extra_cylinder_geometries_are_exact() {
    // 81-83 cylinder images occur in the wild (SPEC §Corpus observations).
    for (cyl, size) in [(81u32, 912_384usize), (82, 923_648), (83, 934_912)] {
        assert_eq!(Volume::new(cyl, 2, 11, 0).build().len(), size);
    }
}

#[test]
fn bootblock_is_well_formed() {
    let img = Volume::dd(3).named("Test").build();
    assert_eq!(&img[..3], b"DOS");
    assert_eq!(img[3], 3);
    assert_eq!(
        get_u32(&img, 4),
        ade_fixtures::bootblock_checksum(&img[..BSIZE * 2]),
        "bootblock checksum must validate under its own algorithm"
    );
}

#[test]
fn rootblock_is_well_formed() {
    let v = Volume::dd(0).named("Fixture");
    let root = v.root();
    let img = v.build();
    let b = block(&img, root);
    assert_eq!(get_u32(b, 0), ade_fixtures::T_HEADER);
    assert_eq!(get_u32(b, BSIZE - 4), ade_fixtures::ST_ROOT);
    assert_eq!(get_u32(b, 12), 72, "ht_size for a 512-byte block");
    assert_eq!(get_u32(b, BSIZE - 200), 0xFFFF_FFFF, "bm_flag: -1 is valid");
    assert_eq!(b[BSIZE - 80], 7, "name length");
    assert_eq!(&b[BSIZE - 79..BSIZE - 72], b"Fixture");
    assert!(checksum_ok(&img, root));
}

#[test]
fn the_two_checksum_algorithms_are_not_interchangeable() {
    let img = Volume::dd(0).build();
    let boot = &img[..BSIZE * 2];
    // If these ever agree, one of them is implemented wrong.
    assert_ne!(
        ade_fixtures::bootblock_checksum(boot),
        normal_checksum(boot),
        "add-with-carry-then-complement must differ from sum-then-negate"
    );
}

#[test]
fn files_are_reachable_through_the_hash_table() {
    let mut v = Volume::dd(1);
    let names = ["readme", "data.bin", "a", "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"];
    let blocks: Vec<_> = names.iter().map(|n| v.add_file(n, b"payload")).collect();
    let root = v.root();
    let img = v.build();

    for (name, expect) in names.iter().zip(&blocks) {
        let slot = hash_name(name.as_bytes(), Volume::HT_SIZE, false) as usize;
        let mut cur = get_u32(block(&img, root), 24 + slot * 4);
        let mut found = false;
        let mut guard = 0;
        while cur != 0 && guard < 100 {
            if cur == *expect {
                found = true;
                break;
            }
            cur = get_u32(block(&img, cur), BSIZE - 16);
            guard += 1;
        }
        assert!(found, "{name} not reachable from its hash slot");
    }
}

#[test]
fn ofs_and_ffs_lay_data_out_differently() {
    let payload = vec![0xABu8; 600]; // spills to a second block either way

    let mut ofs = Volume::dd(0);
    let h = ofs.add_file("f", &payload);
    let img = ofs.build();
    let hdr = block(&img, h);
    let first = get_u32(hdr, 16);
    let d = block(&img, first);
    assert_eq!(get_u32(d, 0), ade_fixtures::T_DATA);
    assert_eq!(get_u32(d, 8), 1, "seq_num counts from 1");
    assert_eq!(get_u32(d, 12), 488, "OFS payload is BSIZE - 24 (C-005)");
    assert!(checksum_ok(&img, first), "OFS data blocks are checksummed");

    let mut ffs = Volume::dd(1);
    let h = ffs.add_file("f", &payload);
    let img = ffs.build();
    let first = get_u32(block(&img, h), 16);
    assert_eq!(
        &block(&img, first)[..4],
        &[0xAB; 4],
        "FFS data starts at byte 0"
    );
}

#[test]
fn data_block_pointers_run_backwards() {
    let mut v = Volume::dd(1);
    let h = v.add_file("f", &vec![0u8; BSIZE * 3]);
    let img = v.build();
    let hdr = block(&img, h);
    assert_eq!(get_u32(hdr, 8), 3, "high_seq");
    // The first data block sits at data_blocks[71], i.e. offset BSIZE-204.
    assert_eq!(get_u32(hdr, BSIZE - 204), get_u32(hdr, 16), "first_data");
    // ...and the table descends from there.
    assert!(get_u32(hdr, BSIZE - 204) < get_u32(hdr, BSIZE - 208));
}

#[test]
fn a_set_bitmap_bit_means_free() {
    let mut v = Volume::dd(0);
    let f = v.add_file("f", b"x");
    let root = v.root();
    let img = v.build();
    let bm = get_u32(block(&img, root), BSIZE - 196);
    let b = block(&img, bm);
    let bit = |blk: u32| {
        let idx = blk - ade_fixtures::RESERVED;
        get_u32(b, 4 + (idx / 32) as usize * 4) >> (idx % 32) & 1
    };
    assert_eq!(bit(f), 0, "an allocated block has its bit CLEAR");
    assert_eq!(bit(root), 0, "the rootblock is allocated");
    assert_eq!(
        bit(1500),
        1,
        "an untouched block is free, so its bit is SET"
    );
    assert!(checksum_ok(&img, bm));
}

#[test]
fn international_and_plain_hashing_differ() {
    // Same name, two dostypes: DOS\1 is plain, DOS\5 is dircache and therefore
    // international even with the INTL bit clear (C-006).
    let name = b"\xe4pfel"; // 'äpfel' in Latin-1
    let plain = hash_name(name, Volume::HT_SIZE, false);
    let intl = hash_name(name, Volume::HT_SIZE, true);
    assert_ne!(plain, intl, "accented names must hash differently");
    assert!(
        Volume::dd(5).is_international(),
        "DOS\\5 hashes as international"
    );
    assert!(!Volume::dd(1).is_international());
}

// --- corruption fixtures ----------------------------------------------------

#[test]
fn corruptions_break_exactly_what_they_claim() {
    let v = Volume::dd(0);
    let root = v.root();
    let clean = v.build();

    let mut img = clean.clone();
    corrupt::bootblock_checksum(&mut img);
    assert_ne!(
        get_u32(&img, 4),
        ade_fixtures::bootblock_checksum(&img[..BSIZE * 2])
    );
    assert!(checksum_ok(&img, root), "the rootblock must stay intact");

    let mut img = clean.clone();
    corrupt::bitmap_flag_invalid(&mut img, root);
    assert_eq!(get_u32(block(&img, root), BSIZE - 200), 0, "AV-003");
    assert!(
        checksum_ok(&img, root),
        "a cleared flag is not a checksum error"
    );

    let mut img = clean.clone();
    corrupt::rootblock_wrong_type(&mut img, root);
    assert_ne!(get_u32(block(&img, root), BSIZE - 4), ade_fixtures::ST_ROOT);
}

#[test]
fn hash_chain_cycles_are_built_as_described() {
    let mut v = Volume::dd(1);
    let a = v.add_file("alpha", b"a");
    let b = v.add_file("beta", b"b");
    let img = v.build();

    let mut self_loop = img.clone();
    corrupt::hash_chain_loop(&mut self_loop, a);
    assert_eq!(
        get_u32(block(&self_loop, a), BSIZE - 16),
        a,
        "AV-001, self-cycle"
    );

    let mut two = img;
    corrupt::hash_chain_cycle(&mut two, a, b);
    assert_eq!(get_u32(block(&two, a), BSIZE - 16), b);
    assert_eq!(get_u32(block(&two, b), BSIZE - 16), a);
    // A depth limit cannot distinguish this from a long legitimate chain;
    // only a visited-set can. That is the point of the fixture.
}

#[test]
fn size_anomalies_reproduce_the_survey() {
    let img = Volume::dd(0).build();
    assert_eq!(corrupt::with_trailing_junk(&img, 1).len(), 901_121);
    assert_eq!(corrupt::truncated(&img, 176).len(), 90_112);
    assert_eq!(corrupt::zeroed_volume().len(), 901_120);
}

#[test]
fn non_dos_bootblocks_keep_a_valid_checksum() {
    let v = Volume::dd(0);
    let root = v.root();
    let mut img = v.build();
    corrupt::non_dos_bootblock(&mut img, b"ATN!");
    assert_eq!(&img[..4], b"ATN!");
    assert_eq!(
        get_u32(&img, 4),
        ade_fixtures::bootblock_checksum(&img[..BSIZE * 2])
    );
    assert!(
        checksum_ok(&img, root),
        "ten survey images do exactly this and still mount"
    );
}
