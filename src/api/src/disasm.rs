//! The Assembly view's engine side (F-032, D-019).
//!
//! What a front end asks is "show me this file's code" or "show me this
//! disk's boot code". Deciding whether a file *is* code, and where in it the
//! code is, belongs here rather than in a front end: the GUI and the CLI must
//! give the same answer, and the answer comes from the hunk format, not from a
//! file name.

use ade_object::hunk;
pub use ade_object::listing::{Line, LineKind, Listing};

use crate::Image;

/// How to treat a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Disassemble it only if it is an AmigaDOS executable.
    Auto,
    /// Decode it from its first byte whatever it is, headed as a guess.
    Raw,
}

/// Why a file was not disassembled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotCode(pub String);

impl core::fmt::Display for NotCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for NotCode {}

/// A file's contents as a listing.
///
/// # Errors
/// [`NotCode`] in [`Mode::Auto`] for anything that is not an executable,
/// saying what it begins with — the view shows that, and offers [`Mode::Raw`].
pub fn file(bytes: &[u8], mode: Mode) -> Result<Listing, NotCode> {
    if mode == Mode::Raw {
        return Ok(ade_object::listing::raw(bytes));
    }
    if bytes.is_empty() {
        return Err(NotCode("the file is empty".to_owned()));
    }
    if !hunk::is_executable(bytes) {
        return Err(NotCode(
            "not an AmigaDOS executable: it does not begin with a hunk header (000003F3)"
                .to_owned(),
        ));
    }
    ade_object::listing::executable(bytes).ok_or_else(|| {
        NotCode("it begins like an executable, but its hunk header is unreadable".to_owned())
    })
}

/// The image's boot code as a listing.
///
/// # Errors
/// [`NotCode`] when the image's first block is not an AmigaDOS bootblock — a
/// hard disk's is its partition table, an extended ADF's is its own header.
pub fn bootblock(image: &Image) -> Result<Listing, NotCode> {
    let boot = image.read_range(0, 1024);
    if boot.len() < 1024 || boot.get(..3) != Some(b"DOS".as_slice()) {
        return Err(NotCode(
            "this image does not begin with an AmigaDOS bootblock".to_owned(),
        ));
    }
    Ok(ade_object::listing::bootblock(&boot))
}
