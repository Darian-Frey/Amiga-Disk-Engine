//! `ade` — the command-line front-end.
//!
//! Deliberately thin: every capability lives behind [`ade_core`] so that the
//! CLI and the GUI share one engine (F-002). The scriptable surface (F-015)
//! is specified from Phase 1 onward — stable exit codes and structured output
//! are a commitment, not a convenience.

fn main() {
    println!("ade {} — Amiga Disk Engine", ade_core::version());
    println!();
    println!("No commands yet: the engine is a scaffold (Phase 1 not started).");
    println!("See Docs/ROADMAP.md for what lands first.");
}
