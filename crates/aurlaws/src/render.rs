// SPDX-License-Identifier: GPL-2.0-or-later
//! The graph as DOT and as a self-contained SVG. Layers follow the flow, left to right, and a
//! finding's path is drawn in red. Labels come from hostile files, so everything is escaped.
use crate::graph::{EdgeKind as E, Graph, NodeKind as K};
use crate::laws::Finding;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;

/// Past this many nodes only the findings and what touches them are drawn.
pub const MAX_NODES: usize = 400;
const RED: &str = "#b3261e";
const LABEL: usize = 30;
const W: i64 = 220;
const H: i64 = 46;
const GAP_X: i64 = 90;
const GAP_Y: i64 = 16;

fn kind(k: K) -> &'static str {
    match k {
        K::Source => "source",
        K::Url => "address",
        K::File => "file",
        K::Env => "variable",
        K::Command => "command",
        K::Function => "function",
        K::Process => "process",
        K::Socket => "socket",
    }
}

fn verb(k: E) -> &'static str {
    match k {
        E::Fetches => "fetches",
        E::Writes => "writes",
        E::Reads => "reads",
        E::Executes => "executes",
        E::Decodes => "decodes",
        E::Sends => "sends",
        E::Verifies => "verifies",
        E::Calls => "calls",
    }
}

fn short(s: &str) -> String {
    let t: String = s.chars().filter(|c| !c.is_control()).take(LABEL).collect();
    if s.chars().count() > LABEL {
        format!("{t}...")
    } else {
        t
    }
}

fn dot_text(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .flat_map(|c| {
            if matches!(c, '"' | '\\') {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect::<String>()
        .replace('<', "\\<")
}

fn xml(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '&' => o.push_str("&amp;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c if !c.is_control() => o.push(c),
            _ => {}
        }
    }
    o
}

struct Picture {
    keep: Vec<u32>,
    hot_nodes: HashSet<u32>,
    hot_edges: HashSet<(u32, u32)>,
    cut: usize,
}

/// Which nodes to draw. Structure edges and lone nodes say nothing, so they go first.
fn pick(g: &Graph, f: &[Finding]) -> Picture {
    let ids: HashMap<&str, u32> = g
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.key.as_str(), i as u32))
        .collect();
    let (mut hot_nodes, mut hot_edges) = (HashSet::new(), HashSet::new());
    for x in f {
        let path: Vec<u32> = x
            .path
            .iter()
            .filter_map(|s| ids.get(s.node.as_str()).copied())
            .collect();
        hot_nodes.extend(path.iter().copied());
        hot_edges.extend(path.windows(2).map(|w| (w[0], w[1])));
    }
    let mut keep: Vec<u32> = (0..g.nodes.len() as u32).collect();
    let mut cut = 0;
    if keep.len() > MAX_NODES {
        let mut set: HashSet<u32> = hot_nodes.clone();
        for e in g
            .edges
            .iter()
            .filter(|e| hot_nodes.contains(&e.src) || hot_nodes.contains(&e.dst))
        {
            if set.len() >= MAX_NODES / 2 {
                break;
            }
            set.insert(e.src);
            set.insert(e.dst);
        }
        for i in 0..g.nodes.len() as u32 {
            if set.len() >= MAX_NODES / 2 {
                break;
            }
            set.insert(i);
        }
        keep.retain(|i| set.contains(i));
        cut = g.nodes.len() - keep.len();
    }
    Picture {
        keep,
        hot_nodes,
        hot_edges,
        cut,
    }
}

/// The DOT text, for Graphviz.
pub fn dot(g: &Graph, f: &[Finding]) -> String {
    let p = pick(g, f);
    let kept: HashSet<u32> = p.keep.iter().copied().collect();
    let mut o = String::from(
        "digraph aurlaws {\n  rankdir=LR;\n  node [shape=box, fontname=\"sans-serif\"];\n",
    );
    if p.cut > 0 {
        let _ = writeln!(o, "  label=\"{} nodes not drawn\";", p.cut);
    }
    for &i in &p.keep {
        let n = &g.nodes[i as usize];
        let extra = if p.hot_nodes.contains(&i) {
            format!(", color=\"{RED}\", penwidth=2")
        } else {
            String::new()
        };
        let _ = writeln!(
            o,
            "  n{i} [label=\"{}\\n{}\"{extra}];",
            kind(n.kind),
            dot_text(&short(&n.label))
        );
    }
    for e in g
        .edges
        .iter()
        .filter(|e| e.kind != E::Calls && kept.contains(&e.src) && kept.contains(&e.dst))
    {
        let extra = if p.hot_edges.contains(&(e.src, e.dst)) {
            format!(", color=\"{RED}\", penwidth=2")
        } else {
            String::new()
        };
        let _ = writeln!(
            o,
            "  n{} -> n{} [label=\"{}\"{extra}];",
            e.src,
            e.dst,
            verb(e.kind)
        );
    }
    o.push_str("}\n");
    o
}

/// Each node's column: as far right as what flows into it, so a path reads left to right.
fn ranks(g: &Graph, keep: &[u32]) -> HashMap<u32, i64> {
    let mut rank: HashMap<u32, i64> = keep.iter().map(|i| (*i, 0)).collect();
    let edges: Vec<(u32, u32)> = g
        .edges
        .iter()
        .filter(|e| {
            e.kind != E::Calls
                && e.kind != E::Sends
                && rank.contains_key(&e.src)
                && rank.contains_key(&e.dst)
        })
        .map(|e| (e.src, e.dst))
        .collect();
    // A pass per column at most; a loop in the graph must not push columns forever.
    for _ in 0..12 {
        let mut moved = false;
        for (a, b) in &edges {
            let (ra, rb) = (rank[a], rank[b]);
            if rb <= ra && ra < 11 {
                rank.insert(*b, ra + 1);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    rank
}

/// The picture itself, needing nothing else to display.
pub fn svg(g: &Graph, f: &[Finding]) -> String {
    let p = pick(g, f);
    let rank = ranks(g, &p.keep);
    let mut cols: BTreeMap<i64, Vec<u32>> = BTreeMap::new();
    for &i in &p.keep {
        cols.entry(rank[&i]).or_default().push(i);
    }
    let mut at: HashMap<u32, (i64, i64)> = HashMap::new();
    let top = if p.cut > 0 { 30 } else { 8 };
    for (c, nodes) in &cols {
        for (r, i) in nodes.iter().enumerate() {
            at.insert(*i, (8 + c * (W + GAP_X), top + r as i64 * (H + GAP_Y)));
        }
    }
    let width = at.values().map(|p| p.0 + W + 8).max().unwrap_or(240);
    let height = at.values().map(|p| p.1 + H + 8).max().unwrap_or(60);
    let mut o = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {width} {height}\" width=\"{width}\" height=\"{height}\" font-family=\"sans-serif\" font-size=\"12\">\n"
    );
    let _ = writeln!(
        o,
        "<defs><marker id=\"a\" markerWidth=\"8\" markerHeight=\"6\" refX=\"8\" refY=\"3\" orient=\"auto\"><path d=\"M0 0L8 3L0 6z\" fill=\"#777\"/></marker><marker id=\"h\" markerWidth=\"8\" markerHeight=\"6\" refX=\"8\" refY=\"3\" orient=\"auto\"><path d=\"M0 0L8 3L0 6z\" fill=\"{RED}\"/></marker></defs>"
    );
    if p.cut > 0 {
        let _ = writeln!(
            o,
            "<text x=\"8\" y=\"18\" fill=\"#555\">{} nodes not drawn; what the findings touch is kept.</text>",
            p.cut
        );
    }
    let mut drawn = HashSet::new();
    for e in g.edges.iter().filter(|e| e.kind != E::Calls) {
        let (Some(a), Some(b)) = (at.get(&e.src), at.get(&e.dst)) else {
            continue;
        };
        if !drawn.insert((e.src, e.dst, e.kind as u8)) {
            continue;
        }
        let (x1, y1, x2, y2) = (a.0 + W, a.1 + H / 2, b.0, b.1 + H / 2);
        let bend = (x2 - x1).abs().max(40) / 2;
        let hot = p.hot_edges.contains(&(e.src, e.dst));
        let (colour, mark, sw) = if hot { (RED, "h", 2) } else { ("#999", "a", 1) };
        let _ = writeln!(
            o,
            "<path d=\"M{x1} {y1}C{} {y1} {} {y2} {x2} {y2}\" fill=\"none\" stroke=\"{colour}\" stroke-width=\"{sw}\" marker-end=\"url(#{mark})\"><title>{}</title></path>",
            x1 + bend,
            x2 - bend,
            verb(e.kind)
        );
    }
    for &i in &p.keep {
        let n = &g.nodes[i as usize];
        let (x, y) = at[&i];
        let hot = p.hot_nodes.contains(&i);
        let (stroke, sw) = if hot { (RED, 2) } else { ("#888", 1) };
        let place = n
            .loc
            .as_ref()
            .map(|l| format!(" at {l}"))
            .unwrap_or_default();
        let _ = writeln!(
            o,
            "<g><title>{}{}</title><rect x=\"{x}\" y=\"{y}\" width=\"{W}\" height=\"{H}\" rx=\"6\" fill=\"#fff\" stroke=\"{stroke}\" stroke-width=\"{sw}\"/><text x=\"{}\" y=\"{}\" fill=\"#666\" font-size=\"10\">{}</text><text x=\"{}\" y=\"{}\" fill=\"#111\">{}</text></g>",
            xml(&n.label),
            xml(&place),
            x + 8,
            y + 15,
            kind(n.kind),
            x + 8,
            y + 33,
            xml(&short(&n.label))
        );
    }
    o.push_str("</svg>\n");
    o
}
