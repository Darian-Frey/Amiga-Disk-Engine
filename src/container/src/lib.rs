//! Container front-end — normalises ADF, ADZ, HDF, HDZ, and DMS into the block layer.
//!
//! Sits beside the pipeline rather than in it: dispatch is by content sniffing
//! rather than file extension (F-003), and the upper layers never see
//! compression or wrapping. Implements [`ade_block::BlockSource`].
//!
//! Not yet implemented. Phase 1 brings plain ADF and ADZ; Phase 3 brings DMS,
//! whose route is still open (D-009).
