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
use ade_core::{Image, Inspection, examine, inspect_path};

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

/// An open image. Opaque to C.
pub struct AdeImage {
    bytes: Vec<u8>,
    inspection: Inspection,
    container: CString,
    absent: Option<CString>,
}

/// A directory listing, owned by the caller until freed.
pub struct AdeListing {
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
        let Ok(bytes) = std::fs::read(&path) else {
            set(AdeResult::Io);
            return std::ptr::null_mut();
        };
        let Ok(inspection) = inspect_path(&path) else {
            set(AdeResult::Io);
            return std::ptr::null_mut();
        };
        let container = CString::new(inspection.detection.kind.to_string()).unwrap_or_default();
        let absent = inspection
            .volume_absent
            .as_ref()
            .and_then(|s| CString::new(s.as_str()).ok());
        set(AdeResult::Ok);
        Box::into_raw(Box::new(AdeImage {
            bytes,
            inspection,
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
    with_image(image, 0, |i| examine(i.bytes.clone()).findings.len())
}

/// List a directory.
///
/// `block` is a root or directory block, from [`ade_image_root_block`] or an
/// [`AdeEntry`]. Returns null if the image has no volume or the block is not a
/// directory. Release with [`ade_listing_free`].
///
/// # Safety
/// `image` must be a live handle or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ade_dir_open(image: *const AdeImage, block: u32) -> *mut AdeListing {
    with_image(image, std::ptr::null_mut(), |image| {
        // The handle must outlive the volume, which borrows from it — hence
        // two bindings rather than a chain.
        let Ok(handle) = Image::from_bytes(image.bytes.clone()) else {
            return std::ptr::null_mut();
        };
        let Ok(volume) = handle.volume() else {
            return std::ptr::null_mut();
        };
        let Ok(listing) = volume.list(block) else {
            return std::ptr::null_mut();
        };

        // Names are copied out so they outlive the volume, which borrows the
        // image bytes and is dropped at the end of this call.
        let names: Vec<Vec<u8>> = listing.entries.iter().map(|e| e.name.clone()).collect();
        let entries = listing
            .entries
            .iter()
            .zip(names.iter())
            .map(|(entry, name)| AdeEntry {
                name: AdeBytes::of(name),
                block: entry.block,
                size: entry.byte_size,
                kind: entry.kind.into(),
                protection: entry.protection.0,
                days: entry.altered.days,
                mins: entry.altered.mins,
                ticks: entry.altered.ticks,
            })
            .collect();
        Box::into_raw(Box::new(AdeListing { names, entries }))
    })
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
pub unsafe extern "C" fn ade_file_read(image: *const AdeImage, block: u32) -> *mut AdeBuffer {
    with_image(image, std::ptr::null_mut(), |image| {
        let Ok(handle) = Image::from_bytes(image.bytes.clone()) else {
            return std::ptr::null_mut();
        };
        let Ok(volume) = handle.volume() else {
            return std::ptr::null_mut();
        };
        let Ok(entry) = volume.entry_at(block) else {
            return std::ptr::null_mut();
        };
        let Ok(contents) = volume.read_file(&entry) else {
            return std::ptr::null_mut();
        };
        Box::into_raw(Box::new(AdeBuffer {
            bytes: contents.into_bytes(),
        }))
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
