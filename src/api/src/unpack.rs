//! Writing every file off a disk into a folder (F-024).
//!
//! `extract` takes one path at a time. This takes the lot, which is what
//! somebody who wants the contents of a disk actually wants, and it is where
//! an Amiga name meets a filesystem that will not always take it.
//!
//! # What a name has to survive
//!
//! Measured across all 4,652 corpus images, 83,487 distinct filenames:
//!
//! | shape | names | what is done |
//! |---|---|---|
//! | non-ASCII, Latin-1 | many, and meaningful — `Effekte für AE 2 Deutsch.info`, `CD³²_Prefs` | decoded and written as UTF-8 |
//! | a NUL or control byte | 3, all on `Xenon 2 - Megablast_Disk1` | escaped: a NUL cannot go in a POSIX filename at all |
//! | `/` | **0** | escaped anyway; it is structurally impossible to write |
//! | exactly `.` or `..` | 1 | escaped; it is not a name, it is a direction |
//! | a literal `%` | 3 | escaped, so the escaping stays reversible |
//! | Windows-illegal `\ : * ? " < > \|` | 62, all decorative — `>>> BY AEON <<<` | **left alone** |
//! | trailing dot or space, or all spaces | 328 | **left alone** |
//!
//! The last two rows are the judgement. Those names are legal on POSIX, and
//! escaping them would mangle 390 real names to buy portability to a platform
//! ADE has never been built on. A Windows build will need to escape more, and
//! that is a decision to take with a Windows build in front of you rather than
//! now — the numbers above are what it costs.
//!
//! # Nothing is ever overwritten
//!
//! A target that already exists is skipped and reported, never replaced. Two
//! files in one drawer cannot share a name — AmigaDOS's hash table prevents it
//! — so a collision here means either a case-insensitive host or two names
//! that escaped to the same string. Exactly one corpus image collides that
//! way: `1869 (AGA)_Disk1` has `Startup-sequence.bak` and
//! `startup-sequence.bak` in the same drawer. On Linux both are written; on a
//! case-insensitive host the second is skipped and said so, which is the only
//! honest answer available.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ade_filesystem::entry::EntryKind;

/// A file that was not written, and why.
#[derive(Debug, Clone)]
pub struct Skipped {
    /// The path on the disk.
    pub path: String,
    /// Why it was not written.
    pub reason: String,
}

/// What an unpack did.
#[derive(Debug, Clone, Default)]
pub struct Unpacked {
    /// Files written.
    pub files: u64,
    /// Directories created.
    pub directories: u64,
    /// Bytes written.
    pub bytes: u64,
    /// Everything that was not written, and why.
    pub skipped: Vec<Skipped>,
}

/// The name to write an Amiga name under on this host.
///
/// Takes a name that has **already been decoded** from Latin-1 — which is what
/// [`ade_filesystem::entry::Entry::name_lossy`] and every path in a `Walk`
/// hold. Taking bytes here instead would decode a second time and turn `für`
/// into `fÃ¼r`, which is the classic way to lose a name while appearing to
/// preserve it.
///
/// What comes back is UTF-8, with anything that cannot be a filename escaped
/// as `%XX` of its original Latin-1 byte. The escape is reversible — `%` is
/// escaped too — which matters because for a file taken off a disk this is the
/// only record of what its name was.
#[must_use]
pub fn host_name(name: &str) -> String {
    // A name that is nothing, or that names a direction rather than a thing.
    // `.` and `..` would resolve to a different directory rather than fail,
    // which is the one case here where a quiet mistake is possible. One corpus
    // image carries such a name.
    if name.is_empty() {
        return "%00".to_owned();
    }
    if name == "." || name == ".." {
        let mut out = String::new();
        for ch in name.chars() {
            escape(&mut out, ch);
        }
        return out;
    }

    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        let code = ch as u32;
        match ch {
            // The path separator, the escape character itself, and anything a
            // filesystem cannot hold. A name is Latin-1, so every character is
            // one byte and the escape names that byte.
            '/' | '%' => escape(&mut out, ch),
            _ if code < 0x20 || code == 0x7F => escape(&mut out, ch),
            _ => out.push(ch),
        }
    }
    out
}

/// Append `%XX` for a character's Latin-1 byte.
fn escape(out: &mut String, ch: char) {
    use core::fmt::Write as _;
    // A name is Latin-1, so every character is one byte. A character that
    // somehow is not fails the write rather than being truncated into a
    // different, plausible-looking escape.
    let _ = write!(out, "%{:02X}", ch as u32);
}

/// Write every file on `volume` into `dest`.
///
/// # Errors
/// Only for a failure to create `dest` itself. A file that cannot be read or
/// written is recorded in [`Unpacked::skipped`] and the rest continue: a run
/// over a damaged disk that stops at the first bad file has recovered nothing,
/// which is the same reasoning as `ade batch`.
pub fn unpack(volume: &ade_filesystem::volume::Volume<'_>, dest: &Path) -> io::Result<Unpacked> {
    fs::create_dir_all(dest)?;
    let mut out = Unpacked::default();

    let walk = match volume.walk(volume.root()) {
        Ok(w) => w,
        Err(e) => {
            out.skipped.push(Skipped {
                path: String::new(),
                reason: format!("the volume could not be walked: {e}"),
            });
            return Ok(out);
        }
    };

    // Directories first, so a file never has to create its own parent and a
    // drawer that holds nothing is still recovered — an empty drawer is a fact
    // about the disk.
    for (path, entry) in &walk.entries {
        if !entry.kind.is_directory() {
            continue;
        }
        let target = under(dest, path);
        match fs::create_dir_all(&target) {
            Ok(()) => out.directories = out.directories.saturating_add(1),
            Err(e) => out.skipped.push(Skipped {
                path: path.clone(),
                reason: format!("{e}"),
            }),
        }
    }

    for (path, entry) in &walk.entries {
        if !entry.kind.is_file() {
            continue;
        }
        // A hard link holds no data of its own; the block it names does
        // (BUG-005). Reading the link directly gives an empty file, silently.
        let real = match entry.kind {
            EntryKind::HardLinkFile => match volume.resolve(entry) {
                Ok(r) => r,
                Err(e) => {
                    out.skipped.push(Skipped {
                        path: path.clone(),
                        reason: format!("link could not be resolved: {e}"),
                    });
                    continue;
                }
            },
            _ => entry.clone(),
        };

        let data = match volume.read_file(&real) {
            Ok(d) => d.into_bytes(),
            Err(e) => {
                out.skipped.push(Skipped {
                    path: path.clone(),
                    reason: format!("{e}"),
                });
                continue;
            }
        };

        let target = under(dest, path);
        if target.exists() {
            out.skipped.push(Skipped {
                path: path.clone(),
                reason: format!("{} already exists", target.display()),
            });
            continue;
        }
        if let Some(parent) = target.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match fs::write(&target, &data) {
            Ok(()) => {
                out.files = out.files.saturating_add(1);
                out.bytes = out.bytes.saturating_add(data.len() as u64);
            }
            Err(e) => out.skipped.push(Skipped {
                path: path.clone(),
                reason: format!("{e}"),
            }),
        }
    }
    out.skipped.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// `dest` joined with an Amiga path, one sanitised component at a time.
///
/// Component by component rather than sanitising the whole string, because the
/// separators are the one thing that must survive and the names are the one
/// thing that must not be trusted.
fn under(dest: &Path, path: &str) -> PathBuf {
    let mut target = dest.to_path_buf();
    for component in path.split('/') {
        if component.is_empty() {
            continue;
        }
        target.push(host_name(component));
    }
    target
}
