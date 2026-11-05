// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-30 (the Store's index bombs and downloads of unknown size, the web
//! bridge's request lines, the hub's screenshots, the plugin pack; docs/STANDARDS.md section 9 and
//! "Origins of the 0.13 bugs"): input from outside the process has a size the code names before it
//! reads it. RR-021 already forbids `.read_to_end(` / `.read_to_string(` on a stream that is not
//! bounded; RH-301 closes the two doors it leaves open. A stream is copied with `io::copy(`
//! through `.take(N)`, and a decoder (gzip, deflate, zstd, xz) is built only in a function that
//! also caps its output with `.take(` or a counting writer, because a small file decompresses to
//! gigabytes. A copy from a file the program itself opened is local and does not count.
//! Counts of the rest are frozen per file in [`BASELINE`].
#[path = "support/rc.rs"]
mod rc;
use rc::{Hits, Row, Src, assert_ratchet, close_of, find_all, find_words, hit};

/// Debt when the rule came (2026-11-04, merged tree up to wd-store), each a reason, not a
/// forgotten cap. Only shrinks.
const BASELINE: &[Row] = &[
    // The reply past the cap is drained into `sink()` so the child never blocks on a full pipe;
    // the child runs under `reaper::run_until`'s deadline.
    ("RH-301", "crates/api/src/webapp.rs", 1, "STD-7"),
    // The member's reader goes to `Walker::member`, which counts bytes against `lim.member`.
    ("RH-301", "crates/xguard/src/walk.rs", 1, "STD-7"),
];

const IDS: &[&str] = &["RH-301"];

/// Constructors of a decoder whose output is as large as the sender chose.
const DECODERS: &[&str] = &[
    "GzDecoder::new(",
    "MultiGzDecoder::new(",
    "DeflateDecoder::new(",
    "ZlibDecoder::new(",
    "StreamingDecoder::new(",
    "xz_decompress(",
    "lzma_decompress(",
    "zstd::decode_all(",
];

/// Marks of a bound in a function: a `.take(` adaptor or a counting writer named `Capped`.
fn bounded(body: &str) -> bool {
    body.contains(".take(") || body.contains("Capped(")
}

/// The text of the function around offset `p`: from its `fn` line to the line that closes it at
/// the same indentation.
fn around(code: &str, p: usize) -> &str {
    let line_start = |i: usize| code[..i].rfind('\n').map_or(0, |n| n + 1);
    let mut i = line_start(p);
    let (start, indent) = loop {
        let l = &code[i..code[i..].find('\n').map_or(code.len(), |n| i + n)];
        let t = l.trim_start();
        if t.starts_with("fn ")
            || t.starts_with("pub fn ")
            || t.starts_with("pub(crate) fn ")
            || t.starts_with("pub(super) fn ")
        {
            break (i, l.len() - t.len());
        }
        if i == 0 {
            break (0, 0);
        }
        i = line_start(i - 1);
    };
    let close = format!("\n{}}}", " ".repeat(indent));
    let end = code[p..].find(&close).map_or(code.len(), |n| p + n);
    &code[start..end]
}

fn unbounded(s: &Src, h: &mut Hits) {
    let code = String::from_utf8_lossy(&s.code).into_owned();
    for p in find_all(&s.code, "io::copy(") {
        let open = p + "io::copy".len();
        let args = &code[open..close_of(&s.code, open)];
        let first = args.split(',').next().unwrap_or("");
        if first.contains(".take(") {
            continue;
        }
        let fun = around(&code, p);
        if fun.contains("File::open(") || fun.contains("fs::File") {
            continue;
        }
        hit(
            h,
            &s.rel,
            format!("{} io::copy(..) without .take(N)", s.at(p)),
        );
    }
    for n in DECODERS {
        for p in find_words(&s.code, n) {
            if !bounded(around(&code, p)) {
                hit(
                    h,
                    &s.rel,
                    format!("{} {n}..) with no cap on its output", s.at(p)),
                );
            }
        }
    }
}

#[test]
fn rh_301_input_from_outside_has_a_named_size() {
    let mut h = Hits::new();
    for s in rc::sources(&["crates"]) {
        unbounded(&s, &mut h);
    }
    assert_ratchet(
        "RH-301",
        "io::copy and decoders are bounded by .take(N) or a counting writer",
        BASELINE,
        &h,
    );
}

#[test]
fn rc_30_counter_counts_what_it_says() {
    let n = |code: &str| {
        let s = Src::parse("crates/x/src/a.rs", code.to_owned());
        let mut h = Hits::new();
        unbounded(&s, &mut h);
        h.values().map(Vec::len).sum::<usize>()
    };
    assert_eq!(
        n("fn f(body: R, w: &mut W) { std::io::copy(&mut body.into_reader(), w).ok(); }"),
        1,
        "a network body copied whole is the bug"
    );
    assert_eq!(
        n(
            "fn f(body: R, w: &mut W) { std::io::copy(&mut body.into_reader().take(MAX + 1), w).ok(); }"
        ),
        0
    );
    assert_eq!(
        n("fn f(a: &Path, w: &mut W) { let mut s = File::open(a)?; std::io::copy(&mut s, w)?; }"),
        0,
        "a local file is the program's own"
    );
    assert_eq!(
        n(
            "fn f(b: &[u8]) { let mut d = flate2::read::GzDecoder::new(b); let mut v = vec![]; d.read_to_end(&mut v); }"
        ),
        1,
        "a decoder with no cap is a bomb"
    );
    assert_eq!(
        n(
            "fn f(b: &[u8]) { let d = flate2::read::GzDecoder::new(b); d.take(MAX + 1).read_to_end(&mut v); }"
        ),
        0
    );
}

#[test]
fn the_doc_and_the_tests_agree() {
    let me = include_str!("rules_rc_30_bounded_input.rs");
    rc::assert_doc_agrees("rules_rc_30_bounded_input.rs", me, IDS, BASELINE);
}
