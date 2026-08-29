//! The core library API — the single seam the CLI and the Qt6 GUI both consume.
//!
//! No engine logic lives here and none lives in the front-ends (F-002); this
//! crate wires the layers together and presents one surface. It is the only
//! crate that depends on every layer, and the only place cross-layer
//! coordination is permitted (D-003).

use std::path::{Path, PathBuf};

pub mod assemble;
pub mod batch;
pub mod consolidate;
pub mod convert;
pub mod find;
pub mod health;
pub mod inspect;
pub mod json;
pub mod scan;

pub use assemble::{Assembly, assemble};
pub use batch::{Record, Summary};
pub use consolidate::{Consolidation, Diff, consolidate, diff};
pub use convert::{Conversion, conversion};
pub use health::{
    BitmapHealth, DirCacheHealth, Examined, Finding, Health, Severity, examine, examine_partition,
};
pub use inspect::{
    Compression, Description, Fault, Image, Inspection, InspectionError, MAX_DECOMPRESSED,
    MAX_DESCRIPTION, PartitionInfo, RdbInfo, TrackTable, VolumeInfo, entry_to_json,
    entry_to_json_hashed, inspect_bytes, inspect_bytes_named, inspect_path,
};

/// The layer crates, re-exported so that front-ends depend on this crate alone.
pub mod layers {
    pub use ade_block as block;
    pub use ade_catalogue as catalogue;
    pub use ade_container as container;
    pub use ade_endian as endian;
    pub use ade_filesystem as filesystem;
    pub use ade_flux as flux;
    pub use ade_object as object;
    pub use ade_track as track;
}

/// The version of the engine, as reported by front-ends.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Where a dataset lives, when the caller did not say.
///
/// # Why identification is configured rather than automatic
///
/// F-013 asks for identification *on open*. It cannot simply always happen:
/// loading 88,921 entries from 98 datfiles takes **140 ms**, and `ade info`
/// itself takes under ten. Identifying every image unconditionally would make
/// the fastest command in the tool fourteen times slower for everyone,
/// including the corpus scripts that call it thousands of times.
///
/// So it happens when a dataset is *configured*, and costs nothing when it is
/// not. In order:
///
/// 1. what the caller passed (`--datfiles=`),
/// 2. `$ADE_DATFILES`,
/// 3. `$XDG_DATA_HOME/ade/datfiles`, or `~/.local/share/ade/datfiles`.
///
/// A path that does not exist is treated as no dataset rather than as an
/// error: an unset-up machine should run ADE, not refuse to.
///
/// **Scripted use over a corpus should use `batch --datfiles=`**, which loads
/// the dataset once instead of once per image — the difference between a
/// second and thirteen minutes over 4,652 images.
#[must_use]
pub fn datfiles_location(explicit: Option<&Path>) -> Option<PathBuf> {
    let candidate = |p: PathBuf| p.is_dir().then_some(p);
    if let Some(path) = explicit {
        // An explicit path that is wrong is worth reporting, so it is returned
        // even when absent and the caller fails on it.
        return Some(path.to_path_buf());
    }
    if let Some(dir) = std::env::var_os("ADE_DATFILES")
        && let Some(found) = candidate(PathBuf::from(dir))
    {
        return Some(found);
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    candidate(base.join("ade/datfiles"))
}
