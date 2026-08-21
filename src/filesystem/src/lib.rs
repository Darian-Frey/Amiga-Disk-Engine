//! OFS and FFS mount logic, dostypes, directory traversal, RDB partitions.
//!
//! Consumes an [`ade_block::BlockSource`]; knows nothing of how the blocks
//! were obtained. Directory traversal must detect hash-chain loops from the
//! first commit rather than after AV-001 bites.
//!
//! Only the dostype vocabulary exists so far — see [`dostype`].

pub mod dostype;
