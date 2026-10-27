//! Object model — files, directories, links, comments, protection bits, datestamps.
//!
//! The neutral representation the UI and catalogue consume. Undelete and salvage
//! operate here (F-012).
//!
//! Content signatures (F-020) live here: recognising what bytes *are*, as
//! distinct from what a directory entry calls them.

pub mod decoded;
pub mod explain;
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    clippy::too_many_lines,
    clippy::nonminimal_bool,
    reason = "a test module over every opcode word"
)]
mod explain_audit;
pub mod find;
pub mod font;
pub mod hardware;
pub mod hunk;
pub mod icon;
pub mod ilbm;
pub mod listing;
pub mod m68k;
pub mod prefs;
pub mod signature;
pub mod svx;
