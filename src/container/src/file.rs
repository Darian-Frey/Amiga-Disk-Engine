//! A block source that reads from the file rather than a copy of it (IMP-005).
//!
//! # Why this exists
//!
//! [`RawImage`](crate::RawImage) owns the whole image. That is right for a
//! command that opens one disk, examines it and exits — `ade batch` runs a
//! 4,652-image corpus at 9 MB peak precisely because it holds one image at a
//! time. It is wrong for a front end that keeps every opened image alive: 400
//! floppies cost 400 MB, and the ceiling is not a disk count but total bytes,
//! so one 500 MB hardfile exceeds the whole floppy corpus.
//!
//! This reads each block from the file when it is asked for, so what stays
//! resident is a file handle and whatever the operating system chooses to
//! cache — which it can also reclaim, and which is shared with every other
//! reader of the same file.
//!
//! # What it costs, and it is not nothing
//!
//! **The file becomes live.** An image held in memory is a snapshot; an image
//! held this way is a window onto a file that can be deleted, truncated or
//! replaced underneath it. Reads that could not fail before can now fail, and
//! a front end holding an image from a USB stick will see it break when the
//! stick is pulled. That is why this is a *choice* — `Image::open` still reads
//! whole, and only a caller that knows it will hold many images asks for this.
//!
//! **It only works where a block's offset is its offset in the file.** A
//! gzip-wrapped image is decompressed before it is blocks, and a flux capture
//! is reconstructed; neither has a file to read blocks from, so both stay
//! materialised. The caller decides, because the caller is the one that
//! sniffed the container.
//!
//! # No `unsafe`, no dependency
//!
//! Memory-mapping would be the obvious answer and is unavailable twice over:
//! the workspace forbids `unsafe` (D-006) and has no dependencies, and every
//! mmap crate is one or the other. Positional reads — `read_at` on Unix,
//! `seek_read` on Windows — are safe, in `std`, and enough.

use std::fs::File;
use std::io;
use std::path::Path;

use ade_block::{BlockError, BlockIndex, BlockSource, Geometry, ValidBlock};

/// An image read from its file, one block at a time.
#[derive(Debug)]
pub struct FileSource {
    file: File,
    geometry: Geometry,
}

impl FileSource {
    /// Open a file as a block source with the given geometry.
    ///
    /// # Errors
    /// [`io::Error`] if the file cannot be opened or is shorter than the
    /// geometry needs. The length is checked once here rather than on every
    /// read: a truncated image should be refused when it is opened, not
    /// discovered halfway through a directory walk.
    pub fn open(path: &Path, geometry: Geometry) -> io::Result<Self> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        if len < geometry.total_bytes() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "file is shorter than its geometry",
            ));
        }
        Ok(Self { file, geometry })
    }

    /// Read exactly `out.len()` bytes from `offset`.
    ///
    /// Positional rather than seek-then-read, so the source needs no interior
    /// mutability and two threads cannot move each other's cursor.
    fn read_exact_at(&self, offset: u64, out: &mut [u8]) -> io::Result<()> {
        #[cfg(unix)]
        {
            std::os::unix::fs::FileExt::read_exact_at(&self.file, out, offset)
        }
        #[cfg(windows)]
        {
            let mut done = 0usize;
            while done < out.len() {
                let Some(slice) = out.get_mut(done..) else {
                    break;
                };
                let read = std::os::windows::fs::FileExt::seek_read(
                    &self.file,
                    slice,
                    offset.saturating_add(done as u64),
                )?;
                if read == 0 {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short read"));
                }
                done = done.saturating_add(read);
            }
            Ok(())
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (offset, out);
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "positional reads are not available on this platform",
            ))
        }
    }
}

impl BlockSource for FileSource {
    fn geometry(&self) -> &Geometry {
        &self.geometry
    }

    fn read_block(&self, block: ValidBlock, out: &mut [u8]) -> Result<(), BlockError> {
        let size = self.geometry.block_size() as usize;
        if out.len() != size {
            return Err(BlockError::BufferSize {
                got: out.len(),
                want: size,
            });
        }
        let offset = block
            .index()
            .saturating_mul(u64::from(self.geometry.block_size()));
        // A read that fails is reported as a truncation of that block. The
        // caller cannot do anything more specific with an errno, and every
        // reader above already handles a block that would not read.
        self.read_exact_at(offset, out)
            .map_err(|_| BlockError::Truncated {
                index: BlockIndex(block.index()),
            })
    }
}
