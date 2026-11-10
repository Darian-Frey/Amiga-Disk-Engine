//! Drawn graphs on real programs (F-036): every function's control flow,
//! and every program's call graph, laid out.
//!
//! Two properties the drawing depends on. **The layout goes downhill:** a
//! forward edge reaches a lower layer, and only an edge closing a loop goes
//! back up — or the picture has arrows pointing every way. **The blocks are
//! the function:** a function's blocks hold the instructions the walk of
//! F-036 reached in it, found by two walks written apart. Measured
//! 2026-11-10 over every 10th corpus program; local only.

#![allow(
    clippy::unwrap_used,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::indexing_slicing,
    clippy::many_single_char_names,
    reason = "test scaffolding"
)]

use std::path::PathBuf;

use ade_core::Image;
use ade_core::layers::object::{flow, graph, hunk};

#[test]
fn every_graph_goes_downhill_and_every_functions_blocks_hold_its_code() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let (mut seen, mut programs, mut functions, mut agree) = (0usize, 0usize, 0usize, 0usize);
    let mut differ = Vec::new();
    for path in &paths {
        let Ok(image) = Image::open(path) else {
            continue;
        };
        let Ok(volume) = image.volume() else { continue };
        let Ok(walk) = volume.walk(volume.root()) else {
            continue;
        };
        for (name, entry) in walk.entries.iter().filter(|(_, e)| e.kind.is_file()) {
            let Ok(c) = volume.read_file(entry) else {
                continue;
            };
            let bytes = c.into_bytes();
            if !hunk::is_executable(&bytes) {
                continue;
            }
            seen += 1;
            if seen % 10 != 0 {
                continue;
            }
            let (Some(a), Some(exe)) = (flow::executable(&bytes), hunk::parse(&bytes)) else {
                continue;
            };
            programs += 1;
            let label = |(k, at): flow::Place| format!("h{k}_{at:x}");
            let calls = graph::calls(&a, &label);
            for e in &calls.edges {
                let (from, to) = (&calls.nodes[e.from], &calls.nodes[e.to]);
                assert!(
                    e.back || to.layer > from.layer,
                    "{}: {name}: call graph edge uphill",
                    path.display()
                );
            }
            for f in &a.functions {
                let h = &exe.hunks[f.start.0];
                let code = &bytes[h.file_offset..h.file_offset + h.file_len];
                let relocs: std::collections::HashMap<u32, flow::Place> = h
                    .relocs
                    .iter()
                    .filter(|r| (r.target as usize) < exe.hunks.len())
                    .filter_map(|r| {
                        let v = ade_core::layers::endian::u32_at(code, r.offset as usize).ok()?;
                        Some((r.offset, (r.target as usize, v)))
                    })
                    .collect();
                let g = graph::function(code, (0, f.start.0), &relocs, f.start.1, &label);
                for e in &g.edges {
                    let (from, to) = (&g.nodes[e.from], &g.nodes[e.to]);
                    assert!(
                        if e.back {
                            to.layer <= from.layer
                        } else {
                            to.layer > from.layer
                        },
                        "{}: {name}: {:?}",
                        path.display(),
                        f.start
                    );
                }
                functions += 1;
                // The instructions the blocks hold, against the walk's count.
                let mut held = 0usize;
                for b in &g.nodes {
                    let mut at = b.place.1;
                    while at < b.end {
                        let rest = &code[at as usize..];
                        let d = ade_core::layers::object::m68k::decode(rest, at);
                        held += 1;
                        at += d.len();
                    }
                }
                if held == f.instructions {
                    agree += 1;
                } else if differ.len() < 8 {
                    differ.push(format!(
                        "{}: {name}: {:?}: blocks {held}, walk {}",
                        path.display(),
                        f.start,
                        f.instructions
                    ));
                }
            }
        }
    }
    eprintln!(
        "{programs} programs, {functions} functions, {agree} with blocks holding exactly the walk's code"
    );
    eprintln!("{differ:#?}");
    assert!(programs > 0);
    assert!(agree * 100 >= functions * 99, "{agree} of {functions}");
}
