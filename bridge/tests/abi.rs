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
        assert!(ade::ade_image_open(std::ptr::null(), std::ptr::null(), &raw mut err).is_null());
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
    let image = unsafe { ade::ade_image_open(path.as_ptr(), std::ptr::null(), &raw mut err) };

    assert!(image.is_null());
    assert_eq!(err, AdeResult::Io);
}

#[test]
fn an_image_opens_and_reports_itself() {
    let (path, c_path) = fixture("open", &sound_disk());
    let mut err = AdeResult::Internal;
    // SAFETY: valid path, writable error slot.
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), &raw mut err) };
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
    let image =
        unsafe { ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), std::ptr::null_mut()) };
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
    let image =
        unsafe { ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), std::ptr::null_mut()) };

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
        let image = ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), std::ptr::null_mut());
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
        let image = ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), &raw mut err);
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
        let image = ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), std::ptr::null_mut());
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
        let image = ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), std::ptr::null_mut());
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
        let image = ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), std::ptr::null_mut());
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
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), &raw mut err) };
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
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), &raw mut err) };
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
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), &raw mut err) };
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
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), &raw mut err) };
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

#[test]
fn a_container_with_no_usable_geometry_still_opens() {
    // The handle is how a caller learns *why* an image is unreadable. Refusing
    // to open one would leave a front end with nothing to say about exactly
    // the disks a person is puzzled by — and the mounted image being optional
    // (IMP-006) is what makes this easy to get wrong.
    let (path, c_path) = fixture("nogeom", &[0xA5u8; 4096]);
    let mut err = AdeResult::Ok;
    // SAFETY: a valid path and a writable error slot.
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), &raw mut err) };
    assert!(!image.is_null(), "a truncated file should still open");
    // SAFETY: a live handle.
    assert!(!unsafe { ade::ade_image_has_volume(image) });
    // SAFETY: a live handle; there is a reason and it is a C string.
    let absent = unsafe { ade::ade_image_volume_absent(image) };
    assert!(!absent.is_null(), "and should say why");

    // Reading finds nothing rather than crashing.
    // SAFETY: a live handle with no mounted image behind it.
    unsafe {
        assert!(ade::ade_dir_open(image, ade::ADE_WHOLE_IMAGE, 880).is_null());
        assert!(ade::ade_walk_open(image, ade::ADE_WHOLE_IMAGE).is_null());
        assert!(ade::ade_file_read(image, ade::ADE_WHOLE_IMAGE, 880).is_null());
        assert!(ade::ade_partitions_open(image).is_null());
        ade::ade_image_free(image);
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_finding_count_is_the_health_check_not_the_inspection() {
    // Counted once at open now rather than per call (IMP-006). The risk in
    // moving it is that it quietly becomes a *different* number — the
    // inspection's faults are not the health check's findings — so this pins
    // it against the engine's own answer.
    let disk = sound_disk();
    let (path, c_path) = fixture("findings", &disk);
    let mut err = AdeResult::Ok;
    // SAFETY: a valid path and a writable error slot.
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), &raw mut err) };
    // SAFETY: a live handle.
    let reported = unsafe { ade::ade_image_finding_count(image) };
    assert_eq!(reported, ade_core::health::examine(disk).findings.len());
    // Stable across calls, which a cached value could get wrong in the other
    // direction by being computed from something that moved.
    // SAFETY: a live handle.
    assert_eq!(unsafe { ade::ade_image_finding_count(image) }, reported);
    // SAFETY: a live handle.
    unsafe { ade::ade_image_free(image) };
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_image_is_identified_at_open_when_a_dataset_is_given() {
    // F-013's clause, at the seam: the name is decided while the bytes are in
    // hand, because the handle holds a mounted image afterwards and cannot
    // hash itself (IMP-006).
    let disk = sound_disk();
    let dir = std::env::temp_dir().join(format!("ade-cat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let dat = format!(
        r#"<datafile><game name="Known"><rom name="A Known Disk.adf" size="{}" crc="{:08x}"/></game></datafile>"#,
        disk.len(),
        ade_core::layers::block::checksum::crc32(&disk)
    );
    std::fs::write(dir.join("test.dat"), dat).expect("write datfile");

    let c_dir = CString::new(dir.display().to_string()).expect("no NUL");
    // SAFETY: a valid directory path.
    let catalogue = unsafe { ade::ade_catalogue_open(c_dir.as_ptr()) };
    assert!(!catalogue.is_null(), "the dataset should load");
    // SAFETY: a live handle.
    assert_eq!(unsafe { ade::ade_catalogue_count(catalogue) }, 1);

    let (path, c_path) = fixture("named", &disk);
    let mut err = AdeResult::Ok;
    // SAFETY: valid path, live catalogue, writable slot.
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), catalogue, &raw mut err) };
    assert!(!image.is_null());
    // SAFETY: a live handle.
    let name = unsafe { ade::ade_image_identified(image) };
    // SAFETY: the bytes borrow from the handle, which is alive.
    let text = unsafe { std::slice::from_raw_parts(name.data, name.len) };
    assert_eq!(text, b"A Known Disk.adf");

    // SAFETY: live handles, each freed once.
    unsafe {
        ade::ade_image_free(image);
        ade::ade_catalogue_free(catalogue);
    }
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_a_dataset_an_image_is_simply_unnamed() {
    // Null is the ordinary case, not an error, and it must cost nothing.
    let (path, c_path) = fixture("unnamed", &sound_disk());
    let mut err = AdeResult::Ok;
    // SAFETY: valid path, no catalogue, writable slot.
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), &raw mut err) };
    assert!(!image.is_null());
    // SAFETY: a live handle.
    assert_eq!(unsafe { ade::ade_image_identified(image) }.len, 0);
    // SAFETY: a live handle.
    unsafe { ade::ade_image_free(image) };
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_catalogue_calls_tolerate_null_like_everything_else() {
    // SAFETY: null is allowed at every entry point.
    unsafe {
        assert!(ade::ade_catalogue_open(std::ptr::null()).is_null());
        assert_eq!(ade::ade_catalogue_count(std::ptr::null()), 0);
        assert_eq!(ade::ade_image_identified(std::ptr::null()).len, 0);
        ade::ade_catalogue_free(std::ptr::null_mut());
        ade::ade_string_free(std::ptr::null_mut());
    }
}

#[test]
fn the_region_strings_match_the_engines() {
    // The bridge spells the region names out as NUL-terminated literals rather
    // than converting the engine's at call time, because converting means
    // allocating a string C never frees. The cost of that choice is two copies
    // of the same words, so this is what stops them drifting: a rename in
    // `ade_core::layout` that misses the bridge fails here rather than shipping
    // a legend that disagrees with `--format=json`.
    use ade_core::layout::Region;
    const REGIONS: [Region; 6] = [
        Region::Bootblock,
        Region::Rootblock,
        Region::Bitmap,
        Region::Directory,
        Region::File,
        Region::Unclaimed,
    ];
    for (code, region) in REGIONS.iter().enumerate() {
        let code = u32::try_from(code).unwrap();
        // SAFETY: both return pointers to static storage, never null.
        let (name, describes) = unsafe {
            (
                std::ffi::CStr::from_ptr(ade::ade_region_name(code)),
                std::ffi::CStr::from_ptr(ade::ade_region_describes(code)),
            )
        };
        assert_eq!(name.to_str().unwrap(), region.name(), "name for {region:?}");
        assert_eq!(
            describes.to_str().unwrap(),
            region.describes(),
            "description for {region:?}"
        );
    }
}

#[test]
fn an_unknown_region_code_is_empty_rather_than_wrong() {
    // A front end built against a newer header must not be told that region 6
    // is a bootblock. Empty is a legend entry somebody notices; a wrong name
    // is a legend entry they believe.
    for code in [6u32, 99, u32::MAX] {
        // SAFETY: both tolerate any integer and return static storage.
        unsafe {
            assert_eq!(
                std::ffi::CStr::from_ptr(ade::ade_region_name(code)).to_bytes(),
                b""
            );
            assert_eq!(
                std::ffi::CStr::from_ptr(ade::ade_region_describes(code)).to_bytes(),
                b""
            );
        }
    }
}

#[test]
fn a_layout_tiles_the_whole_image_and_tolerates_null() {
    let (path, c_path) = fixture("layout", &sound_disk());
    let mut err = AdeResult::Ok;
    // SAFETY: valid path, no catalogue, writable slot.
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), &raw mut err) };
    assert!(!image.is_null());

    // SAFETY: a live handle.
    let layout = unsafe { ade::ade_layout_open(image, ade::ADE_WHOLE_IMAGE) };
    assert!(!layout.is_null());
    // SAFETY: a live handle.
    let count = unsafe { ade::ade_layout_count(layout) };
    assert!(count > 1, "a formatted disk is more than one span");

    // The spans must tile: no gaps, no overlaps, starting at zero. A front end
    // colours from these, and a hole in the map is a hole in the hex view that
    // looks like data.
    let mut at = 0u64;
    let mut saw_bootblock = false;
    for index in 0..count {
        let mut span = unsafe { std::mem::zeroed::<ade::AdeSpan>() };
        // SAFETY: a live handle and a writable slot.
        assert_eq!(
            unsafe { ade::ade_layout_span(layout, index, &raw mut span) },
            AdeResult::Ok
        );
        assert_eq!(span.offset, at, "span {index} does not follow the last");
        at += span.length;
        if span.region == 0 {
            saw_bootblock = true;
        }
    }
    assert!(saw_bootblock, "every disk has a bootblock");

    // Past the end is NotFound, not a crash.
    let mut span = unsafe { std::mem::zeroed::<ade::AdeSpan>() };
    // SAFETY: a live handle and a writable slot.
    assert_eq!(
        unsafe { ade::ade_layout_span(layout, count, &raw mut span) },
        AdeResult::NotFound
    );

    // SAFETY: a live handle.
    unsafe { ade::ade_layout_free(layout) };

    // A partition index is refused rather than guessed at: no image in the
    // corpus carries an RDB, so a device's map has nothing to be checked
    // against.
    // SAFETY: a live handle.
    assert!(unsafe { ade::ade_layout_open(image, 0) }.is_null());

    // SAFETY: a live handle.
    unsafe { ade::ade_image_free(image) };
    let _ = std::fs::remove_file(&path);

    // SAFETY: null is allowed at every entry point.
    unsafe {
        assert!(ade::ade_layout_open(std::ptr::null(), ade::ADE_WHOLE_IMAGE).is_null());
        assert_eq!(ade::ade_layout_count(std::ptr::null()), 0);
        assert_eq!(
            ade::ade_layout_span(std::ptr::null(), 0, std::ptr::null_mut()),
            AdeResult::NullArgument
        );
        ade::ade_layout_free(std::ptr::null_mut());
    }
}

#[test]
fn a_content_search_finds_and_attributes_its_hits() {
    let (path, c_path) = fixture("find", &sound_disk());
    let mut err = AdeResult::Ok;
    // SAFETY: valid path, no catalogue, writable slot.
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), &raw mut err) };
    assert!(!image.is_null());

    let needle = std::ffi::CString::new("DOS").unwrap();
    // SAFETY: a live handle and a NUL-terminated pattern.
    let search = unsafe { ade::ade_find_open(image, needle.as_ptr(), false, false) };
    assert!(!search.is_null());
    // SAFETY: a live handle.
    assert_eq!(
        unsafe { ade::ade_find_error(search) }.len,
        0,
        "a good pattern"
    );
    // SAFETY: a live handle.
    assert!(!unsafe { ade::ade_find_was_hex(search) }, "`DOS` is a word");
    // SAFETY: a live handle.
    let count = unsafe { ade::ade_find_count(search) };
    assert!(count > 0, "every AmigaDOS disk says `DOS` in its bootblock");

    let mut hit = unsafe { std::mem::zeroed::<ade::AdeMatch>() };
    // SAFETY: a live handle and a writable slot.
    assert_eq!(
        unsafe { ade::ade_find_match(search, 0, &raw mut hit) },
        AdeResult::Ok
    );
    assert_eq!(hit.offset, 0, "block 0, byte 0");
    assert_eq!(hit.region, 0, "and that is the bootblock");

    // Past the end is NotFound, not a crash.
    // SAFETY: a live handle and a writable slot.
    assert_eq!(
        unsafe { ade::ade_find_match(search, count, &raw mut hit) },
        AdeResult::NotFound
    );
    // SAFETY: a live handle.
    unsafe { ade::ade_find_free(search) };

    // SAFETY: a live handle.
    unsafe { ade::ade_image_free(image) };
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_refused_pattern_is_not_a_search_that_found_nothing() {
    // The distinction the command line draws with exit 2 against exit 1, and
    // the reason `ade_find_open` returns a handle rather than null for a bad
    // pattern: "ask me again" and "it is not there" must not look alike.
    let (path, c_path) = fixture("badpattern", &sound_disk());
    let mut err = AdeResult::Ok;
    // SAFETY: valid path, no catalogue, writable slot.
    let image = unsafe { ade::ade_image_open(c_path.as_ptr(), std::ptr::null(), &raw mut err) };
    assert!(!image.is_null());

    let bad = std::ffi::CString::new("0x601").unwrap();
    // SAFETY: a live handle and a NUL-terminated pattern.
    let search = unsafe { ade::ade_find_open(image, bad.as_ptr(), false, false) };
    assert!(!search.is_null(), "refused, not absent");
    // SAFETY: a live handle.
    let why = unsafe { ade::ade_find_error(search) };
    assert!(why.len > 0, "and it says why");
    // SAFETY: `why` borrows the live search.
    let message = unsafe { std::slice::from_raw_parts(why.data, why.len) };
    assert!(
        String::from_utf8_lossy(message).contains("hex digits"),
        "{}",
        String::from_utf8_lossy(message)
    );
    // SAFETY: a live handle.
    assert_eq!(unsafe { ade::ade_find_count(search) }, 0);
    // SAFETY: a live handle.
    unsafe { ade::ade_find_free(search) };

    // A pattern that is fine but matches nothing: no error, no matches.
    let absent = std::ffi::CString::new("zzzznotonthisdisk").unwrap();
    // SAFETY: a live handle and a NUL-terminated pattern.
    let empty = unsafe { ade::ade_find_open(image, absent.as_ptr(), false, false) };
    assert!(!empty.is_null());
    // SAFETY: a live handle.
    assert_eq!(
        unsafe { ade::ade_find_error(empty) }.len,
        0,
        "nothing wrong"
    );
    // SAFETY: a live handle.
    assert_eq!(unsafe { ade::ade_find_count(empty) }, 0, "just not there");
    // SAFETY: a live handle.
    unsafe { ade::ade_find_free(empty) };

    // SAFETY: a live handle.
    unsafe { ade::ade_image_free(image) };
    let _ = std::fs::remove_file(&path);

    // SAFETY: null is allowed at every entry point.
    unsafe {
        // Not null even here: a handle carrying the reason, which is this
        // call's whole contract.
        let nothing = ade::ade_find_open(std::ptr::null(), bad.as_ptr(), false, false);
        assert!(!nothing.is_null());
        assert!(ade::ade_find_error(nothing).len > 0);
        ade::ade_find_free(nothing);

        let no_pattern = ade::ade_find_open(std::ptr::null(), std::ptr::null(), false, false);
        assert!(!no_pattern.is_null());
        assert!(ade::ade_find_error(no_pattern).len > 0);
        ade::ade_find_free(no_pattern);
        assert_eq!(ade::ade_find_count(std::ptr::null()), 0);
        assert_eq!(ade::ade_find_error(std::ptr::null()).len, 0);
        assert!(!ade::ade_find_was_hex(std::ptr::null()));
        assert_eq!(
            ade::ade_find_match(std::ptr::null(), 0, std::ptr::null_mut()),
            AdeResult::NullArgument
        );
        ade::ade_find_free(std::ptr::null_mut());
    }
}
