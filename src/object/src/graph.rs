//! Graphs to draw (F-036): a function's control flow, and who calls whom.
//!
//! **A function's control flow** is its basic blocks — runs of instructions
//! entered only at the top and left only at the bottom — and the edges
//! between them: a branch taken, a branch not taken, a jump, falling
//! through. It is found the way [`crate::flow`] walks a function, following
//! only what the instructions state: a block ends at a branch, a jump, a
//! return, or where another block begins; a call is not an edge, since
//! control comes back. **The call graph** is [`crate::flow`]'s functions
//! and the calls between them, each library function a call reaches a leaf.
//!
//! **Laid out here, drawn elsewhere.** Each node gets a layer and a place in
//! it, the way a layered drawing (Sugiyama's, Graphviz's `dot`) does it:
//! edges that close a loop are found by a depth-first walk and set aside;
//! each node's layer is its longest path from the start; and the order in
//! each layer is settled by a few sweeps placing each node at the mean of
//! its neighbours' places. A front end only scales layers and places to
//! pixels. Bounded (D-006): a graph stops at [`MAX_NODES`] and says so.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use crate::flow::{Analysis, Place};
use crate::m68k::{Decoded, Operand, decode};

/// Nodes a graph may hold before it stops.
pub const MAX_NODES: usize = 2_000;

/// What a node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// A basic block.
    Block,
    /// The block a function starts with.
    Entry,
    /// A block control leaves the function from: a return, or a jump out.
    Exit,
    /// A function in the program.
    Function,
    /// A library function a call reaches.
    Library,
}

/// How one node leads to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// A conditional branch, taken.
    Taken,
    /// A conditional branch, not taken: on to the next instruction.
    NotTaken,
    /// A `bra` or `jmp`.
    Jump,
    /// Falling through into the next block, which begins there because
    /// something else branches to it.
    Next,
    /// A call.
    Call,
}

/// One node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Where it starts.
    pub place: Place,
    /// For a block, where it ends: the offset just past its last
    /// instruction. For anything else, its start.
    pub end: u32,
    /// What to call it: a place's label, a library function's name.
    pub label: String,
    /// What it is.
    pub kind: NodeKind,
    /// Its layer, from 0 at the top.
    pub layer: u32,
    /// Its place in the layer, from 0 at the left.
    pub order: u32,
}

/// One edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    /// The node it leaves, by index.
    pub from: usize,
    /// The node it reaches.
    pub to: usize,
    /// How.
    pub kind: EdgeKind,
    /// Whether it goes back up the drawing: it closes a loop.
    pub back: bool,
}

/// A graph, laid out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Graph {
    /// The nodes.
    pub nodes: Vec<Node>,
    /// The edges.
    pub edges: Vec<Edge>,
    /// Whether it stopped at [`MAX_NODES`].
    pub truncated: bool,
}

/// How control leaves one instruction, for splitting blocks.
enum Leaves {
    /// On to the next, only.
    On,
    /// To a target, or on.
    Branch(u32),
    /// To a target, only.
    Jump(u32),
    /// Nowhere the instruction says: a return, or a jump through a register.
    Out,
}

/// Every instruction a function starting at `start` reaches, with its
/// length and how it leaves: [`function`]'s walk.
fn reach<S: std::hash::BuildHasher>(
    code: &[u8],
    (origin, hunk): (u32, usize),
    relocs: &HashMap<u32, Place, S>,
    start: u32,
) -> BTreeMap<u32, (u32, Leaves)> {
    let inside = |a: u32| {
        a.checked_sub(origin)
            .is_some_and(|o| (o as usize) < code.len())
    };
    // Every instruction the function reaches, with its length and how it
    // leaves.
    let mut instructions: BTreeMap<u32, (u32, Leaves)> = BTreeMap::new();
    let mut queue = VecDeque::from([start]);
    while let Some(at) = queue.pop_front() {
        if instructions.contains_key(&at) || !inside(at) || instructions.len() >= MAX_NODES * 8 {
            continue;
        }
        let Some(rest) = code.get(at.saturating_sub(origin) as usize..) else {
            continue;
        };
        let Decoded::Instruction(i) = decode(rest, at) else {
            continue;
        };
        // A relocated long inside the instruction, pointing into this hunk.
        let moved = |v: u32| {
            (at.saturating_add(2)..at.saturating_add(i.len)).any(|p| {
                relocs.get(&p).is_some_and(|&(k, w)| {
                    k == hunk
                        && w == v
                        && p.checked_sub(origin).is_some_and(|o| {
                            ade_endian::u32_at(code, o as usize).is_ok_and(|x| x == v)
                        })
                })
            })
        };
        let target = i.operands.iter().find_map(|o| match o {
            Operand::Target(t) | Operand::PcDisp(t) => Some(*t),
            Operand::AbsL(v) if moved(*v) => Some(*v),
            _ => None,
        });
        let m = i.mnemonic;
        let leaves = if matches!(m, "rts" | "rte" | "rtr" | "illegal") {
            Leaves::Out
        } else if m == "bra" || m == "jmp" {
            match target.filter(|t| inside(*t)) {
                Some(t) => Leaves::Jump(t),
                None => Leaves::Out,
            }
        } else if crate::flow::is_branch(m) || crate::flow::is_db(m) {
            match target.filter(|t| inside(*t)) {
                Some(t) => Leaves::Branch(t),
                None => Leaves::On,
            }
        } else {
            Leaves::On
        };
        let next = at.saturating_add(i.len);
        match &leaves {
            Leaves::On => queue.push_back(next),
            Leaves::Branch(t) => {
                queue.push_back(*t);
                queue.push_back(next);
            }
            Leaves::Jump(t) => queue.push_back(*t),
            Leaves::Out => {}
        }
        instructions.insert(at, (i.len, leaves));
    }

    instructions
}

/// The control flow of the function starting at `start` in `code`, whose
/// first byte is address `origin`, in hunk `hunk`; `relocs` are the hunk's
/// relocations, by offset, the place each long points at, so that a jump to
/// a relocated address in the hunk is followed as the walks follow it;
/// `label` names a place.
#[must_use]
pub fn function<S: std::hash::BuildHasher>(
    code: &[u8],
    (origin, hunk): (u32, usize),
    relocs: &HashMap<u32, Place, S>,
    start: u32,
    label: &dyn Fn(Place) -> String,
) -> Graph {
    let instructions = reach(code, (origin, hunk), relocs, start);

    // Where blocks begin: the start, every target, and after every branch.
    let mut leaders: BTreeSet<u32> = BTreeSet::from([start]);
    for (&at, (len, leaves)) in &instructions {
        match leaves {
            Leaves::Branch(t) => {
                leaders.insert(*t);
                leaders.insert(at.saturating_add(*len));
            }
            Leaves::Jump(t) => {
                leaders.insert(*t);
            }
            Leaves::On | Leaves::Out => {}
        }
    }
    leaders.retain(|l| instructions.contains_key(l));

    let mut graph = Graph::default();
    let mut index: BTreeMap<u32, usize> = BTreeMap::new();
    // Each block runs from its leader to the instruction before the next
    // leader, or to where control leaves.
    let mut pending: Vec<(usize, u32, EdgeKind)> = Vec::new();
    for &leader in &leaders {
        if graph.nodes.len() >= MAX_NODES {
            graph.truncated = true;
            break;
        }
        let n = graph.nodes.len();
        index.insert(leader, n);
        let mut at = leader;
        let mut kind = if leader == start {
            NodeKind::Entry
        } else {
            NodeKind::Block
        };
        let end = loop {
            let Some((len, leaves)) = instructions.get(&at) else {
                break at;
            };
            let next = at.saturating_add(*len);
            match leaves {
                Leaves::Branch(t) => {
                    pending.push((n, *t, EdgeKind::Taken));
                    pending.push((n, next, EdgeKind::NotTaken));
                    break next;
                }
                Leaves::Jump(t) => {
                    pending.push((n, *t, EdgeKind::Jump));
                    break next;
                }
                Leaves::Out => {
                    if kind == NodeKind::Block {
                        kind = NodeKind::Exit;
                    }
                    break next;
                }
                Leaves::On => {
                    if leaders.contains(&next) {
                        pending.push((n, next, EdgeKind::Next));
                        break next;
                    }
                    if !instructions.contains_key(&next) {
                        break next;
                    }
                    at = next;
                }
            }
        };
        graph.nodes.push(Node {
            place: (hunk, leader),
            end,
            label: label((hunk, leader)),
            kind,
            layer: 0,
            order: 0,
        });
    }
    for (from, to, kind) in pending {
        if let Some(&to) = index.get(&to) {
            graph.edges.push(Edge {
                from,
                to,
                kind,
                back: false,
            });
        }
    }
    lay_out(&mut graph, 0);
    graph
}

/// Who calls whom in `a`: its functions, and each library function a call
/// reaches; `label` names a function's place.
#[must_use]
pub fn calls(a: &Analysis, label: &dyn Fn(Place) -> String) -> Graph {
    let mut graph = Graph::default();
    let mut index: BTreeMap<Place, usize> = BTreeMap::new();
    for f in &a.functions {
        if graph.nodes.len() >= MAX_NODES {
            graph.truncated = true;
            break;
        }
        index.insert(f.start, graph.nodes.len());
        graph.nodes.push(Node {
            place: f.start,
            end: f.start.1,
            label: label(f.start),
            kind: NodeKind::Function,
            layer: 0,
            order: 0,
        });
    }
    let mut libraries: BTreeMap<String, usize> = BTreeMap::new();
    for f in &a.functions {
        let Some(&from) = index.get(&f.start) else {
            continue;
        };
        for callee in &f.calls {
            if let Some(&to) = index.get(callee) {
                graph.edges.push(Edge {
                    from,
                    to,
                    kind: EdgeKind::Call,
                    back: false,
                });
            }
        }
        for site in &f.library_calls {
            let Some(c) = a.library_call(*site) else {
                continue;
            };
            let name = match c.function {
                Some(n) => format!("{} {n}", c.library.short()),
                None => c.describe(),
            };
            let to = if let Some(&n) = libraries.get(&name) {
                n
            } else {
                if graph.nodes.len() >= MAX_NODES {
                    graph.truncated = true;
                    continue;
                }
                let n = graph.nodes.len();
                graph.nodes.push(Node {
                    place: c.at,
                    end: c.at.1,
                    label: name.clone(),
                    kind: NodeKind::Library,
                    layer: 0,
                    order: 0,
                });
                libraries.insert(name, n);
                n
            };
            let edge = Edge {
                from,
                to,
                kind: EdgeKind::Call,
                back: false,
            };
            if !graph.edges.contains(&edge) {
                graph.edges.push(edge);
            }
        }
    }
    let roots: Vec<usize> = a
        .functions
        .iter()
        .filter(|f| f.entry)
        .filter_map(|f| index.get(&f.start).copied())
        .collect();
    lay_out(&mut graph, roots.first().copied().unwrap_or(0));
    graph
}

/// Give each node a layer and a place in it, marking the edges that close
/// loops as going back.
fn lay_out(graph: &mut Graph, root: usize) {
    let n = graph.nodes.len();
    if n == 0 {
        return;
    }
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (e, edge) in graph.edges.iter().enumerate() {
        if let Some(list) = out.get_mut(edge.from) {
            list.push(e);
        }
    }
    back_edges(graph, root, &out);
    let layer = layers(graph, &out);
    order(graph, &layer);
}

/// A count or a place in a layer, as a float for averaging: far below
/// where an `f64` stops holding every integer.
fn small(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// Mark the edges that close loops: those reaching a node still on a
/// depth-first stack.
fn back_edges(graph: &mut Graph, root: usize, out: &[Vec<usize>]) {
    let n = graph.nodes.len();
    // Back edges: those reaching a node still on the depth-first stack. The
    // walk starts at the root, then at anything it did not reach.
    let mut state = vec![0u8; n]; // 0 unseen, 1 on the stack, 2 done
    let mut starts: Vec<usize> = vec![root];
    starts.extend((0..n).filter(|&v| v != root));
    for s in starts {
        if state.get(s) != Some(&0) {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(s, 0)];
        if let Some(st) = state.get_mut(s) {
            *st = 1;
        }
        while let Some(&mut (v, ref mut next)) = stack.last_mut() {
            let edges = out.get(v).map_or(&[][..], Vec::as_slice);
            if let Some(&e) = edges.get(*next) {
                *next = next.saturating_add(1);
                let to = graph.edges.get(e).map_or(v, |edge| edge.to);
                match state.get(to) {
                    Some(0) => {
                        if let Some(st) = state.get_mut(to) {
                            *st = 1;
                        }
                        stack.push((to, 0));
                    }
                    Some(1) => {
                        if let Some(edge) = graph.edges.get_mut(e) {
                            edge.back = true;
                        }
                    }
                    _ => {}
                }
            } else {
                if let Some(st) = state.get_mut(v) {
                    *st = 2;
                }
                stack.pop();
            }
        }
    }
}

/// Each node's layer: its longest forward path from a node with nothing
/// above it.
fn layers(graph: &Graph, out: &[Vec<usize>]) -> Vec<u32> {
    let n = graph.nodes.len();
    // Layers: the longest path from a node with nothing above it, over the
    // forward edges, in a topological order.
    let mut indegree = vec![0usize; n];
    for edge in graph.edges.iter().filter(|e| !e.back) {
        if let Some(d) = indegree.get_mut(edge.to) {
            *d = d.saturating_add(1);
        }
    }
    let mut layer = vec![0u32; n];
    let mut ready: VecDeque<usize> = (0..n).filter(|&v| indegree.get(v) == Some(&0)).collect();
    while let Some(v) = ready.pop_front() {
        for &e in out.get(v).map_or(&[][..], Vec::as_slice) {
            let Some(edge) = graph.edges.get(e) else {
                continue;
            };
            if edge.back {
                continue;
            }
            let here = layer.get(v).copied().unwrap_or(0).saturating_add(1);
            if let Some(l) = layer.get_mut(edge.to) {
                *l = (*l).max(here);
            }
            if let Some(d) = indegree.get_mut(edge.to) {
                *d = d.saturating_sub(1);
                if *d == 0 {
                    ready.push_back(edge.to);
                }
            }
        }
    }
    layer
}

/// Each node's place in its layer.
fn order(graph: &mut Graph, layer: &[u32]) {
    let n = graph.nodes.len();
    // Order within each layer: first by where the node is in the program,
    // then by a few sweeps to the mean place of each node's neighbours.
    let layers = layer.iter().copied().max().unwrap_or(0) as usize + 1;
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); layers];
    for v in 0..n {
        if let Some(row) = layer.get(v).and_then(|&l| rows.get_mut(l as usize)) {
            row.push(v);
        }
    }
    let mut order = vec![0f64; n];
    for row in &rows {
        for (i, &v) in row.iter().enumerate() {
            if let Some(o) = order.get_mut(v) {
                *o = small(i);
            }
        }
    }
    let neighbours: Vec<Vec<usize>> = (0..n)
        .map(|v| {
            graph
                .edges
                .iter()
                .filter(|e| !e.back)
                .filter_map(|e| {
                    if e.to == v {
                        Some(e.from)
                    } else if e.from == v {
                        Some(e.to)
                    } else {
                        None
                    }
                })
                .collect()
        })
        .collect();
    for _ in 0..4 {
        for row in &mut rows {
            let mean = |v: usize| {
                let ns = neighbours.get(v).map_or(&[][..], Vec::as_slice);
                if ns.is_empty() {
                    order.get(v).copied().unwrap_or(0.0)
                } else {
                    ns.iter()
                        .map(|&u| order.get(u).copied().unwrap_or(0.0))
                        .sum::<f64>()
                        / small(ns.len())
                }
            };
            row.sort_by(|&a, &b| mean(a).total_cmp(&mean(b)));
            for (i, &v) in row.iter().enumerate() {
                if let Some(o) = order.get_mut(v) {
                    *o = small(i);
                }
            }
        }
    }
    for (l, row) in rows.iter().enumerate() {
        for (i, &v) in row.iter().enumerate() {
            if let Some(node) = graph.nodes.get_mut(v) {
                node.layer = u32::try_from(l).unwrap_or(u32::MAX);
                node.order = u32::try_from(i).unwrap_or(u32::MAX);
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    reason = "test scaffolding: code built by hand"
)]
mod tests {
    use super::*;

    fn words(w: &[u16]) -> Vec<u8> {
        w.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    #[test]
    fn a_loop_is_a_block_that_branches_back_to_itself() {
        let code = words(&[
            0x7003, // moveq #3,d0
            0x5340, // $2: subq.w #1,d0
            0x66FC, // bne.s $2
            0x4E75, // rts
        ]);
        let g = function(&code, (0, 0), &HashMap::new(), 0, &|(_, at)| {
            format!("h0_{at:04x}")
        });
        let blocks: Vec<(u32, u32, NodeKind)> =
            g.nodes.iter().map(|n| (n.place.1, n.end, n.kind)).collect();
        assert_eq!(
            blocks,
            [
                (0, 2, NodeKind::Entry),
                (2, 6, NodeKind::Block),
                (6, 8, NodeKind::Exit)
            ]
        );
        let edges: Vec<(usize, usize, EdgeKind, bool)> = g
            .edges
            .iter()
            .map(|e| (e.from, e.to, e.kind, e.back))
            .collect();
        assert_eq!(
            edges,
            [
                (0, 1, EdgeKind::Next, false),
                (1, 1, EdgeKind::Taken, true),
                (1, 2, EdgeKind::NotTaken, false)
            ]
        );
        // Laid out top to bottom.
        let layers: Vec<u32> = g.nodes.iter().map(|n| n.layer).collect();
        assert_eq!(layers, [0, 1, 2]);
    }

    #[test]
    fn an_if_makes_a_diamond() {
        let code = words(&[
            0x4A80, // tst.l d0
            0x6704, // beq.s $8
            0x7001, // moveq #1,d0
            0x6002, // bra.s $a
            0x7002, // $8: moveq #2,d0
            0x4E75, // $a: rts
        ]);
        let g = function(&code, (0, 0), &HashMap::new(), 0, &|(_, at)| {
            format!("h0_{at:04x}")
        });
        assert_eq!(g.nodes.len(), 4);
        // The two arms share a layer, side by side; the join is below both.
        let at = |a: u32| g.nodes.iter().find(|n| n.place.1 == a).unwrap();
        assert_eq!(at(4).layer, at(8).layer);
        assert_ne!(at(4).order, at(8).order);
        assert!(at(0xa).layer > at(4).layer);
        assert!(g.edges.iter().all(|e| !e.back));
    }
}
