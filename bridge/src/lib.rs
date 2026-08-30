//! The C ABI over ADE's core, for the Qt6 GUI (D-001, F-002).
//!
//! D-001 chose a Rust core with a C-ABI bridge and a Qt6 front end over it,
//! partly so no engine logic could end up in UI code (F-002) and partly
//! because memory safety serves the untrusted-input mandate (D-006). This
//! crate is that bridge, and it is the **only** place in ADE that writes
//! `unsafe`.
//!
//! # Three rules this file exists to keep
//!
//! **No panic may cross the boundary.** Unwinding into C is undefined
//! behaviour, so every entry point is wrapped in [`std::panic::catch_unwind`]
//! and returns a null or an error code instead. The workspace already forbids
//! panicking constructs in library code; this is the belt to that's braces,
//! because "should not panic" and "cannot unwind into C" are different claims.
//!
//! **Names are bytes, not C strings.** Amiga filenames are Latin-1 and
//! routinely hold bytes above 0x7F — `äpfel` is in the corpus. Handing those
//! out as `char*` would either lie about the encoding or mangle the name, so
//! anything that came off a disk is returned as an explicit pointer-and-length
//! [`AdeBytes`] and the caller decides how to decode it. Only ADE's own
//! diagnostics, which are ASCII by construction, are C strings.
//!
//! **The caller owns nothing it did not ask for.** Every pointer handed out is
//! either borrowed from a live handle and valid until that handle is freed, or
//! owned and freed by a named function. Each one says which.
//!
//! # Safety
//!
//! Every `unsafe` block here is dereferencing a pointer the caller supplied.
//! The contract is stated per function; in all cases a null pointer is handled
//! rather than dereferenced, because a C caller that has just had an error will
//! pass one.

use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use ade_core::layers::filesystem::entry::EntryKind;
use ade_core::layers::filesystem::volume::Volume;
use ade_core::{Image, Inspection, examine, inspect_bytes};

/// How a call turned out.
///
/// Deliberately flat integers rather than a Rust error type: the GUI switches
/// on these, and they are part of the ABI once released.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdeResult {
    /// The call succeeded.
    Ok = 0,
    /// A null pointer was passed where one was required.
    NullArgument = 1,
    /// The file could not be read.
    Io = 2,
    /// The container was recognised but holds no mountable volume.
    NoVolume = 3,
    /// A path or name was not valid UTF-8.
    BadEncoding = 4,
    /// The requested block or index does not exist.
    NotFound = 5,
    /// Something in the engine failed in a way the ABI has no code for.
    Internal = 6,
}

/// A borrowed run of bytes, with no encoding claimed.
///
/// Used for anything that came off a disk. `data` is valid for as long as the
/// handle it came from, and is never null when `len` is non-zero.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AdeBytes {
    /// Start of the run.
    pub data: *const u8,
    /// Length in bytes.
    pub len: usize,
}

impl AdeBytes {
    /// An empty run, which is what every failure returns.
    const fn empty() -> Self {
        Self {
            data: std::ptr::null(),
            len: 0,
        }
    }

    fn of(slice: &[u8]) -> Self {
        Self {
            data: slice.as_ptr(),
            len: slice.len(),
        }
    }
}

/// What kind of thing a directory entry is.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdeEntryKind {
    /// A file.
    File = 0,
    /// A directory.
    Directory = 1,
    /// A hard link to a file.
    LinkFile = 2,
    /// A hard link to a directory.
    LinkDir = 3,
    /// A soft link.
    SoftLink = 4,
    /// Something else, reported rather than guessed at.
    Unknown = 5,
}

impl From<EntryKind> for AdeEntryKind {
    fn from(kind: EntryKind) -> Self {
        match kind {
            EntryKind::File => Self::File,
            EntryKind::Root | EntryKind::Directory => Self::Directory,
            EntryKind::HardLinkFile => Self::LinkFile,
            EntryKind::HardLinkDir => Self::LinkDir,
            EntryKind::SoftLink => Self::SoftLink,
            EntryKind::Unknown(_) => Self::Unknown,
        }
    }
}

/// One directory entry, flattened for C.
///
/// `name` borrows from the [`AdeListing`] it came from.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AdeEntry {
    /// The name exactly as stored — Latin-1, not UTF-8.
    pub name: AdeBytes,
    /// The full path from the volume root, for entries from [`ade_walk_open`].
    ///
    /// Empty for entries from [`ade_dir_open`], which are already relative to
    /// the directory that was asked for.
    pub path: AdeBytes,
    /// The block the entry occupies, and the handle for descending into it.
    pub block: u32,
    /// File size in bytes; zero for a directory.
    pub size: u32,
    /// What this entry is.
    pub kind: AdeEntryKind,
    /// Protection flags, as stored.
    pub protection: u32,
    /// Days since 1978-01-01.
    pub days: u32,
    /// Minutes past midnight.
    pub mins: u32,
    /// Ticks past the minute, at 50 Hz.
    pub ticks: u32,
}

/// Passed as `partition` to mean "the image's own volume, not a partition".
///
/// A floppy has one volume and no partition table; a hard disk has a partition
/// table and no volume of its own. One selector covers both, which is why the
/// reading calls take it rather than coming in two families — a device is not
/// a special case of an image, it is what an image is when it has an RDB.
pub const ADE_WHOLE_IMAGE: u32 = u32::MAX;

/// One partition of a device.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AdePartition {
    /// The drive name, `DH0`. Latin-1 bytes, like every other name.
    pub name: AdeBytes,
    /// The volume's own name, empty when the partition does not mount.
    pub volume_name: AdeBytes,
    /// The dostype longword, unmodified.
    pub dostype: u32,
    /// First block of the partition on the device.
    pub first_block: u32,
    /// Blocks the partition spans.
    pub blocks: u32,
    /// Bytes per block. Usually 512, and not always.
    pub block_size: u32,
    /// Blocks reserved at the front, which fixes where the rootblock sits.
    pub reserved: u32,
    /// The rootblock, relative to the partition. Zero when it does not mount.
    pub root_block: u32,
    /// Whether the partition is flagged bootable.
    pub bootable: bool,
    /// Whether an AmigaDOS volume actually mounts inside it.
    ///
    /// Separate from `bootable`, and the more useful of the two: a partition
    /// can be flagged bootable and hold nothing, or hold a perfectly good
    /// volume and not be bootable. It can also be a `PFS\0` or `SFS\0`
    /// partition, which is a real partition ADE cannot read.
    pub mounts: bool,
}

/// A loaded dataset, owned by the caller until freed.
///
/// Held rather than reloaded per image, because loading 88,921 entries takes
/// 140 ms and a front end opens many images in one session: paid once at
/// startup, spent on every disk after (F-013).
pub struct AdeCatalogue {
    inner: ade_core::layers::catalogue::Catalogue,
}

/// A device's partition table, owned by the caller until freed.
pub struct AdePartitions {
    /// Owns the name bytes the entries point into — same role as
    /// [`AdeListing::names`], and the same reason it must not be removed.
    #[allow(dead_code, reason = "keeps the name buffers alive for `entries`")]
    names: Vec<Vec<u8>>,
    entries: Vec<AdePartition>,
}

/// An open image. Opaque to C.
///
/// # Everything expensive is done once, at open
///
/// The mounted [`Image`] is held rather than the raw bytes, and the health
/// count is computed here rather than on demand. Both were per-call before
/// (IMP-006), and the cost was not the copy: `Image::from_bytes` reassembles
/// the container, so every directory expansion re-decoded 160 tracks of an SCP
/// or re-inflated an ADZ. Measured at ~131 ms per interaction with 30 MB
/// captures open, against ~16 ms on plain ADFs.
pub struct AdeImage {
    /// The mounted image. Volumes borrow from this, which is why it is stored
    /// rather than rebuilt: a `Volume` cannot outlive the `Image` it came
    /// from, and rebuilding per call was the way to sidestep that.
    ///
    /// `None` when the bytes match no usable geometry — a truncated file, or a
    /// container ADE does not read. **The handle still opens**, because the
    /// container and the reason are worth having and are the whole point of
    /// opening such a file: a quarter of real images hold no AmigaDOS volume,
    /// and a front end that could not open them would be refusing to describe
    /// exactly the disks a person is puzzled by.
    image: Option<Image>,
    inspection: Inspection,
    /// What a dataset called this image, when one was supplied at open.
    identified: Option<String>,
    /// Findings from a full health check, counted at open.
    ///
    /// `examine` walks the whole volume and cross-checks the bitmap, so
    /// running it per call — which is what a badge in a GUI asks for — cost
    /// more than everything else the window did.
    findings: usize,
    container: CString,
    absent: Option<CString>,
}

/// A directory listing or a whole-volume walk, owned by the caller until freed.
pub struct AdeListing {
    /// The path bytes each entry's `path` points into, when this came from a
    /// walk. Same ownership role as `names`.
    #[allow(dead_code, reason = "keeps the path buffers alive for `entries`")]
    paths: Vec<Vec<u8>>,
    /// The name bytes each entry's [`AdeBytes`] points into.
    ///
    /// **Never read, and must not be removed.** It is the owner of the
    /// allocations that `entries[..].name` borrows: dropping it would leave
    /// every name in the listing dangling. `dead_code` is right that nothing
    /// reads it and wrong about what that means, which is exactly the kind of
    /// lint a C ABI attracts.
    #[allow(dead_code, reason = "keeps the name buffers alive for `entries`")]
    names: Vec<Vec<u8>>,
    entries: Vec<AdeEntry>,
}

/// A file's contents, owned by the caller until freed.
pub struct AdeBuffer {
    bytes: Vec<u8>,
}

/// Everything the reading calls need to reach a volume.
///
/// The three functions that read — a directory, a walk, a file — each need an
/// `Image` that outlives the `Volume` borrowed from it, and each may be asked
/// for a partition instead of the image's own volume. Written once here so the
/// partition lookup cannot drift between them: a device whose third partition
/// listed differently from the one it extracted would be a bad way to find out
/// they were three copies of the same logic.
///
/// Returns null-equivalent (`None`) rather than an error, because every caller
/// of it returns a null pointer on failure and C has no other channel here.
fn with_volume<T>(
    image: &AdeImage,
    partition: u32,
    body: impl FnOnce(&Volume<'_>) -> Option<T>,
) -> Option<T> {
    let handle = image.image.as_ref()?;
    if partition == ADE_WHOLE_IMAGE {
        let volume = handle.volume().ok()?;
        return body(&volume);
    }
    let (partitions, _faults) = handle.partitions().ok()?;
    let chosen = partitions.get(partition as usize)?;
    // The window borrows the image and the volume borrows the window, so both
    // are bound here rather than chained: a temporary window would be dropped
    // while the volume still pointed into it.
    let window = handle.partition_window(chosen).ok()?;
    let volume = Volume::mount(&window).ok()?;
    body(&volume)
}

/// Run `body`, turning any panic into `fallback`.
///
/// Unwinding into C is undefined behaviour. Nothing here is expected to panic —
/// the workspace denies the constructs that would — but "expected not to" is
/// not the same as "cannot".
fn guard<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(fallback)
}

/// ADE's version string, NUL-terminated ASCII and valid for the program's life.
#[unsafe(no_mangle)]
pub extern "C" fn ade_version() -> *const c_char {
    // A leaked allocation, once, so the pointer is valid forever and the caller
    // never has to free it.
    static VERSION: std::sync::OnceLock<CString> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| CString::new(ade_core::version()).unwrap_or_default())
        .as_ptr()
}

/// Open an image.
///
/// Returns null on failure and writes the reason to `out_err` when that is not
/// null. The handle must be released with [`ade_image_free`].
///
/// # Safety
/// `path` must be a valid NUL-terminated string, and `out_err` must be null or
/// point to a writable [`AdeResult`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_image_open(
    path: *const c_char,
    catalogue: *const AdeCatalogue,
    out_err: *mut AdeResult,
) -> *mut AdeImage {
    let set = |code: AdeResult| {
        if !out_err.is_null() {
            // SAFETY: checked non-null; the caller promises it is writable.
            unsafe { *out_err = code };
        }
    };
    if path.is_null() {
        set(AdeResult::NullArgument);
        return std::ptr::null_mut();
    }
    // SAFETY: checked non-null; the caller promises NUL termination.
    let raw = unsafe { CStr::from_ptr(path) };
    let Ok(text) = raw.to_str() else {
        set(AdeResult::BadEncoding);
        return std::ptr::null_mut();
    };
    let path = PathBuf::from(text);

    guard(std::ptr::null_mut(), || {
        // Read once. `inspect_path` would read the file a second time.
        let Ok(bytes) = std::fs::read(&path) else {
            set(AdeResult::Io);
            return std::ptr::null_mut();
        };
        // Identification on open (F-013), from the bytes already read. Null
        // is the ordinary case: most callers have no dataset, and it costs
        // them nothing.
        // SAFETY: the caller's contract; null is checked by `as_ref`.
        let identified = unsafe { catalogue.as_ref() }.and_then(|c| {
            c.inner
                .identify(&bytes)
                .first()
                .map(|entry| entry.name.clone())
        });
        let inspection = inspect_bytes(bytes.clone());
        let findings = examine(bytes.clone()).findings.len();
        // A container ADE cannot mount still gets a handle: the caller wants
        // the container and the reason. The reading calls simply find nothing.
        //
        // Opened **lazily** (IMP-005): a front end holds every image it opens,
        // and holding the bytes is the whole cost — 400 floppies is 400 MB.
        // Blocks come from the file instead, and a container whose blocks are
        // not its file falls back to reading whole on its own.
        drop(bytes);
        let handle = Image::open_lazy(&path).ok();
        let container = CString::new(inspection.detection.kind.to_string()).unwrap_or_default();
        let absent = inspection
            .volume_absent
            .as_ref()
            .and_then(|s| CString::new(s.as_str()).ok());
        set(AdeResult::Ok);
        Box::into_raw(Box::new(AdeImage {
            image: handle,
            identified,
            inspection,
            findings,
            container,
            absent,
        }))
    })
}

/// Release an image opened by [`ade_image_open`].
///
/// # Safety
/// `image` must be a pointer from [`ade_image_open`] that has not been freed.
/// Passing null is allowed and does nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_image_free(image: *mut AdeImage) {
    if image.is_null() {
        return;
    }
    guard((), || {
        // SAFETY: checked non-null; the caller promises it came from
        // `ade_image_open` and has not already been freed.
        drop(unsafe { Box::from_raw(image) });
    });
}

/// Borrow an image handle, or return `fallback` if it is null.
fn with_image<T>(image: *const AdeImage, fallback: T, body: impl FnOnce(&AdeImage) -> T) -> T {
    if image.is_null() {
        return fallback;
    }
    // SAFETY: checked non-null; the caller promises a live handle.
    let image = unsafe { &*image };
    guard(fallback, || body(image))
}

/// The container ADE identified, as NUL-terminated ASCII.
///
/// Borrowed from the handle; valid until it is freed.
///
/// # Safety
/// `image` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_image_container(image: *const AdeImage) -> *const c_char {
    with_image(image, std::ptr::null(), |i| i.container.as_ptr())
}

/// Why no volume was found, as NUL-terminated ASCII, or null if one was.
///
/// # Safety
/// `image` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_image_volume_absent(image: *const AdeImage) -> *const c_char {
    with_image(image, std::ptr::null(), |i| {
        i.absent.as_ref().map_or(std::ptr::null(), |s| s.as_ptr())
    })
}

/// The image's size in bytes.
///
/// # Safety
/// `image` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_image_size(image: *const AdeImage) -> u64 {
    with_image(image, 0, |i| i.inspection.size)
}

/// Whether the image holds a mountable volume.
///
/// # Safety
/// `image` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_image_has_volume(image: *const AdeImage) -> bool {
    with_image(image, false, |i| i.inspection.volume.is_some())
}

/// The volume label, exactly as stored — Latin-1, not UTF-8.
///
/// Borrowed from the handle. Empty when there is no volume.
///
/// # Safety
/// `image` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_image_volume_name(image: *const AdeImage) -> AdeBytes {
    with_image(image, AdeBytes::empty(), |i| {
        i.inspection
            .volume
            .as_ref()
            .map_or(AdeBytes::empty(), |v| AdeBytes::of(&v.rootblock.name))
    })
}

/// The root directory's block number, for use with [`ade_dir_open`].
///
/// # Safety
/// `image` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_image_root_block(image: *const AdeImage) -> u32 {
    with_image(image, 0, |i| {
        i.inspection
            .volume
            .as_ref()
            .map_or(0, |v| u32::try_from(v.rootblock_at).unwrap_or(0))
    })
}

/// How many findings a health check reports.
///
/// A count rather than the findings themselves: the GUI's first use is a
/// badge, and the detail can follow when there is a panel to put it in.
///
/// # Safety
/// `image` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_image_finding_count(image: *const AdeImage) -> usize {
    with_image(image, 0, |i| i.findings)
}

/// List a directory.
///
/// `block` is a root or directory block, from [`ade_image_root_block`], an
/// [`AdePartition`], or an [`AdeEntry`]. `partition` selects which volume the
/// block belongs to: an index from [`ade_partitions_open`], or
/// [`ADE_WHOLE_IMAGE`] for a floppy or hardfile that holds its own volume.
///
/// Returns null if there is no such volume or the block is not a directory.
/// Release with [`ade_listing_free`].
///
/// # Safety
/// `image` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_dir_open(
    image: *const AdeImage,
    partition: u32,
    block: u32,
) -> *mut AdeListing {
    with_image(image, std::ptr::null_mut(), |image| {
        let found = with_volume(image, partition, |volume| {
            let listing = volume.list(block).ok()?;

            // Names are copied out so they outlive the volume, which borrows
            // the image bytes and is dropped at the end of this call.
            let names: Vec<Vec<u8>> = listing.entries.iter().map(|e| e.name.clone()).collect();
            let entries = listing
                .entries
                .iter()
                .zip(names.iter())
                .map(|(entry, name)| AdeEntry {
                    name: AdeBytes::of(name),
                    path: AdeBytes::empty(),
                    block: entry.block,
                    size: entry.byte_size,
                    kind: entry.kind.into(),
                    protection: entry.protection.0,
                    days: entry.altered.days,
                    mins: entry.altered.mins,
                    ticks: entry.altered.ticks,
                })
                .collect();
            Some(AdeListing {
                names,
                paths: Vec::new(),
                entries,
            })
        });
        found.map_or(std::ptr::null_mut(), |listing| {
            Box::into_raw(Box::new(listing))
        })
    })
}

/// Every entry on the volume, flattened, with full paths.
///
/// This exists so a front end never has to write its own traversal. Walking an
/// Amiga volume safely is engine logic, not UI logic: a hard link to a
/// directory makes cycles reachable on an *uncorrupted* disk (AV-001), and the
/// engine's walk carries both a visited set and an explicit depth bound —
/// the latter because a cycle grows the path *strings* without bound even
/// while the entry count stays inside its cap (IMP-003). A recursive
/// `ade_dir_open` in C++ would have neither.
///
/// Release with [`ade_listing_free`].
///
/// # Safety
/// `image` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_walk_open(image: *const AdeImage, partition: u32) -> *mut AdeListing {
    with_image(image, std::ptr::null_mut(), |image| {
        let found = with_volume(image, partition, |volume| {
            let walk = volume.walk(volume.root()).ok()?;

            let names: Vec<Vec<u8>> = walk.entries.iter().map(|(_, e)| e.name.clone()).collect();
            let paths: Vec<Vec<u8>> = walk
                .entries
                .iter()
                .map(|(path, _)| path.as_bytes().to_vec())
                .collect();
            let entries = walk
                .entries
                .iter()
                .zip(names.iter())
                .zip(paths.iter())
                .map(|(((_, entry), name), path)| AdeEntry {
                    name: AdeBytes::of(name),
                    path: AdeBytes::of(path),
                    block: entry.block,
                    size: entry.byte_size,
                    kind: entry.kind.into(),
                    protection: entry.protection.0,
                    days: entry.altered.days,
                    mins: entry.altered.mins,
                    ticks: entry.altered.ticks,
                })
                .collect();
            Some(AdeListing {
                paths,
                names,
                entries,
            })
        });
        found.map_or(std::ptr::null_mut(), |listing| {
            Box::into_raw(Box::new(listing))
        })
    })
}

/// Load every datfile in a directory.
///
/// Returns null when the directory holds none, or cannot be read. Release with
/// [`ade_catalogue_free`].
///
/// # Safety
/// `dir` must be a NUL-terminated path, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_catalogue_open(dir: *const c_char) -> *mut AdeCatalogue {
    guard(std::ptr::null_mut(), || {
        if dir.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: checked non-null; the caller promises NUL termination.
        let raw = unsafe { CStr::from_ptr(dir) };
        let Ok(text) = raw.to_str() else {
            return std::ptr::null_mut();
        };
        match ade_core::layers::catalogue::Catalogue::load_dir(&PathBuf::from(text)) {
            Ok(inner) => Box::into_raw(Box::new(AdeCatalogue { inner })),
            Err(_) => std::ptr::null_mut(),
        }
    })
}

/// Where a dataset lives when the front end was not told.
///
/// Checks `$ADE_DATFILES` then the conventional data directory, and returns
/// null when neither exists — which is the ordinary case and not an error.
/// The returned string is owned by the caller and must be freed with
/// [`ade_string_free`].
#[unsafe(no_mangle)]
pub extern "C" fn ade_datfiles_location() -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let Some(dir) = ade_core::datfiles_location(None) else {
            return std::ptr::null_mut();
        };
        CString::new(dir.display().to_string())
            .map_or(std::ptr::null_mut(), std::ffi::CString::into_raw)
    })
}

/// Release a string this library allocated.
///
/// # Safety
/// `text` must have come from a call documented as returning an owned string,
/// or be null. Freeing twice is undefined.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_string_free(text: *mut c_char) {
    guard((), || {
        if !text.is_null() {
            // SAFETY: the caller's contract.
            drop(unsafe { CString::from_raw(text) });
        }
    });
}

/// How many entries a dataset holds.
///
/// # Safety
/// `catalogue` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_catalogue_count(catalogue: *const AdeCatalogue) -> usize {
    guard(0, || {
        // SAFETY: the caller's contract; null is checked.
        unsafe { catalogue.as_ref() }.map_or(0, |c| c.inner.len())
    })
}

/// Release a dataset.
///
/// # Safety
/// `catalogue` must come from [`ade_catalogue_open`], or be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_catalogue_free(catalogue: *mut AdeCatalogue) {
    guard((), || {
        if !catalogue.is_null() {
            // SAFETY: the caller's contract.
            drop(unsafe { Box::from_raw(catalogue) });
        }
    });
}

/// What a dataset called this image, from [`ade_image_open`].
///
/// Empty when no catalogue was supplied at open, or when the dataset does not
/// hold this image. The bytes borrow from the handle.
///
/// # Why this is decided at open and not asked later
///
/// The handle holds a mounted image, not the file's bytes (IMP-006), so it
/// cannot hash itself after the fact. That suits F-013, which asks for
/// identification **on open** rather than on demand: the bytes are in hand
/// exactly once, and that is when the question gets answered.
///
/// # Safety
/// `image` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_image_identified(image: *const AdeImage) -> AdeBytes {
    with_image(image, AdeBytes::empty(), |i| {
        i.identified
            .as_ref()
            .map_or(AdeBytes::empty(), |name| AdeBytes::of(name.as_bytes()))
    })
}

/// The device's partition table.
///
/// Returns null when the image has no Rigid Disk Block, which is most images —
/// a floppy has no partition table and that is not a fault. Release with
/// [`ade_partitions_free`].
///
/// # Why a partition is not just an offset
///
/// It would be tempting for a front end to take `first_block` and read from
/// there. It must not: a partition carries its own block size and its own
/// reserved-block count, and the rootblock is computed from both (C-007). A
/// partition with four reserved blocks instead of two puts its rootblock
/// somewhere a caller assuming the usual layout will not find it. `root_block`
/// here is the computed answer, and the reading calls take a partition index
/// so they can do the same computation rather than trusting an offset.
///
/// # Safety
/// `image` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_partitions_open(image: *const AdeImage) -> *mut AdePartitions {
    with_image(image, std::ptr::null_mut(), |image| {
        let Some(handle) = image.image.as_ref() else {
            return std::ptr::null_mut();
        };
        // A broken chain yields the partitions found before it rather than
        // nothing: half a partition table is still worth browsing.
        let Ok((partitions, _faults)) = handle.partitions() else {
            return std::ptr::null_mut();
        };
        if partitions.is_empty() {
            return std::ptr::null_mut();
        }

        let mut names: Vec<Vec<u8>> = Vec::with_capacity(partitions.len().saturating_mul(2));
        let mut entries = Vec::with_capacity(partitions.len());
        for partition in &partitions {
            // Whether it mounts is worth more than whether it is flagged
            // bootable, and only mounting it answers that. A `PFS\0` partition
            // is a real partition ADE cannot read, and saying so beats an
            // empty listing.
            let mounted = handle.partition_window(partition).ok().and_then(|window| {
                Volume::mount(&window)
                    .ok()
                    .map(|v| (v.rootblock().name.clone(), v.root()))
            });

            names.push(partition.name.clone());
            let name_index = names.len().saturating_sub(1);
            names.push(mounted.as_ref().map(|(n, _)| n.clone()).unwrap_or_default());
            let volume_index = names.len().saturating_sub(1);

            entries.push(AdePartition {
                name: AdeBytes::of(names.get(name_index).map_or(&[][..], Vec::as_slice)),
                volume_name: AdeBytes::of(names.get(volume_index).map_or(&[][..], Vec::as_slice)),
                dostype: partition.dostype,
                first_block: u32::try_from(partition.first_block()).unwrap_or(0),
                blocks: u32::try_from(partition.block_count()).unwrap_or(0),
                block_size: partition.block_size,
                reserved: partition.reserved,
                root_block: mounted.as_ref().map_or(0, |(_, root)| *root),
                bootable: partition.bootable,
                mounts: mounted.is_some(),
            });
        }
        Box::into_raw(Box::new(AdePartitions { names, entries }))
    })
}

/// A map of a whole image. Opaque to C.
pub struct AdeLayout {
    /// Owns the owner strings the spans point into — same role as
    /// [`AdeListing::names`], and the same reason it must not be removed.
    #[allow(dead_code, reason = "keeps the owner buffers alive for `spans`")]
    owners: Vec<Vec<u8>>,
    spans: Vec<AdeSpan>,
}

/// One run of blocks that are all the same thing. Mirrors `AdeSpan` in `ade.h`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AdeSpan {
    /// First byte.
    pub offset: u64,
    /// How many bytes.
    pub length: u64,
    /// First block.
    pub block: u64,
    /// How many blocks.
    pub blocks: u64,
    /// What it is.
    pub region: u32,
    /// The owning path, Latin-1; empty when nothing owns it.
    pub owner: AdeBytes,
}

/// Map what occupies every block of an image (F-022).
///
/// Only [`ADE_WHOLE_IMAGE`] is mapped: a partition index returns null. A
/// device's map would place several volumes, each with its own block size, at
/// absolute offsets, and no image in the corpus carries an RDB — there is
/// nothing to check such a map against, so it is refused rather than guessed.
///
/// Release with [`ade_layout_free`].
///
/// # Safety
/// `image` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_layout_open(image: *const AdeImage, partition: u32) -> *mut AdeLayout {
    with_image(image, std::ptr::null_mut(), |image| {
        if partition != ADE_WHOLE_IMAGE {
            return std::ptr::null_mut();
        }
        let Some(handle) = image.image.as_ref() else {
            return std::ptr::null_mut();
        };
        let map = ade_core::layout::Layout::of(handle);

        let mut owners: Vec<Vec<u8>> = Vec::with_capacity(map.spans.len());
        let mut spans = Vec::with_capacity(map.spans.len());
        for span in &map.spans {
            owners.push(span.owner.clone().unwrap_or_default().into_bytes());
            let at = owners.len().saturating_sub(1);
            spans.push(AdeSpan {
                offset: span.start,
                length: span.end.saturating_sub(span.start),
                block: span.block,
                blocks: span.blocks,
                region: span.region as u32,
                owner: AdeBytes::of(owners.get(at).map_or(&[][..], Vec::as_slice)),
            });
        }
        Box::into_raw(Box::new(AdeLayout { owners, spans }))
    })
}

/// How many spans the map holds.
///
/// # Safety
/// `layout` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_layout_count(layout: *const AdeLayout) -> usize {
    guard(0, || {
        // SAFETY: the caller's contract; null is checked.
        unsafe { layout.as_ref() }.map_or(0, |l| l.spans.len())
    })
}

/// Copy span `index` into `*out`.
///
/// # Safety
/// `layout` must be a live handle or null, and `out` writable or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_layout_span(
    layout: *const AdeLayout,
    index: usize,
    out: *mut AdeSpan,
) -> AdeResult {
    guard(AdeResult::Internal, || {
        // SAFETY: the caller's contract; both are checked.
        let (Some(layout), Some(out)) = (unsafe { layout.as_ref() }, unsafe { out.as_mut() })
        else {
            return AdeResult::NullArgument;
        };
        let Some(span) = layout.spans.get(index) else {
            return AdeResult::NotFound;
        };
        *out = *span;
        AdeResult::Ok
    })
}

/// Release a map.
///
/// # Safety
/// `layout` must have come from [`ade_layout_open`], or be null, and must not
/// be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_layout_free(layout: *mut AdeLayout) {
    guard((), || {
        if !layout.is_null() {
            // SAFETY: the caller's contract.
            drop(unsafe { Box::from_raw(layout) });
        }
    });
}

/// A region's short name, for a legend. Static; never freed.
///
/// # Safety
/// None: takes an integer and returns a pointer to static storage.
#[unsafe(no_mangle)]
pub extern "C" fn ade_region_name(region: u32) -> *const c_char {
    region_text(region, &REGION_NAMES)
}

/// A region's one-line description, for a legend. Static; never freed.
///
/// # Safety
/// None: takes an integer and returns a pointer to static storage.
#[unsafe(no_mangle)]
pub extern "C" fn ade_region_describes(region: u32) -> *const c_char {
    region_text(region, &REGION_DESCRIPTIONS)
}

/// The region names, NUL-terminated, in `AdeRegion` order.
///
/// Spelled out here rather than converted from [`ade_core::layout::Region`] at
/// call time, because converting means allocating and a call that allocates a
/// string it never frees is a leak the caller cannot see. `the_region_strings_
/// match_the_engines` pins these against the engine's, so the duplication
/// cannot drift into two different names for one thing.
static REGION_NAMES: [&CStr; 6] = [
    c"bootblock",
    c"rootblock",
    c"bitmap",
    c"directory",
    c"file",
    c"unclaimed",
];

/// The region descriptions, NUL-terminated, in `AdeRegion` order.
static REGION_DESCRIPTIONS: [&CStr; 6] = [
    c"boot code and the dostype — where protection lives",
    c"the volume's name, datestamps and hash table",
    c"which blocks are free; a set bit means free",
    c"a directory header, holding its name",
    c"a file's header or its data",
    c"nothing points here: free space, deleted data, or damage",
];

/// One of a static table, or an empty string for a code this build does not
/// know — never a wrong name. A front end built against a newer header must
/// not be told that region 6 is a bootblock.
fn region_text(region: u32, table: &'static [&'static CStr; 6]) -> *const c_char {
    guard(c"".as_ptr(), || {
        usize::try_from(region)
            .ok()
            .and_then(|i| table.get(i))
            .map_or(c"".as_ptr(), |text| text.as_ptr())
    })
}

/// How many partitions the table holds.
///
/// # Safety
/// `partitions` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_partitions_count(partitions: *const AdePartitions) -> usize {
    guard(0, || {
        // SAFETY: the caller's contract; null is checked.
        unsafe { partitions.as_ref() }.map_or(0, |p| p.entries.len())
    })
}

/// Copy one partition out of the table.
///
/// # Safety
/// `partitions` must be a live handle or null, and `out` writable or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_partitions_entry(
    partitions: *const AdePartitions,
    index: usize,
    out: *mut AdePartition,
) -> AdeResult {
    guard(AdeResult::Internal, || {
        // SAFETY: the caller's contract; both are checked for null.
        let Some(table) = (unsafe { partitions.as_ref() }) else {
            return AdeResult::NullArgument;
        };
        let Some(slot) = (unsafe { out.as_mut() }) else {
            return AdeResult::NullArgument;
        };
        let Some(entry) = table.entries.get(index) else {
            return AdeResult::NotFound;
        };
        *slot = *entry;
        AdeResult::Ok
    })
}

/// Release a partition table.
///
/// # Safety
/// `partitions` must come from [`ade_partitions_open`], or be null. Freeing
/// twice is undefined.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_partitions_free(partitions: *mut AdePartitions) {
    guard((), || {
        if !partitions.is_null() {
            // SAFETY: the caller's contract.
            drop(unsafe { Box::from_raw(partitions) });
        }
    });
}

/// How many entries a listing holds.
///
/// # Safety
/// `listing` must be a live listing or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_listing_count(listing: *const AdeListing) -> usize {
    if listing.is_null() {
        return 0;
    }
    // SAFETY: checked non-null; the caller promises a live listing.
    let listing = unsafe { &*listing };
    listing.entries.len()
}

/// Copy one entry out of a listing.
///
/// Returns [`AdeResult::NotFound`] if `index` is past the end. The `name` in
/// the entry borrows from the listing and is valid until it is freed.
///
/// # Safety
/// `listing` must be a live listing or null, and `out` must be null or point
/// to a writable [`AdeEntry`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_listing_entry(
    listing: *const AdeListing,
    index: usize,
    out: *mut AdeEntry,
) -> AdeResult {
    if listing.is_null() || out.is_null() {
        return AdeResult::NullArgument;
    }
    // SAFETY: checked non-null; the caller promises a live listing.
    let listing = unsafe { &*listing };
    let Some(entry) = listing.entries.get(index) else {
        return AdeResult::NotFound;
    };
    // SAFETY: checked non-null; the caller promises it is writable.
    unsafe { *out = *entry };
    AdeResult::Ok
}

/// Release a listing from [`ade_dir_open`].
///
/// # Safety
/// `listing` must come from [`ade_dir_open`] and not have been freed. Null is
/// allowed and does nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_listing_free(listing: *mut AdeListing) {
    if listing.is_null() {
        return;
    }
    guard((), || {
        // SAFETY: checked non-null; the caller promises provenance.
        drop(unsafe { Box::from_raw(listing) });
    });
}

/// Read a file's contents by its entry block.
///
/// Returns null if there is no volume, the block is not a file, or it cannot
/// be read. Release with [`ade_buffer_free`].
///
/// # Safety
/// `image` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_file_read(
    image: *const AdeImage,
    partition: u32,
    block: u32,
) -> *mut AdeBuffer {
    with_image(image, std::ptr::null_mut(), |image| {
        let found = with_volume(image, partition, |volume| {
            let entry = volume.entry_at(block).ok()?;
            Some(volume.read_file(&entry).ok()?.into_bytes())
        });
        found.map_or(std::ptr::null_mut(), |bytes| {
            Box::into_raw(Box::new(AdeBuffer { bytes }))
        })
    })
}

/// Read raw bytes of the mounted image, for a hex view of the disk itself.
///
/// Offsets are in the space [`ade_layout_open`] maps and [`ade_image_size`]
/// counts: the image as it mounts, not the file as it sits on disk. A short
/// read at the end is not an error, and past the end is an empty buffer rather
/// than null — a caller scrolling off the bottom gets nothing, not a failure.
///
/// Release with [`ade_buffer_free`].
///
/// # Safety
/// `image` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_image_read(
    image: *const AdeImage,
    offset: u64,
    length: u64,
) -> *mut AdeBuffer {
    with_image(image, std::ptr::null_mut(), |image| {
        let bytes = image
            .image
            .as_ref()
            .map(|handle| handle.read_range(offset, length))
            .unwrap_or_default();
        Box::into_raw(Box::new(AdeBuffer { bytes }))
    })
}

/// The bytes a buffer holds, borrowed until it is freed.
///
/// # Safety
/// `buffer` must be a live buffer or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_buffer_bytes(buffer: *const AdeBuffer) -> AdeBytes {
    if buffer.is_null() {
        return AdeBytes::empty();
    }
    // SAFETY: checked non-null; the caller promises a live buffer.
    let buffer = unsafe { &*buffer };
    AdeBytes::of(&buffer.bytes)
}

/// Release a buffer from [`ade_file_read`].
///
/// # Safety
/// `buffer` must come from [`ade_file_read`] and not have been freed. Null is
/// allowed and does nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_buffer_free(buffer: *mut AdeBuffer) {
    if buffer.is_null() {
        return;
    }
    guard((), || {
        // SAFETY: checked non-null; the caller promises provenance.
        drop(unsafe { Box::from_raw(buffer) });
    });
}
