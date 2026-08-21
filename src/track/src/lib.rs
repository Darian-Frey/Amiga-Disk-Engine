//! MFM track codec — encode and decode, sync words, gaps.
//!
//! Presents decoded sectors upward as an [`ade_block::BlockSource`] and accepts
//! sectors for encoding downward.
//!
//! Not yet implemented — Phase 4. The internal model must be able to hold a
//! raw MFM track from the start (D-005), so this crate exists from day one
//! rather than being retrofitted.
