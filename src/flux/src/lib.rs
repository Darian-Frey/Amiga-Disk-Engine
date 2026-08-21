//! Flux layer — SCP, extended-ADF, optional IPF-read, and hardware.
//!
//! All Greaseweazle interaction is confined here, so the rest of the engine is
//! testable without a device attached. SCP is the open write target; IPF is
//! read-only, optional, and licence-gated (D-007, C-003).
//!
//! Not yet implemented — Phase 4.
