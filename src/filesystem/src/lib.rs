//! OFS and FFS mount logic, dostypes, directory traversal, RDB partitions.
//!
//! Consumes an [`ade_block::BlockSource`]; knows nothing of how the blocks
//! were obtained. Directory traversal must detect hash-chain loops from the
//! first commit rather than after AV-001 bites.
//!
//! Implemented so far: the [`dostype`] vocabulary, [`datestamp`] decoding, and
//! read-only [`bootblock`] and [`rootblock`] inspection. Mounting, traversal and
//! extraction are still to come.

pub mod bootblock;
pub mod datestamp;
pub mod dostype;
pub mod rootblock;
