//! Flux layer — SCP, and the conversion from flux to bits.
//!
//! A flux image records the intervals between magnetic transitions rather than
//! any decoded content, so this layer's job is to hand an MFM bit stream up to
//! `ade-track`, which already knows how to find sectors in one. Nothing here
//! knows what a sector is.
//!
//! Greaseweazle interaction will live here too, so the rest of the engine
//! stays testable without a device attached. SCP is the open write target; IPF
//! is read-only, optional, and licence-gated (D-007, C-003).

pub mod mfm;
pub mod scp;
