//! An image file being read: held, or read where it is (IMP-029).
//!
//! A floppy is read whole, as ADE always has: it is quick, and what is read is
//! a snapshot. A hard disk image may be gigabytes of which a command looks at
//! a few blocks, so one past [`WHOLE_UP_TO`] is left in its file. The answers
//! are the same either way; what differs is that a file changed while it is
//! being read is read as it changes.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use ade_catalogue::{Catalogue, Identification};
use ade_container::bytes::ByteSource;
use ade_container::{file::FileBytes, sniff};
use ade_object::find::Pattern;

use crate::inspect::{Inspection, InspectionError, head_of, reads_in_place};
use crate::{Health, Image};

/// The largest file read whole. Every floppy, and the hardfiles small enough
/// that holding one is nothing.
pub const WHOLE_UP_TO: u64 = 16 * 1024 * 1024;

/// An image file opened to be read.
pub struct Input {
    path: PathBuf,
    held: Held,
}

enum Held {
    Bytes(Vec<u8>),
    File(FileBytes),
}

impl Input {
    /// Open a file, holding it if it is small or is not its own blocks.
    ///
    /// # Errors
    /// Whatever opening or reading it reports.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        Self::open_over(path, WHOLE_UP_TO)
    }

    /// [`Self::open`], leaving in place any file larger than `limit`.
    ///
    /// # Errors
    /// As [`Self::open`].
    pub fn open_over(path: &Path, limit: u64) -> std::io::Result<Self> {
        let mut file = File::open(path)?;
        let about = file.metadata()?;
        // A pipe has no length and no positions: it can only be read whole.
        if about.is_file() && about.len() > limit {
            let bytes = FileBytes::new(file)?;
            if reads_in_place(sniff(&head_of(&bytes), bytes.size()).kind) {
                return Ok(Self {
                    path: path.to_path_buf(),
                    held: Held::File(bytes),
                });
            }
            file = File::open(path)?;
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(Self {
            path: path.to_path_buf(),
            held: Held::Bytes(bytes),
        })
    }

    /// The file's size.
    #[must_use]
    pub fn size(&self) -> u64 {
        match &self.held {
            Held::Bytes(bytes) => bytes.len() as u64,
            Held::File(file) => file.size(),
        }
    }

    /// Inspect it.
    #[must_use]
    pub fn inspect(&self) -> Inspection {
        match &self.held {
            Held::Bytes(bytes) => {
                if reads_in_place(sniff(&head_of(&bytes.as_slice()), self.size()).kind) {
                    crate::inspect::inspect_in_place(&bytes.as_slice())
                } else {
                    crate::inspect_bytes(bytes.clone())
                }
            }
            Held::File(file) => crate::inspect::inspect_in_place(file),
        }
    }

    /// Examine it, or the partition named: see [`crate::examine_partition`].
    #[must_use]
    pub fn examine(&self, partition: Option<&str>) -> Health {
        match &self.held {
            Held::Bytes(bytes) => {
                if reads_in_place(sniff(&head_of(&bytes.as_slice()), self.size()).kind) {
                    crate::health::examine_in_place(&bytes.as_slice(), partition)
                } else {
                    crate::examine_partition(bytes.clone(), partition)
                }
            }
            Held::File(file) => crate::health::examine_in_place(file, partition),
        }
    }

    /// Name it from a dataset. A file left in place is read only if the
    /// dataset has anything of its size.
    ///
    /// # Errors
    /// Whatever reading it reports.
    pub fn identify<'c>(&self, catalogue: &'c Catalogue) -> std::io::Result<Identification<'c>> {
        match &self.held {
            Held::Bytes(bytes) => Ok(catalogue.identify_detailed(bytes)),
            Held::File(file) if catalogue.has_size(file.size()) => {
                Ok(catalogue.identify_detailed(&std::fs::read(&self.path)?))
            }
            Held::File(_) => Ok(Identification::unknown()),
        }
    }

    /// The names a dataset gives it.
    ///
    /// # Errors
    /// As [`Self::identify`].
    pub fn names(&self, catalogue: &Catalogue) -> std::io::Result<Vec<String>> {
        Ok(self
            .identify(catalogue)?
            .entries
            .into_iter()
            .map(|e| e.name.clone())
            .collect())
    }

    /// Every byte of it, read whole if it was not — for what needs them all.
    ///
    /// # Errors
    /// Whatever reading it reports.
    pub fn into_bytes(self) -> std::io::Result<Vec<u8>> {
        match self.held {
            Held::Bytes(bytes) => Ok(bytes),
            Held::File(_) => std::fs::read(&self.path),
        }
    }

    /// Open it for browsing.
    ///
    /// # Errors
    /// As [`Image::open`].
    pub fn into_image(self) -> Result<Image, InspectionError> {
        match self.held {
            Held::Bytes(bytes) => Image::from_bytes(bytes),
            Held::File(_) => Image::open_lazy(&self.path),
        }
    }

    /// Scan it for known content: see [`crate::scan::Scan::of`].
    ///
    /// # Errors
    /// Whatever reading it reports.
    pub fn scan(&self) -> std::io::Result<crate::scan::Scan> {
        let block_size = self.inspect().geometry.map_or(512, |g| g.block_size());
        match &self.held {
            Held::Bytes(bytes) => Ok(crate::scan::Scan::of(bytes, block_size)),
            Held::File(file) => crate::scan::Scan::over(file, block_size),
        }
    }

    /// Search it: see [`crate::find::Search::run`].
    ///
    /// # Errors
    /// Whatever reading it reports.
    pub fn search(&self, pattern: &Pattern) -> std::io::Result<crate::find::Search> {
        match &self.held {
            Held::Bytes(bytes) => Ok(crate::find::Search::run(bytes, pattern)),
            Held::File(file) => {
                let image = Image::open_lazy(&self.path).ok();
                crate::find::Search::in_place(image.as_ref(), file, pattern)
            }
        }
    }

    /// What it says it needs: see [`crate::specs::Specs::of`].
    ///
    /// # Errors
    /// Whatever opening or reading it reports.
    pub fn specs(self) -> Result<crate::specs::Specs, InspectionError> {
        match self.held {
            Held::Bytes(bytes) => {
                let image = Image::from_bytes(bytes.clone())?;
                Ok(crate::specs::Specs::of(&image, &bytes.as_slice()))
            }
            Held::File(file) => {
                let image = Image::open_lazy(&self.path)?;
                Ok(crate::specs::Specs::of(&image, &file))
            }
        }
    }

    /// Its bytes, by position.
    #[must_use]
    pub fn bytes(&self) -> Box<dyn ByteSource + '_> {
        match &self.held {
            Held::Bytes(bytes) => Box::new(bytes.as_slice()),
            Held::File(file) => Box::new(Borrowed(file)),
        }
    }
}

struct Borrowed<'a>(&'a FileBytes);

impl ByteSource for Borrowed<'_> {
    fn size(&self) -> u64 {
        self.0.size()
    }

    fn read_at(&self, offset: u64, out: &mut [u8]) -> bool {
        self.0.read_at(offset, out)
    }
}

/// How many bytes are read at a time by what reads every one.
const PIECE: usize = 1024 * 1024;

/// Read all of `bytes` a piece at a time, each a whole number of blocks and
/// followed by up to `overlap` bytes of the next: `each` is given the piece
/// and its overlap, the offset it starts at and how much of it is its own,
/// and returns whether to go on.
///
/// Returns how many bytes were read: less than there are only if reading
/// failed, or `each` stopped it.
pub(crate) fn pieces(
    bytes: &dyn ByteSource,
    block_size: u32,
    overlap: usize,
    mut each: impl FnMut(&[u8], u64, usize) -> bool,
) -> u64 {
    let block = usize::try_from(block_size).unwrap_or(512).max(1);
    let piece = PIECE
        .checked_div(block)
        .unwrap_or(1)
        .max(1)
        .saturating_mul(block);
    let mut at = 0u64;
    while at < bytes.size() {
        let left = usize::try_from(bytes.size().saturating_sub(at)).unwrap_or(usize::MAX);
        let buffer = bytes.range(at, piece.saturating_add(overlap).min(left));
        if buffer.is_empty() {
            break;
        }
        let own = buffer.len().min(piece);
        let go_on = each(&buffer, at, own);
        at = at.saturating_add(own as u64);
        if !go_on {
            break;
        }
    }
    at
}
