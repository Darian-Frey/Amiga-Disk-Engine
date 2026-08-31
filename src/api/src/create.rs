//! Making a blank disk (F-019, F-025).
//!
//! The *formatting* is [`ade_filesystem::format::blank`]. What lives here is
//! everything a front end would otherwise have to know for itself: which
//! filesystems ADE will write, what to call them, and what shape each disk is.
//!
//! It is here rather than in the CLI because the GUI needs the same answers,
//! and two front ends deciding separately which dostypes exist is two chances
//! to disagree with the engine — the same reasoning that moved MFM encoding
//! out of the CLI (IMP-007).

use std::path::Path;

use ade_block::Geometry;
use ade_filesystem::dostype::Dostype;
use ade_filesystem::format::{self, FormatError, Stamp};

/// A filesystem `ade create` will write.
#[derive(Debug, Clone, Copy)]
pub struct DiskType {
    /// What a caller names it: `ffs-intl`.
    pub name: &'static str,
    /// What a person reads: `FFS, international (DOS\3)`.
    pub label: &'static str,
    /// The dostype's flags byte.
    pub flags: u8,
}

/// The six AmigaDOS types ADE writes.
///
/// `DOS\6` and `DOS\7` are absent on purpose. LNFS is deferred by **D-013** on
/// verifiability rather than effort: no corpus image carries one and ADFlib
/// misreads them, so writing one means producing a format checkable only
/// against itself — which is what D-002 gave up ADFlib's accumulated knowledge
/// to avoid. Beyond AmigaDOS there are some forty other 4-byte tags in SPEC's
/// registry; none of them is ADE's to write.
pub const TYPES: [DiskType; 6] = [
    DiskType {
        name: "ofs",
        label: "OFS (DOS\\0)",
        flags: 0,
    },
    DiskType {
        name: "ffs",
        label: "FFS (DOS\\1)",
        flags: 1,
    },
    DiskType {
        name: "ofs-intl",
        label: "OFS, international (DOS\\2)",
        flags: 2,
    },
    DiskType {
        name: "ffs-intl",
        label: "FFS, international (DOS\\3)",
        flags: 3,
    },
    DiskType {
        name: "ofs-dc",
        label: "OFS, directory cache (DOS\\4)",
        flags: 4,
    },
    DiskType {
        name: "ffs-dc",
        label: "FFS, directory cache (DOS\\5)",
        flags: 5,
    },
];

/// The default, and why it is not plain `ffs`.
///
/// Everything since Workbench 2.0 writes the international variant, and a name
/// with an accent sorts wrongly without it (C-006).
pub const DEFAULT_TYPE: &str = "ffs-intl";

/// The dostype a type name means, or `None` if ADE will not write it.
#[must_use]
pub fn dostype(name: &str) -> Option<Dostype> {
    let wanted = name.to_ascii_lowercase();
    let found = TYPES.iter().find(|t| t.name == wanted)?;
    Dostype::from_raw(0x444F_5300 | u32::from(found.flags)).ok()
}

/// The shape of a disk to make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// 3.5" double density: 880 KB, the norm.
    Dd,
    /// 3.5" high density: 1.76 MB.
    Hd,
    /// 5.25" double density: the A1020's 440 KB.
    ///
    /// No corpus image is one, and ADFlib refuses the size before it reaches
    /// any filesystem, so this rests on the rootblock formula being verified
    /// at other block counts rather than on an oracle.
    Dd525,
    /// An unpartitioned hard disk of this many megabytes.
    Hard(u32),
}

impl Shape {
    /// The geometry to format.
    ///
    /// # Errors
    /// A size that is not a disk, or one too large to express.
    pub fn geometry(self) -> Result<Geometry, String> {
        match self {
            // No option for the 81-, 82- and 83-cylinder images that occur in
            // the wild: those are not larger volumes but ordinary 80-cylinder
            // filesystems in files holding extra tracks, with the rootblock
            // still at 880 (BUG-009). Writing one would produce a file
            // matching no real disk.
            Self::Dd => Geometry::new(80, 2, 11, 512, 2),
            Self::Hd => Geometry::new(80, 2, 22, 512, 2),
            Self::Dd525 => Geometry::new(40, 2, 11, 512, 2),
            Self::Hard(megabytes) => {
                if megabytes == 0 {
                    return Err("0 MB is not a disk".to_owned());
                }
                // A hard disk is not a bigger floppy: it has no cylinders,
                // heads or sectors that anything reads, only a block count
                // (SPEC §Hardfiles). One head and 32 sectors is UAE's
                // convention and makes a megabyte exactly 64 cylinders.
                let cylinders = megabytes
                    .checked_mul(64)
                    .ok_or_else(|| format!("{megabytes} MB: too large"))?;
                Geometry::new(cylinders, 1, 32, 512, 2)
            }
        }
        .map_err(|e| format!("{e}"))
    }
}

/// Why a disk could not be made.
#[derive(Debug)]
pub enum CreateError {
    /// ADE will not write that filesystem.
    UnknownType(String),
    /// The shape is not a disk ADE can make.
    Shape(String),
    /// The formatter refused.
    Format(FormatError),
    /// Something already exists at that path.
    Exists,
    /// The file could not be written.
    Io(std::io::Error),
}

impl core::fmt::Display for CreateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownType(name) => {
                let names: Vec<&str> = TYPES.iter().map(|t| t.name).collect();
                write!(f, "{name}: expected one of {}", names.join(", "))
            }
            Self::Shape(why) => write!(f, "{why}"),
            Self::Format(e) => write!(f, "{e}"),
            Self::Exists => f.write_str("already exists, refusing to overwrite"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl core::error::Error for CreateError {}

/// The current time, as AmigaDOS counts it.
///
/// "Created" means when the disk was made, and a tool that lies about that is
/// worse than one that omits it. Day zero is *illegal*: SPEC records that
/// Amiga software treats it as unset, and ADE's own health check agrees — the
/// first disk `ade create` ever produced reported three `datestamp-day-zero`
/// findings against itself.
///
/// Here rather than in a front end because every front end needs it and none
/// of them should be deciding what an Amiga epoch is. [`blank`] still takes an
/// explicit stamp, so tests stay deterministic.
///
/// Days are counted from 1978-01-01, which is 2,922 days after the Unix epoch
/// — eight years including the leap days of 1972 and 1976. A clock before 1978
/// gives day 1 rather than a negative: the field is unsigned, and a disk
/// stamped "the day after the Amiga's epoch" is odd where an underflowed one
/// is corrupt.
#[must_use]
pub fn now() -> Stamp {
    const AMIGA_EPOCH_IN_UNIX_DAYS: u64 = 2922;
    let Ok(since) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return Stamp {
            days: 1,
            mins: 0,
            ticks: 0,
        };
    };
    let secs = since.as_secs();
    let days = secs
        .checked_div(86_400)
        .unwrap_or(0)
        .saturating_sub(AMIGA_EPOCH_IN_UNIX_DAYS)
        .max(1);
    let in_day = secs.checked_rem(86_400).unwrap_or(0);
    Stamp {
        days: u32::try_from(days).unwrap_or(1),
        mins: u32::try_from(in_day.checked_div(60).unwrap_or(0)).unwrap_or(0),
        ticks: u32::try_from(in_day.checked_rem(60).unwrap_or(0).saturating_mul(50)).unwrap_or(0),
    }
}

/// Write a blank disk to `path`.
///
/// **Never overwrites.** A blank disk is the safest write there is precisely
/// because it makes a new file; writing over one somebody already has would
/// give that away for nothing.
///
/// # Errors
/// [`CreateError`], which distinguishes a filesystem ADE will not write from a
/// shape it cannot make from a file it could not put there.
pub fn blank(
    path: &Path,
    type_name: &str,
    volume_name: &str,
    shape: Shape,
    created: Stamp,
) -> Result<(), CreateError> {
    let dostype =
        dostype(type_name).ok_or_else(|| CreateError::UnknownType(type_name.to_owned()))?;
    let geometry = shape.geometry().map_err(CreateError::Shape)?;
    let bytes =
        format::blank(geometry, dostype, volume_name, created).map_err(CreateError::Format)?;

    if path.exists() {
        return Err(CreateError::Exists);
    }
    std::fs::write(path, &bytes).map_err(CreateError::Io)
}
