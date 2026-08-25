//! The core library API — the single seam the CLI and the Qt6 GUI both consume.
//!
//! No engine logic lives here and none lives in the front-ends (F-002); this
//! crate wires the layers together and presents one surface. It is the only
//! crate that depends on every layer, and the only place cross-layer
//! coordination is permitted (D-003).

pub mod convert;
pub mod health;
pub mod inspect;
pub mod json;

pub use convert::{Conversion, conversion};
pub use health::{
    BitmapHealth, DirCacheHealth, Examined, Finding, Health, Severity, examine, examine_partition,
};
pub use inspect::{
    Compression, Description, Fault, Image, Inspection, InspectionError, MAX_DECOMPRESSED,
    MAX_DESCRIPTION, PartitionInfo, RdbInfo, TrackTable, VolumeInfo, entry_to_json, inspect_bytes,
    inspect_path,
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
