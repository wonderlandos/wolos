// SPDX-License-Identifier: GPL-2.0-or-later
//! Between versions, new edges need explaining.
use aurlaws::{Evidence, LawId as L, audit_text, audit_text_with};

fn pkg(ver: &str, body: &str) -> String {
    format!(
        "pkgname=demo\npkgver={ver}\npkgrel=1\narch=(any)\nurl=https://example.org/demo\n{body}\n"
    )
}

const SUM: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn src(ver: &str) -> String {
    pkg(
        ver,
        &format!(
            "source=(https://example.org/demo-$pkgver.tar.gz)\nsha256sums=({SUM})\nbuild() {{ cd demo-$pkgver; make; }}"
        ),
    )
}

#[test]
fn a_version_bump_alone_adds_nothing() {
    let old = audit_text(&src("1.0"), None).graph;
    let ev = Evidence {
        previous: Some(old),
        ..Evidence::default()
    };
    let r = audit_text_with(&src("1.1"), None, &ev);
    assert!(r.findings.is_empty(), "{:#?}", r.findings);
}

#[test]
fn a_new_download_path_needs_explaining_and_an_explanation_clears_it() {
    let old = audit_text(&src("1.0"), None).graph;
    let new = src("1.1").replace(
        "build() {",
        "build() { curl -o /tmp/u.txt https://example.org/u.txt;",
    );
    let ev = Evidence {
        previous: Some(old.clone()),
        ..Evidence::default()
    };
    let r = audit_text_with(&new, None, &ev);
    let hit: Vec<_> = r
        .findings
        .iter()
        .filter(|f| f.law == L::UnexplainedNewEdge)
        .collect();
    assert_eq!(hit.len(), 1, "{:#?}", r.findings);
    assert!(
        hit[0].sentence.contains("https://example.org/u.txt"),
        "{}",
        hit[0].sentence
    );

    let ev = Evidence {
        previous: Some(old),
        explained: vec!["url:https://example.org/u.txt".into()],
        ..Evidence::default()
    };
    let r = audit_text_with(&new, None, &ev);
    assert!(
        r.findings.iter().all(|f| f.law != L::UnexplainedNewEdge),
        "{:#?}",
        r.findings
    );
}

#[test]
fn a_new_scriptlet_action_needs_explaining() {
    let old = audit_text(&src("1.0"), None).graph;
    let ev = Evidence {
        previous: Some(old),
        ..Evidence::default()
    };
    let r = audit_text_with(
        &src("1.1"),
        Some("post_install() { sh /opt/demo/setup; }\n"),
        &ev,
    );
    assert!(
        r.findings.iter().any(|f| f.law == L::UnexplainedNewEdge),
        "{:#?}",
        r.findings
    );
}

#[test]
fn a_dropped_checksum_needs_explaining() {
    let old = audit_text(&src("1.0"), None).graph;
    let new = src("1.1").replace(SUM, "SKIP");
    let ev = Evidence {
        previous: Some(old),
        ..Evidence::default()
    };
    let r = audit_text_with(&new, None, &ev);
    assert!(
        r.findings
            .iter()
            .any(|f| f.law == L::UnexplainedNewEdge && f.sentence.contains("checksum")),
        "{:#?}",
        r.findings
    );
}
