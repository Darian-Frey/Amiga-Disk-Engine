//! Object model — files, directories, links, comments, protection bits, datestamps.
//!
//! The neutral representation the UI and catalogue consume. Undelete and salvage
//! operate here (F-012).
//!
//! Content signatures (F-020) live here: recognising what bytes *are*, as
//! distinct from what a directory entry calls them.

pub mod find;
pub mod signature;
