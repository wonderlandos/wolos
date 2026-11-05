// SPDX-License-Identifier: GPL-2.0-or-later
//! The wire fixtures shared byte for byte with the browser's host component
//! (`crates/shared/tests/fixtures/atlas/`): every valid line decodes and encodes back to the same
//! bytes, every invalid one is refused.
use shared::atlas::{Reply, Request};
use std::path::{Path, PathBuf};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../shared/tests/fixtures/atlas")
}

fn lines(name: &str) -> Vec<String> {
    let t = std::fs::read_to_string(dir().join(name)).unwrap();
    assert!(t.ends_with('\n'), "{name} ends with a newline");
    t.lines().map(|l| format!("{l}\n")).collect()
}

#[test]
fn valid_requests_round_trip() {
    let all = lines("requests.valid.jsonl");
    assert!(all.len() >= 6);
    for l in all {
        let r = Request::decode(l.as_bytes()).unwrap_or_else(|e| panic!("{l}: {e:?}"));
        assert_eq!(r.encode(), l);
    }
}

#[test]
fn invalid_requests_are_refused() {
    let all = lines("requests.invalid.jsonl");
    assert!(all.len() >= 20);
    for l in all {
        assert!(Request::decode(l.as_bytes()).is_err(), "accepted {l}");
    }
}

#[test]
fn valid_replies_round_trip() {
    let all = lines("replies.valid.jsonl");
    for l in all {
        let r = Reply::decode(l.as_bytes()).unwrap_or_else(|| panic!("refused {l}"));
        assert_eq!(r.encode(), l);
    }
}

#[test]
fn invalid_replies_are_refused() {
    for l in lines("replies.invalid.jsonl") {
        assert!(Reply::decode(l.as_bytes()).is_none(), "accepted {l}");
    }
}

/// Every reply the bridge can send is in the fixtures, so the browser side tests each one.
#[test]
fn every_text_the_bridge_sends_is_a_fixture() {
    use shared::atlas::{ErrorCode as C, TEXT_UNREACHABLE, Verb, text};
    let replies = std::fs::read_to_string(dir().join("replies.valid.jsonl")).unwrap();
    let mut texts: Vec<&str> = vec![TEXT_UNREACHABLE];
    for v in [Verb::Share, Verb::OpenWith, Verb::Trash] {
        for c in [
            C::InvalidRequest,
            C::Version,
            C::NotBrowser,
            C::PermissionDenied,
            C::NotFound,
            C::Busy,
            C::Failed,
        ] {
            texts.push(text(v, c));
        }
        if v != Verb::OpenWith {
            texts.push(text(v, C::Unavailable));
        }
    }
    for t in texts {
        assert!(
            replies.contains(&format!("\"text\":\"{t}\"")),
            "no fixture for {t}"
        );
    }
}
