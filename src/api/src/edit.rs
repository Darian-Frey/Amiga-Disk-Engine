//! Editing a disk, and saving the result (D-018).
//!
//! The filesystem layer knows how to place a directory or a file on a volume
//! ([`ade_filesystem::edit`]). This is everything around that a front end
//! would otherwise have to get right for itself, and every part of it is one
//! of D-018's four terms:
//!
//! 1. **Edits are buffered.** A [`Session`] holds the image in memory. An edit
//!    changes that and nothing else, so closing without saving costs nothing,
//!    and undo is a stack of earlier copies rather than a promise about a
//!    file.
//! 2. **A save over the file the image was opened from is a separate,
//!    confirmed call** — [`Session::save_over_original`] — which keeps the
//!    original beside it as `.orig` the first time (D-020, amending D-018's
//!    "never"). [`Session::save_as`] still refuses that file, and any other
//!    existing file the session did not make: the majority behaviour IMP-010
//!    records, and the one that matches a preservation tool.
//! 3. **A save is atomic.** A temporary file beside the destination, flushed,
//!    then renamed over it. A process killed halfway leaves the old file or
//!    the new one, never a torn one.
//! 4. **An edit must leave a disk ADE accepts.** Each one is checked with the
//!    health report before it is kept: if the result has a warning or error
//!    the disk did not already have, the edit is undone and refused. The other
//!    half of part four — ADFlib mounting the result — is the test suite's,
//!    since D-002 forbids linking the oracle into anything that ships.
//!
//! # What can be edited
//!
//! Plain ADFs and unpartitioned hardfiles: images whose file *is* their
//! blocks, so what is saved is the same kind of file that was opened. A
//! compressed or flux container would have to be saved as something other
//! than what it was, and a partitioned device needs a partition chosen; both
//! are refused with the reason rather than quietly converted.

use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use ade_block::Geometry;
use ade_filesystem::edit::{self as fs_edit, EditError as FsEditError};
use ade_filesystem::format::Stamp;
use ade_filesystem::volume::FsError;

use crate::health::{self, Severity};

/// How much memory the undo history may hold.
///
/// Sixty floppies, or one hard disk. Past this the oldest step is dropped:
/// running out of undo is a nuisance, running out of memory is a crash.
const UNDO_BUDGET: usize = 64 * 1024 * 1024;

/// Why an edit or a save was refused.
#[derive(Debug)]
pub enum EditError {
    /// The image is not one ADE can edit and save back in the same form.
    NotEditable(String),
    /// The disk already has damage an edit could make worse.
    Unsound(String),
    /// The filesystem layer refused the edit.
    Refused(FsEditError),
    /// The edit was made and then undone, because the health report found
    /// new problems with the result. Each string is one of them.
    ///
    /// This is a bug in ADE rather than in the disk, and is reported as one.
    WouldDamage(Vec<String>),
    /// A path inside the volume did not resolve.
    Lookup(FsError),
    /// The destination is the file the image was opened from (D-018 part 2).
    WouldOverwriteOriginal,
    /// The destination already exists and is not one this session made.
    Exists,
    /// There is nowhere to save to yet: the first save is a Save As.
    NoDestination,
    /// The session was not opened from a file, so there is no original to
    /// save over.
    NoOriginal,
    /// The original file changed on disk after it was opened. Saving over it
    /// would destroy changes this session never saw (D-020).
    ChangedOnDisk,
    /// Reading or writing a file failed.
    Io(std::io::Error),
}

impl core::fmt::Display for EditError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotEditable(why) | Self::Unsound(why) => f.write_str(why),
            Self::Refused(e) => write!(f, "{e}"),
            Self::WouldDamage(found) => write!(
                f,
                "ADE undid this edit because the result failed its own health check \
                 (this is a bug in ADE): {}",
                found.join("; ")
            ),
            Self::Lookup(e) => write!(f, "{e}"),
            Self::WouldOverwriteOriginal => f.write_str(
                "that is the file this disk was opened from; Save As always makes a new \
                 file, and Save is what writes over the original (D-020)",
            ),
            Self::NoOriginal => f.write_str("this disk was not opened from a file"),
            Self::ChangedOnDisk => f.write_str(
                "the file has changed on disk since it was opened; saving over it would lose \
                 those changes, so use Save As",
            ),
            Self::Exists => f.write_str("already exists, refusing to overwrite"),
            Self::NoDestination => f.write_str("nothing has been saved yet; use Save As"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl core::error::Error for EditError {}

impl From<FsEditError> for EditError {
    fn from(e: FsEditError) -> Self {
        Self::Refused(e)
    }
}

/// A disk open for editing.
pub struct Session {
    bytes: Vec<u8>,
    geometry: Geometry,
    /// The file it was opened from, which is never written.
    origin: Option<PathBuf>,
    /// Where this session last saved, which may be saved over again.
    destination: Option<PathBuf>,
    /// Warnings and errors the current bytes already carry, by code.
    baseline: HashMap<&'static str, usize>,
    undo: Vec<Vec<u8>>,
    redo: Vec<Vec<u8>>,
    /// Whether there are edits no save has captured.
    dirty: bool,
    /// A fixed datestamp for edits, in place of the clock.
    clock: Option<Stamp>,
    /// The original file's length and CRC32 as this session last knew it: at
    /// open, then after each save over it. A mismatch means somebody else
    /// wrote it in between (D-020).
    origin_digest: Option<(u64, u32)>,
    /// Where this session kept the original before first saving over it.
    backup: Option<PathBuf>,
    /// How many groups are open; edits inside one undo as one (IMP-014).
    group_depth: u32,
    /// Whether the open group has saved its undo snapshot yet. Only its first
    /// edit does: that snapshot is the state before the whole group.
    group_snapshot: bool,
}

impl Session {
    /// Open an image file for editing.
    ///
    /// # Errors
    /// [`EditError::Io`] if it cannot be read; [`EditError::NotEditable`] or
    /// [`EditError::Unsound`] as for [`Self::from_bytes`].
    pub fn open(path: &Path) -> Result<Self, EditError> {
        let bytes = fs::read(path).map_err(EditError::Io)?;
        let digest = digest(&bytes);
        let mut session = Self::from_bytes(bytes)?;
        session.origin = Some(path.to_path_buf());
        session.origin_digest = Some(digest);
        Ok(session)
    }

    /// Edit bytes already in memory.
    ///
    /// # Errors
    /// [`EditError::NotEditable`] for anything but a plain ADF or an
    /// unpartitioned hardfile holding a volume. [`EditError::Unsound`] for a
    /// disk whose bitmap marks live blocks free: every allocation on it could
    /// land on data, and refusing is the only safe answer until it is
    /// repaired.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, EditError> {
        let Some((_, geometry)) = crate::inspect::plain_volume_geometry(&bytes) else {
            return Err(EditError::NotEditable(
                "ADE edits plain ADF and unpartitioned hardfile images; convert this one \
                 to ADF first"
                    .to_owned(),
            ));
        };
        let report = health::examine(bytes.clone());
        if report.examined.is_none() {
            return Err(EditError::NotEditable(
                "there is no volume on this image that ADE can mount".to_owned(),
            ));
        }
        if let Some(bitmap) = &report.bitmap
            && bitmap.referenced_but_free > 0
        {
            return Err(EditError::Unsound(format!(
                "the bitmap marks {} blocks free that files are using, so a new block could \
                 land on live data",
                bitmap.referenced_but_free
            )));
        }
        Ok(Self {
            bytes,
            geometry,
            origin: None,
            destination: None,
            baseline: problems(&report),
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
            clock: None,
            group_depth: 0,
            group_snapshot: false,
            origin_digest: None,
            backup: None,
        })
    }

    /// The image as it now stands.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The volume's shape.
    #[must_use]
    pub const fn geometry(&self) -> &Geometry {
        &self.geometry
    }

    /// The rootblock's block number: the parent to give for the top level.
    #[must_use]
    pub fn root(&self) -> u32 {
        u32::try_from(self.geometry.root_block().0).unwrap_or(0)
    }

    /// Whether there are edits no save has captured.
    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Where the next [`Self::save`] would write, if anywhere.
    #[must_use]
    pub fn destination(&self) -> Option<&Path> {
        self.destination.as_deref()
    }

    /// The directory at a slash-separated path, as a block to pass as a
    /// parent. An empty path is the root.
    ///
    /// # Errors
    /// [`EditError::Lookup`] if it does not resolve, or is not a directory.
    pub fn directory(&self, path: &str) -> Result<u32, EditError> {
        let image = crate::Image::from_bytes(self.bytes.clone())
            .map_err(|e| EditError::NotEditable(e.to_string()))?;
        let volume = image.volume().map_err(EditError::Lookup)?;
        let entry = volume.lookup(path).map_err(EditError::Lookup)?;
        if !entry.kind.is_directory() {
            return Err(EditError::Lookup(FsError::NotADirectory {
                name: entry.name_lossy(),
            }));
        }
        Ok(entry.block)
    }

    /// Make an empty directory in the directory at `parent`.
    ///
    /// # Errors
    /// See [`EditError`]. A refused edit leaves the session as it was.
    pub fn make_dir(&mut self, parent: u32, name: &[u8]) -> Result<u32, EditError> {
        let geometry = self.geometry;
        let when = self.now();
        self.edit(|bytes| fs_edit::make_dir(bytes, geometry, parent, name, when))
    }

    /// Add a file holding `data` to the directory at `parent`.
    ///
    /// # Errors
    /// See [`EditError`]. A refused edit leaves the session as it was.
    pub fn add_file(&mut self, parent: u32, name: &[u8], data: &[u8]) -> Result<u32, EditError> {
        let geometry = self.geometry;
        let when = self.now();
        self.edit(|bytes| fs_edit::add_file(bytes, geometry, parent, name, data, when))
    }

    /// Stamp every later edit with `when` instead of the clock, so a test's
    /// result is the same bytes on every run. `None` restores the clock.
    pub fn set_clock(&mut self, when: Option<Stamp>) {
        self.clock = when;
    }

    fn now(&self) -> Stamp {
        self.clock.unwrap_or_else(crate::create::now)
    }

    /// Run one edit on a copy, keep it only if the result is no less healthy.
    fn edit(
        &mut self,
        change: impl FnOnce(&mut [u8]) -> Result<u32, FsEditError>,
    ) -> Result<u32, EditError> {
        let mut next = self.bytes.clone();
        let block = change(&mut next)?;

        let report = health::examine(next.clone());
        let after = problems(&report);
        let mut new: Vec<String> = report
            .findings
            .iter()
            .filter(|f| f.severity >= Severity::Warning)
            .filter(|f| {
                after.get(f.code).copied().unwrap_or(0)
                    > self.baseline.get(f.code).copied().unwrap_or(0)
            })
            .map(|f| format!("{}: {}", f.code, f.message))
            .collect();
        new.dedup();
        if !new.is_empty() {
            return Err(EditError::WouldDamage(new));
        }

        let previous = std::mem::replace(&mut self.bytes, next);
        // Inside a group only the first edit records where to undo to, so the
        // whole group comes off with one Undo.
        if self.group_depth == 0 || !self.group_snapshot {
            self.push_undo(previous);
            self.group_snapshot = self.group_depth > 0;
        }
        self.redo.clear();
        self.baseline = after;
        self.dirty = true;
        Ok(block)
    }

    fn push_undo(&mut self, bytes: Vec<u8>) {
        self.undo.push(bytes);
        let mut held: usize = self.undo.iter().map(Vec::len).sum();
        while held > UNDO_BUDGET && self.undo.len() > 1 {
            let dropped = self.undo.remove(0);
            held = held.saturating_sub(dropped.len());
        }
    }

    /// Start a group: every edit until the matching [`Self::end_group`] is
    /// one step for undo and redo (IMP-014).
    ///
    /// For an operation a person sees as one thing — dropping twenty files is
    /// one action, not twenty — so one Undo takes it back. Groups nest; only
    /// the outermost counts. A group in which no edit succeeded leaves nothing
    /// to undo.
    pub fn begin_group(&mut self) {
        if self.group_depth == 0 {
            self.group_snapshot = false;
        }
        self.group_depth = self.group_depth.saturating_add(1);
    }

    /// Close the group [`Self::begin_group`] opened. Closing one that is not
    /// open does nothing.
    pub fn end_group(&mut self) {
        self.group_depth = self.group_depth.saturating_sub(1);
        if self.group_depth == 0 {
            self.group_snapshot = false;
        }
    }

    /// Run `body` as one group, closing it however `body` returns.
    pub fn grouped<T>(&mut self, body: impl FnOnce(&mut Self) -> T) -> T {
        self.begin_group();
        let out = body(self);
        self.end_group();
        out
    }

    /// Moving through history closes any open group: undo means "before what
    /// I did", and a group left open would fold the next edit into the step
    /// just undone.
    fn close_groups(&mut self) {
        self.group_depth = 0;
        self.group_snapshot = false;
    }

    /// Whether there is an edit to undo.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether there is an undone edit to redo.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Undo the last edit. Returns whether there was one.
    pub fn undo(&mut self) -> bool {
        self.close_groups();
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.bytes, previous);
        self.redo.push(current);
        self.settle();
        true
    }

    /// Redo the last undone edit. Returns whether there was one.
    pub fn redo(&mut self) -> bool {
        self.close_groups();
        let Some(next) = self.redo.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.bytes, next);
        self.push_undo(current);
        self.settle();
        true
    }

    /// After moving through history: the baseline follows the bytes, and the
    /// disk counts as changed — a save may already hold a different state.
    fn settle(&mut self) {
        self.baseline = problems(&health::examine(self.bytes.clone()));
        self.dirty = true;
    }

    /// Save to a new file, which later [`Self::save`] calls reuse.
    ///
    /// # Errors
    /// [`EditError::WouldOverwriteOriginal`] for the file the image came
    /// from, [`EditError::Exists`] for any other file this session did not
    /// make, or [`EditError::Io`].
    pub fn save_as(&mut self, path: &Path) -> Result<(), EditError> {
        if self.origin.as_deref().is_some_and(|o| same_file(o, path)) {
            return Err(EditError::WouldOverwriteOriginal);
        }
        let ours = self
            .destination
            .as_deref()
            .is_some_and(|d| same_file(d, path));
        if path.exists() && !ours {
            return Err(EditError::Exists);
        }
        write_atomically(path, &self.bytes).map_err(EditError::Io)?;
        self.destination = Some(path.to_path_buf());
        self.dirty = false;
        Ok(())
    }

    /// Save over the file this session was opened from (D-020).
    ///
    /// Only ever called after the person has confirmed it — that is the front
    /// end's job, and the reason this is a separate call rather than what
    /// [`Self::save`] does. With `keep_backup`, the first save over the
    /// original in this session first copies the file as it is on disk to
    /// `<file>.orig` beside it (or `.orig-2`, `-3` … — an existing backup is
    /// never overwritten), so a wrong edit is still recoverable. The backup is
    /// written and flushed in full before the original is touched, and the
    /// original is then replaced atomically, as every save is.
    ///
    /// Returns the backup's path when this call made one.
    ///
    /// # Errors
    /// [`EditError::NoOriginal`] for a session not opened from a file,
    /// [`EditError::ChangedOnDisk`] if the file is no longer what this
    /// session last read or wrote, or [`EditError::Io`].
    pub fn save_over_original(&mut self, keep_backup: bool) -> Result<Option<PathBuf>, EditError> {
        let origin = self.origin.clone().ok_or(EditError::NoOriginal)?;
        let on_disk = fs::read(&origin).map_err(EditError::Io)?;
        if Some(digest(&on_disk)) != self.origin_digest {
            return Err(EditError::ChangedOnDisk);
        }
        let mut made = None;
        if keep_backup && self.backup.is_none() {
            let path = backup_name(&origin).ok_or_else(|| {
                EditError::Io(std::io::Error::other(
                    "no free name for a backup beside the file",
                ))
            })?;
            write_new(&path, &on_disk).map_err(EditError::Io)?;
            self.backup = Some(path.clone());
            made = Some(path);
        }
        write_atomically(&origin, &self.bytes).map_err(EditError::Io)?;
        self.origin_digest = Some(digest(&self.bytes));
        self.dirty = false;
        Ok(made)
    }

    /// Where this session kept the original, once it has saved over it.
    #[must_use]
    pub fn backup(&self) -> Option<&Path> {
        self.backup.as_deref()
    }

    /// The file this session was opened from, if any.
    #[must_use]
    pub fn origin(&self) -> Option<&Path> {
        self.origin.as_deref()
    }

    /// Save over the file the last [`Self::save_as`] made.
    ///
    /// # Errors
    /// [`EditError::NoDestination`] before any Save As, or as
    /// [`Self::save_as`].
    pub fn save(&mut self) -> Result<(), EditError> {
        let path = self.destination.clone().ok_or(EditError::NoDestination)?;
        self.save_as(&path)
    }
}

/// An Amiga name for a host name: Latin-1 bytes, which is what AmigaDOS
/// stores.
///
/// Every character up to U+00FF has a Latin-1 byte; anything past that has
/// none, and is refused with the character rather than replaced, because a
/// file renamed silently on the way in is one nobody can find again. The
/// AmigaDOS rules — length, `:` and `/` — are the filesystem layer's, and are
/// checked when the name is used.
///
/// # Errors
/// The first character with no Latin-1 byte.
pub fn amiga_name(host: &str) -> Result<Vec<u8>, char> {
    host.chars()
        .map(|c| u8::try_from(u32::from(c)).map_err(|_| c))
        .collect()
}

/// Warnings and errors in a report, counted by code.
///
/// Counted rather than listed because a code is what stays the same across
/// an edit — the message names blocks, and blocks move.
fn problems(report: &health::Health) -> HashMap<&'static str, usize> {
    let mut out: HashMap<&'static str, usize> = HashMap::new();
    for f in report
        .findings
        .iter()
        .filter(|f| f.severity >= Severity::Warning)
    {
        let n = out.entry(f.code).or_insert(0);
        *n = n.saturating_add(1);
    }
    out
}

/// Whether two paths name the same file, including through a link.
fn same_file(a: &Path, b: &Path) -> bool {
    let (Ok(ca), Ok(cb)) = (fs::canonicalize(a), fs::canonicalize(b)) else {
        return false;
    };
    if ca == cb {
        return true;
    }
    // Two names for one file: a hard link canonicalises to itself.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if let (Ok(ma), Ok(mb)) = (fs::metadata(a), fs::metadata(b)) {
            return ma.dev() == mb.dev() && ma.ino() == mb.ino();
        }
    }
    false
}

/// A file's length and CRC32: enough to notice that it changed.
fn digest(bytes: &[u8]) -> (u64, u32) {
    (bytes.len() as u64, ade_block::checksum::crc32(bytes))
}

/// The first of `<file>.orig`, `<file>.orig-2` … that does not exist.
fn backup_name(origin: &Path) -> Option<PathBuf> {
    let name = origin.file_name()?.to_string_lossy().into_owned();
    (1..1000u32)
        .map(|n| {
            let suffix = if n == 1 {
                ".orig".to_owned()
            } else {
                format!(".orig-{n}")
            };
            origin.with_file_name(format!("{name}{suffix}"))
        })
        .find(|p| !p.exists())
}

/// Write a file that must not already exist, flushed before returning. A
/// partial one is removed: a half-written backup is worse than none, because
/// it would be trusted.
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let result = (|| {
        let mut file = fs::File::create_new(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

/// Write `bytes` to `path` so that `path` holds either its old contents or
/// all of the new ones, whatever happens part way (D-018 part 3).
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let name = path
        .file_name()
        .map_or_else(|| "image".into(), |n| n.to_string_lossy().into_owned());
    // In the destination's own directory, so the rename never crosses a
    // filesystem — a cross-device rename is a copy, and a copy is not atomic.
    let temp = dir.join(format!(".{name}.ade-{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::File::create_new(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        // Make the rename itself durable. Best effort: not every platform can
        // open a directory to sync it, and the data is already safe.
        if let Ok(d) = fs::File::open(&dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
