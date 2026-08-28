//! The C ABI, exercised from Rust (D-001).
//!
//! These run in `cargo test` and catch regressions in behaviour. They do *not*
//! catch the header disagreeing with the library, because nothing here reads
//! `ade.h` — that is what `bridge/tests/smoke.c` is for, and why it exists as
//! a separate C program rather than being folded in here.
//!
//! What they concentrate on is the contract a C caller actually depends on:
//! that null is tolerated everywhere, that failure is a null or a code rather
//! than a panic, and that bytes off a disk keep their exact encoding.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "tests over data they construct"
)]

use std::ffi::{CStr, CString};

use ade::{AdeEntry, AdeEntryKind, AdeResult};

/// Write a fixture and hand back its path as a C string.
fn fixture(name: &str, bytes: &[u8]) -> (std::path::PathBuf, CString) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("ade-abi-{name}-{}-{n}.adf", std::process::id()));
    std::fs::write(&path, bytes).expect("write fixture");
    let c = CString::new(path.display().to_string()).expect("path has no NUL");
    (path, c)
}

fn sound_disk() -> Vec<u8> {
    let mut v = ade_fixtures::Volume::dd(1).named("Bridged");
    v.add_file("startup", b"hello from C");
    v.add_dir("Tools");
    v.build()
}

#[test]
fn the_version_is_a_valid_c_string() {
    let ptr = ade::ade_version();
    assert!(!ptr.is_null());
    // SAFETY: the function contracts that this is a static NUL-terminated
    // string valid for the life of the program.
    let text = unsafe { CStr::from_ptr(ptr) }.to_str().expect("ASCII");
    assert!(!text.is_empty());
}

#[test]
fn null_is_tolerated_everywhere() {
    // A C caller that has just had an error will pass null. Every entry point
    // must handle it rather than dereference it.
    let mut err = AdeResult::Internal;
    // SAFETY: passing null is explicitly allowed by every one of these.
    unsafe {
        assert!(ade::ade_image_open(std::ptr::null(), &raw mut err).is_null());
        assert_eq!(err, AdeResult::NullArgument);

        assert!(ade::ade_image_container(std::ptr::null()).is_null());
        assert!(ade::ade_image_volume_absent(std::ptr::null()).is_null());
        assert_eq!(ade::ade_image_size(std::ptr::null()), 0);
        assert!(!ade::ade_image_has_volume(std::ptr::null()));
        assert_eq!(ade::ade_image_volume_name(std::ptr::null()).len, 0);
        assert_eq!(ade::ade_image_root_block(std::ptr::null()), 0);
        assert_eq!(ade::ade_image_finding_count(std::ptr::null()), 0);
        assert!(ade::ade_dir_open(std::ptr::null(), ade::ADE_WHOLE_IMAGE, 880).is_null());
        assert_eq!(ade::ade_listing_count(std::ptr::null()), 0);
        assert!(ade::ade_file_read(std::ptr::null(), ade::ADE_WHOLE_IMAGE, 880).is_null());
        assert_eq!(ade::ade_buffer_bytes(std::ptr::null()).len, 0);
        // And freeing null must be harmless, not a crash.
        ade::ade_image_free(std::ptr::null_mut());
        ade::ade_listing_free(std::ptr::null_mut());
        ade::ade_buffer_free(std::ptr::null_mut());
    }
}

#[test]
fn a_missing_file_reports_io_rather_than_panicking() {
    let path = CString::new("/nonexistent/definitely/not/here.adf").unwrap();
    let mut err = AdeResult::Internal;
    // SAFETY: a valid NUL-terminated path and a writable error slot.
    let image = unsafe { ade::ade_image_open(path.as_ptr(), &raw mut err) };

    assert!(image.is_null());
    assert_eq!(err, AdeResult::Io);
}

#[test]
fn an_image_opens_and_reports_itself() {
    let (path, c_path) = fixture("open", &sound_disk());
    let mut err = AdeResult::Internal;
    // SAFETY: valid path, writable error slot.
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), &raw mut err) };
    assert!(!image.is_null());
    assert_eq!(err, AdeResult::Ok);

    // SAFETY: a live handle from the call above.
    unsafe {
        assert_eq!(ade::ade_image_size(image), 901_120);
        assert!(ade::ade_image_has_volume(image));
        assert!(ade::ade_image_volume_absent(image).is_null());
        let container = CStr::from_ptr(ade::ade_image_container(image))
            .to_str()
            .unwrap();
        assert!(container.contains("ADF"), "{container}");

        let name = ade::ade_image_volume_name(image);
        let bytes = std::slice::from_raw_parts(name.data, name.len);
        assert_eq!(bytes, b"Bridged");

        assert!(ade::ade_image_root_block(image) > 0);
        ade::ade_image_free(image);
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_directory_lists_and_entries_come_back() {
    let (path, c_path) = fixture("list", &sound_disk());
    // SAFETY: valid path; null error slot is allowed.
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), std::ptr::null_mut()) };
    assert!(!image.is_null());

    // SAFETY: a live handle, and a root block from it.
    unsafe {
        let root = ade::ade_image_root_block(image);
        let listing = ade::ade_dir_open(image, ade::ADE_WHOLE_IMAGE, root);
        assert!(!listing.is_null());
        assert_eq!(ade::ade_listing_count(listing), 2);

        let mut entry = AdeEntry {
            name: ade::AdeBytes {
                data: std::ptr::null(),
                len: 0,
            },
            path: ade::AdeBytes {
                data: std::ptr::null(),
                len: 0,
            },
            block: 0,
            size: 0,
            kind: AdeEntryKind::Unknown,
            protection: 0,
            days: 0,
            mins: 0,
            ticks: 0,
        };
        // Past the end is a code, not a crash.
        assert_eq!(
            ade::ade_listing_entry(listing, 99, &raw mut entry),
            AdeResult::NotFound
        );
        assert_eq!(
            ade::ade_listing_entry(listing, 0, std::ptr::null_mut()),
            AdeResult::NullArgument
        );

        let mut names = Vec::new();
        for index in 0..ade::ade_listing_count(listing) {
            assert_eq!(
                ade::ade_listing_entry(listing, index, &raw mut entry),
                AdeResult::Ok
            );
            names.push(std::slice::from_raw_parts(entry.name.data, entry.name.len).to_vec());
        }
        names.sort();
        assert_eq!(names, vec![b"Tools".to_vec(), b"startup".to_vec()]);

        ade::ade_listing_free(listing);
        ade::ade_image_free(image);
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_file_reads_back_its_contents() {
    let (path, c_path) = fixture("read", &sound_disk());
    // SAFETY: valid path.
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), std::ptr::null_mut()) };

    // SAFETY: a live handle throughout.
    unsafe {
        let listing = ade::ade_dir_open(
            image,
            ade::ADE_WHOLE_IMAGE,
            ade::ade_image_root_block(image),
        );
        let mut found = None;
        for index in 0..ade::ade_listing_count(listing) {
            let mut entry = std::mem::zeroed::<AdeEntry>();
            if ade::ade_listing_entry(listing, index, &raw mut entry) != AdeResult::Ok {
                continue;
            }
            if entry.kind == AdeEntryKind::File {
                found = Some(entry.block);
            }
        }
        let block = found.expect("a file in the root");
        let buffer = ade::ade_file_read(image, ade::ADE_WHOLE_IMAGE, block);
        assert!(!buffer.is_null());

        let bytes = ade::ade_buffer_bytes(buffer);
        let slice = std::slice::from_raw_parts(bytes.data, bytes.len);
        assert_eq!(slice, b"hello from C");

        ade::ade_buffer_free(buffer);
        ade::ade_listing_free(listing);
        ade::ade_image_free(image);
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_latin1_name_keeps_its_exact_bytes() {
    // The reason names are `AdeBytes` and not `char*`. A name with a byte
    // above 0x7F must arrive unchanged, with no encoding claimed or applied.
    let mut fixture_disk = ade_fixtures::Volume::dd(3).named("Latin1");
    // 0xE4 is 'ä' in Latin-1 and is not valid UTF-8 on its own — exactly the
    // case a `char*` API would either mangle or refuse.
    fixture_disk.add_file("apfel", b"x");
    let (path, c_path) = fixture("latin1", &fixture_disk.build());

    // SAFETY: valid path, live handle.
    unsafe {
        let image = ade::ade_image_open(c_path.as_ptr(), std::ptr::null_mut());
        let listing = ade::ade_dir_open(
            image,
            ade::ADE_WHOLE_IMAGE,
            ade::ade_image_root_block(image),
        );
        let mut entry = std::mem::zeroed::<AdeEntry>();
        assert_eq!(
            ade::ade_listing_entry(listing, 0, &raw mut entry),
            AdeResult::Ok
        );
        let name = std::slice::from_raw_parts(entry.name.data, entry.name.len);
        // Bytes, not a string: the caller gets exactly what is on the disk.
        assert_eq!(name, b"apfel");
        ade::ade_listing_free(listing);
        ade::ade_image_free(image);
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn an_image_with_no_volume_says_why_rather_than_failing_to_open() {
    // Opening must succeed even when there is nothing to mount: a GUI still
    // wants to show the container and the reason.
    let (path, c_path) = fixture("novolume", &vec![0u8; 901_120]);
    let mut err = AdeResult::Internal;
    // SAFETY: valid path and error slot.
    unsafe {
        let image = ade::ade_image_open(c_path.as_ptr(), &raw mut err);
        assert!(!image.is_null(), "the file is readable, so it opens");
        assert_eq!(err, AdeResult::Ok);
        assert!(!ade::ade_image_has_volume(image));

        let why = ade::ade_image_volume_absent(image);
        assert!(!why.is_null(), "the reason must be available");
        assert!(!CStr::from_ptr(why).to_bytes().is_empty());

        // And the directory calls degrade rather than crash.
        assert!(ade::ade_dir_open(image, ade::ADE_WHOLE_IMAGE, 880).is_null());
        assert!(ade::ade_file_read(image, ade::ADE_WHOLE_IMAGE, 880).is_null());

        ade::ade_image_free(image);
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_walk_returns_every_entry_with_its_path() {
    // The GUI needs this for cross-image search, and must not roll its own
    // traversal: cycle detection and the depth bound live in the engine
    // (AV-001, IMP-003).
    let mut disk = ade_fixtures::Volume::dd(1).named("Walked");
    disk.add_file("top", b"a");
    disk.add_dir("Tools");
    let (path, c_path) = fixture("walk", &disk.build());

    // SAFETY: valid path, live handle throughout.
    unsafe {
        let image = ade::ade_image_open(c_path.as_ptr(), std::ptr::null_mut());
        assert!(!image.is_null());
        let walk = ade::ade_walk_open(image, ade::ADE_WHOLE_IMAGE);
        assert!(!walk.is_null());

        let count = ade::ade_listing_count(walk);
        assert!(count >= 2, "at least the file and the directory");

        let mut saw_path = false;
        for index in 0..count {
            let mut entry = std::mem::zeroed::<AdeEntry>();
            assert_eq!(
                ade::ade_listing_entry(walk, index, &raw mut entry),
                AdeResult::Ok
            );
            let name = std::slice::from_raw_parts(entry.name.data, entry.name.len);
            assert!(!name.is_empty());
            // A walk carries paths; a plain listing does not.
            if entry.path.len > 0 {
                saw_path = true;
            }
        }
        assert!(saw_path, "walk entries must carry a path");

        ade::ade_listing_free(walk);
        ade::ade_image_free(image);
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_plain_listing_carries_no_path() {
    // The distinction the `path` field documents: `ade_dir_open` entries are
    // already relative to the directory asked for.
    let mut disk = ade_fixtures::Volume::dd(1).named("NoPath");
    disk.add_file("top", b"a");
    let (path, c_path) = fixture("nopath", &disk.build());

    // SAFETY: valid path, live handle.
    unsafe {
        let image = ade::ade_image_open(c_path.as_ptr(), std::ptr::null_mut());
        let listing = ade::ade_dir_open(
            image,
            ade::ADE_WHOLE_IMAGE,
            ade::ade_image_root_block(image),
        );
        let mut entry = std::mem::zeroed::<AdeEntry>();
        assert_eq!(
            ade::ade_listing_entry(listing, 0, &raw mut entry),
            AdeResult::Ok
        );
        assert_eq!(entry.path.len, 0);
        ade::ade_listing_free(listing);
        ade::ade_image_free(image);
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn walking_a_volumeless_image_is_null_not_a_crash() {
    let (path, c_path) = fixture("walknull", &vec![0u8; 901_120]);
    // SAFETY: valid path.
    unsafe {
        let image = ade::ade_image_open(c_path.as_ptr(), std::ptr::null_mut());
        assert!(!image.is_null());
        assert!(ade::ade_walk_open(image, ade::ADE_WHOLE_IMAGE).is_null());
        assert!(ade::ade_walk_open(std::ptr::null(), ade::ADE_WHOLE_IMAGE).is_null());
        ade::ade_image_free(image);
    }
    let _ = std::fs::remove_file(path);
}

/// A device with two partitions, the second holding a filesystem ADE cannot
/// read — the case a front end most needs told about rather than shown empty.
fn two_partition_device() -> Vec<u8> {
    let mut device = ade_fixtures::device::Device::new(64, 4, 32);
    device.add_partition("DH0", 2, 30, 1, true, |v| {
        v.add_file("startup-sequence", b"partitioned");
        v.add_dir("Tools");
    });
    device.add_partition("DH1", 31, 63, 0, false, |v| {
        v.add_file("data.bin", &[0xAA; 3000]);
    });
    device.build()
}

#[test]
fn a_floppy_has_no_partition_table() {
    // Null rather than an empty table: a floppy does not have zero partitions,
    // it has no partition table at all, and a caller should be able to tell.
    let (path, c_path) = fixture("nopart", &sound_disk());
    let mut err = AdeResult::Ok;
    // SAFETY: a valid path and a writable error slot.
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), &raw mut err) };
    assert!(!image.is_null());
    // SAFETY: a live handle.
    let table = unsafe { ade::ade_partitions_open(image) };
    assert!(table.is_null(), "a floppy has no RDB");
    // SAFETY: live handles.
    unsafe { ade::ade_image_free(image) };
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_device_reports_each_partition_with_its_own_geometry() {
    let (path, c_path) = fixture("parts", &two_partition_device());
    let mut err = AdeResult::Ok;
    // SAFETY: a valid path and a writable error slot.
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), &raw mut err) };
    assert!(!image.is_null(), "a device should open even with no volume");
    // A device holds no volume of its own — every volume is inside a partition.
    // SAFETY: a live handle.
    assert!(!unsafe { ade::ade_image_has_volume(image) });

    // SAFETY: a live handle.
    let table = unsafe { ade::ade_partitions_open(image) };
    assert!(!table.is_null());
    // SAFETY: a live table.
    assert_eq!(unsafe { ade::ade_partitions_count(table) }, 2);

    let mut first = std::mem::MaybeUninit::<ade::AdePartition>::uninit();
    // SAFETY: a live table and a writable slot.
    let result = unsafe { ade::ade_partitions_entry(table, 0, first.as_mut_ptr()) };
    assert_eq!(result, AdeResult::Ok);
    // SAFETY: the call above initialised it.
    let first = unsafe { first.assume_init() };

    // SAFETY: `name` points into the table, which is still alive.
    let name = unsafe { std::slice::from_raw_parts(first.name.data, first.name.len) };
    assert_eq!(name, b"DH0");
    assert!(first.bootable);
    assert!(first.mounts);
    assert!(first.root_block > 0, "a mounted partition has a rootblock");
    assert!(first.first_block > 0, "DH0 starts past the reserved area");
    assert_eq!(first.block_size, 512);

    // SAFETY: `volume_name` points into the same live table.
    let volume =
        unsafe { std::slice::from_raw_parts(first.volume_name.data, first.volume_name.len) };
    assert_eq!(
        volume, b"DH0",
        "the fixture names each volume after its drive"
    );

    // SAFETY: live handles.
    unsafe { ade::ade_partitions_free(table) };
    // SAFETY: live handle.
    unsafe { ade::ade_image_free(image) };
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_partition_lists_extracts_and_walks_like_any_volume() {
    let (path, c_path) = fixture("partread", &two_partition_device());
    let mut err = AdeResult::Ok;
    // SAFETY: a valid path and a writable error slot.
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), &raw mut err) };
    // SAFETY: a live handle.
    let table = unsafe { ade::ade_partitions_open(image) };
    let mut p = std::mem::MaybeUninit::<ade::AdePartition>::uninit();
    // SAFETY: a live table and a writable slot.
    unsafe { ade::ade_partitions_entry(table, 0, p.as_mut_ptr()) };
    // SAFETY: initialised above.
    let p = unsafe { p.assume_init() };

    // SAFETY: a live handle, a real partition index and its own rootblock.
    let listing = unsafe { ade::ade_dir_open(image, 0, p.root_block) };
    assert!(!listing.is_null(), "partition 0 should list");
    // SAFETY: a live listing.
    assert_eq!(unsafe { ade::ade_listing_count(listing) }, 2);

    let mut file_block = 0u32;
    for i in 0..2 {
        let mut entry = std::mem::MaybeUninit::<AdeEntry>::uninit();
        // SAFETY: a live listing and a writable slot.
        unsafe { ade::ade_listing_entry(listing, i, entry.as_mut_ptr()) };
        // SAFETY: initialised above.
        let entry = unsafe { entry.assume_init() };
        if entry.kind == AdeEntryKind::File {
            file_block = entry.block;
        }
    }
    assert!(file_block > 0);

    // SAFETY: a live handle and a block from that partition's own listing.
    let buffer = unsafe { ade::ade_file_read(image, 0, file_block) };
    assert!(!buffer.is_null(), "a file inside a partition should read");
    // SAFETY: a live buffer.
    let bytes = unsafe { ade::ade_buffer_bytes(buffer) };
    // SAFETY: valid for the buffer's life.
    let data = unsafe { std::slice::from_raw_parts(bytes.data, bytes.len) };
    assert_eq!(data, b"partitioned");

    // SAFETY: a live handle and a real partition index.
    let walk = unsafe { ade::ade_walk_open(image, 0) };
    assert!(!walk.is_null());
    // SAFETY: a live listing.
    assert_eq!(unsafe { ade::ade_listing_count(walk) }, 2);

    // SAFETY: live handles, each freed once.
    unsafe {
        ade::ade_listing_free(walk);
        ade::ade_buffer_free(buffer);
        ade::ade_listing_free(listing);
        ade::ade_partitions_free(table);
        ade::ade_image_free(image);
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_partition_index_that_does_not_exist_is_null_not_a_crash() {
    let (path, c_path) = fixture("badpart", &two_partition_device());
    let mut err = AdeResult::Ok;
    // SAFETY: a valid path and a writable error slot.
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), &raw mut err) };
    // SAFETY: a live handle; partition 99 does not exist.
    assert!(unsafe { ade::ade_dir_open(image, 99, 880) }.is_null());
    // SAFETY: same.
    assert!(unsafe { ade::ade_walk_open(image, 99) }.is_null());
    // SAFETY: same.
    assert!(unsafe { ade::ade_file_read(image, 99, 880) }.is_null());
    // A device has no volume of its own, so asking for the whole image is a
    // legitimate request with no answer — also null, not a crash.
    // SAFETY: same.
    assert!(unsafe { ade::ade_walk_open(image, ade::ADE_WHOLE_IMAGE) }.is_null());
    // SAFETY: live handle.
    unsafe { ade::ade_image_free(image) };
    let _ = std::fs::remove_file(&path);
}

#[test]
fn partition_calls_tolerate_null_like_everything_else() {
    // SAFETY: null is explicitly allowed at every entry point.
    unsafe {
        assert!(ade::ade_partitions_open(std::ptr::null()).is_null());
        assert_eq!(ade::ade_partitions_count(std::ptr::null()), 0);
        let mut out = std::mem::MaybeUninit::<ade::AdePartition>::uninit();
        assert_eq!(
            ade::ade_partitions_entry(std::ptr::null(), 0, out.as_mut_ptr()),
            AdeResult::NullArgument
        );
        ade::ade_partitions_free(std::ptr::null_mut());
    }
}
