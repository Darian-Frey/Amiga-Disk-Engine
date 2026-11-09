//! Where a program's names and comments are kept, and how they are found
//! (D-022).
//!
//! One file per program, `<sha1>.adeproj`, in the user's projects folder:
//! `$ADE_PROJECTS`, or `$XDG_DATA_HOME/ade/projects`, or
//! `~/.local/share/ade/projects`. The SHA-1 is of the program file's bytes,
//! or of a bootblock's 1,024 bytes, so the same program on any disk finds
//! the same notes, and a program changed by a byte finds none. Nothing is
//! ever written outside that folder, and a disk image never is.

use std::path::{Path, PathBuf};

use ade_object::annotations::{Annotations, MAX_FILE};
use ade_object::flow::Place;
use ade_object::hunk;

/// The projects folder, whether or not it exists yet; `None` when there is
/// no home to put one in.
#[must_use]
pub fn location() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("ADE_PROJECTS") {
        return Some(PathBuf::from(dir));
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("ade/projects"))
}

/// A program's key: the SHA-1 of its bytes, in hex.
#[must_use]
pub fn key(program: &[u8]) -> String {
    ade_catalogue::sha1::hex(&ade_catalogue::sha1::sha1(program))
}

/// The file a program's notes are kept in, under `dir`.
#[must_use]
pub fn file_in(dir: &Path, program: &[u8]) -> PathBuf {
    dir.join(format!("{}.adeproj", key(program)))
}

/// A program's notes as read from `dir`, and each line that could not be
/// taken. No file is no notes, and no complaint.
#[must_use]
pub fn load_from(dir: &Path, program: &[u8]) -> (Annotations, Vec<String>) {
    let path = file_in(dir, program);
    let Ok(meta) = std::fs::metadata(&path) else {
        return (Annotations::default(), Vec::new());
    };
    // Bounded before it is read (D-006).
    if meta.len() > MAX_FILE as u64 {
        return (
            Annotations::default(),
            vec![format!(
                "{}: over {MAX_FILE} bytes, and not read",
                path.display()
            )],
        );
    }
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let (notes, problems) = Annotations::parse(&text);
            let problems = problems
                .into_iter()
                .map(|p| format!("{}: {p}", path.display()))
                .collect();
            (notes, problems)
        }
        Err(e) => (
            Annotations::default(),
            vec![format!("{}: {e}", path.display())],
        ),
    }
}

/// A program's notes from the projects folder.
#[must_use]
pub fn load(program: &[u8]) -> (Annotations, Vec<String>) {
    match location() {
        Some(dir) => load_from(&dir, program),
        None => (Annotations::default(), Vec::new()),
    }
}

/// Keep a program's notes in `dir`: written beside the file and renamed
/// over it, so a reader sees the old notes or the new, never half. Notes
/// with nothing in them remove the file.
///
/// # Errors
/// The folder cannot be made, or the file written.
pub fn save_to(dir: &Path, program: &[u8], notes: &Annotations) -> std::io::Result<PathBuf> {
    let path = file_in(dir, program);
    if notes.is_empty() {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        return Ok(path);
    }
    std::fs::create_dir_all(dir)?;
    let text = notes.to_text((&key(program), program.len()));
    let temporary = path.with_extension(format!("adeproj.{}.tmp", std::process::id()));
    std::fs::write(&temporary, text)?;
    if let Err(e) = std::fs::rename(&temporary, &path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(e);
    }
    Ok(path)
}

/// Keep a program's notes in the projects folder.
///
/// # Errors
/// There is no folder to keep them in, or it cannot be written.
pub fn save(program: &[u8], notes: &Annotations) -> Result<PathBuf, String> {
    let dir = location()
        .ok_or_else(|| "there is no home folder to keep notes in; set ADE_PROJECTS".to_owned())?;
    save_to(&dir, program, notes).map_err(|e| format!("{}: {e}", dir.display()))
}

/// The place a listing line's file offset is in an executable: its hunk,
/// and how far into it.
#[must_use]
pub fn place_at(program: &[u8], offset: u64) -> Option<Place> {
    let exe = hunk::parse(program)?;
    exe.hunks.iter().enumerate().find_map(|(k, h)| {
        let at = offset.checked_sub(h.file_offset as u64)?;
        if at >= h.file_len as u64 {
            return None;
        }
        Some((k, u32::try_from(at).ok()?))
    })
}

/// The place a bootblock line's offset is: the offset itself, in hunk 0.
#[must_use]
pub fn boot_place_at(offset: u64) -> Option<Place> {
    let at = u32::try_from(offset).ok()?;
    (12..1024).contains(&at).then_some((0, at))
}

/// Whether `place` is in the program: a hunk it has, at an offset inside
/// the memory that hunk takes.
fn holds(program: &[u8], (k, at): Place) -> bool {
    hunk::parse(program)
        .and_then(|exe| {
            exe.hunks.get(k).map(|h| {
                let end = u32::try_from(h.file_len).unwrap_or(u32::MAX).max(h.size);
                at < end
            })
        })
        .unwrap_or(false)
}

/// What to change about one place.
#[derive(Debug, Clone, Copy)]
pub enum Change<'a> {
    /// Give it this name, or clear its name with `None`.
    Name(Option<&'a str>),
    /// Give it this comment, or clear its comment with `None`.
    Comment(Option<&'a str>),
}

/// Name a place in a program, or comment on it, and keep the change in
/// `dir`. `boot` says the program is a bootblock, whose places are its own
/// offsets from 12 to 1,023.
///
/// # Errors
/// Why not, in words: the place is not in the program, the name or comment
/// breaks a rule (D-022 part 2), or the notes cannot be written.
pub fn change_in(
    dir: &Path,
    program: &[u8],
    boot: bool,
    place: Place,
    change: Change<'_>,
) -> Result<PathBuf, String> {
    let inside = if boot {
        place.0 == 0 && (12..1024).contains(&place.1)
    } else {
        holds(program, place)
    };
    if !inside {
        return Err(format!(
            "{} is not a place in this program",
            ade_object::annotations::place_text(place)
        ));
    }
    let (mut notes, problems) = load_from(dir, program);
    // A file with lines it could not take is not written over: saving would
    // drop them silently.
    if !problems.is_empty() {
        return Err(format!(
            "the notes file has lines ADE cannot read, which saving would lose: {}",
            problems.join("; ")
        ));
    }
    match change {
        Change::Name(name) => notes.set_name(place, name)?,
        Change::Comment(comment) => notes.set_comment(place, comment)?,
    }
    save_to(dir, program, &notes).map_err(|e| format!("{}: {e}", dir.display()))
}

/// [`change_in`] the projects folder.
///
/// # Errors
/// As [`change_in`], or there is no folder to keep notes in.
pub fn change(
    program: &[u8],
    boot: bool,
    place: Place,
    change: Change<'_>,
) -> Result<PathBuf, String> {
    let dir = location()
        .ok_or_else(|| "there is no home folder to keep notes in; set ADE_PROJECTS".to_owned())?;
    change_in(&dir, program, boot, place, change)
}
