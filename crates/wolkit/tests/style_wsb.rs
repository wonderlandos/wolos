// SPDX-License-Identifier: BSD-2-Clause
//! `WSB1`: exact round trips of random tables, deterministic output, and a loader that treats its
//! input as hostile (truncation at every length, bit flips with the checksum repaired, limits).
mod layout_support;
use layout_support::Rng;
use wolkit::style::prop::*;
use wolkit::style::select::*;
use wolkit::style::wsb::*;

/// A sheet text full of different values.
fn sample_sheet(r: &mut Rng, classes: u32) -> String {
    let values: &[&str] = &[
        "display: none",
        "flex-direction: row-reverse",
        "grow: 1.5",
        "basis: 25%",
        "gap: sm md",
        "padding: xs lg",
        "margin: auto md",
        "width: 100px",
        "height: 40%",
        "max-width: 600px",
        "aspect-ratio: 4 / 3",
        "position: absolute",
        "top: 3px",
        "overflow: hidden",
        "z-index: 5",
        "background: card",
        "background: linear-gradient(0deg, accent, glass)",
        "border: 1px line",
        "border-radius: m l",
        "box-shadow: md",
        "opacity: 0.4",
        "outline: 2px accent",
        "transform: translate(2px) rotate(5deg)",
        "cursor: pointer",
        "color: alpha(dim, 50%)",
        "font-size: large",
        "font-weight: 500",
        "letter-spacing: 0.5px",
        "text-align: center",
        "transition: opacity, color fast",
        "spring: gentle",
        "visibility: hidden",
        "font-feature: tnum",
        "user-select: none",
    ];
    let states = ["", ":hover", ":active", ":focus", ":disabled"];
    let media = ["", "@dark", "@contrast", "@dark @reduce-motion"];
    let mut s = String::new();
    for c in 0..classes {
        for k in 0..=r.below(3) {
            let st = if k == 0 { "" } else { r.pick(&states) };
            let md = if k == 0 { "" } else { r.pick(&media) };
            let body: Vec<&str> = (0..r.below(4) + 1).map(|_| r.pick(values)).collect();
            // Avoid the same property twice in one block (a warning, not an error, but noisy).
            let mut seen = std::collections::HashSet::new();
            let body: Vec<&str> = body
                .into_iter()
                .filter(|b| seen.insert(b.split(':').next().unwrap()))
                .collect();
            let rule = format!(".c{c}{st} {{ {}; }}", body.join("; "));
            if md.is_empty() {
                s.push_str(&rule);
            } else {
                s.push_str(&format!("{md} {{ {rule} }}"));
            }
            s.push('\n');
        }
    }
    s
}

fn compile(src: &str) -> wolkit::style::Compiled {
    wolkit::style::compile_sources(&[("t.wolcss", src)])
        .unwrap_or_else(|d| panic!("{}\n{src}", wolkit::style::render_all(&d)))
}

#[test]
fn random_tables_round_trip_exactly() {
    for seed in 1..=150u64 {
        let mut r = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let n = r.below(20) + 1;
        let src = sample_sheet(&mut r, n);
        let c = compile(&src);
        let back = load(&c.bytes).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        assert_eq!(back, c.table, "seed {seed}");
        assert_eq!(
            write(&back).unwrap(),
            c.bytes,
            "seed {seed}: the bytes are canonical"
        );
    }
}

#[test]
fn output_is_deterministic() {
    let mut r = Rng(0xabcdef);
    let src = sample_sheet(&mut r, 12);
    let a = compile(&src).bytes;
    let b = compile(&src).bytes;
    assert_eq!(a, b);
    assert_eq!(&a[..4], b"WSB1");
    assert_eq!(u16::from_le_bytes([a[4], a[5]]), VERSION);
}

fn fix_checksum(b: &mut [u8]) {
    let n = b.len();
    let sum = fnv1a(&b[..n - 4]);
    b[n - 4..].copy_from_slice(&sum.to_le_bytes());
}

#[test]
fn every_truncation_is_refused_without_panicking() {
    let mut r = Rng(77);
    let c = compile(&sample_sheet(&mut r, 6));
    for n in 0..c.bytes.len() {
        assert!(
            load(&c.bytes[..n]).is_err(),
            "a table cut at {n} of {} loaded",
            c.bytes.len()
        );
    }
    assert!(load(&c.bytes).is_ok());
}

#[test]
fn flipped_bits_never_panic_and_never_load_something_invalid() {
    let mut r = Rng(0x5eed_5eed);
    let c = compile(&sample_sheet(&mut r, 8));
    let mut loaded = 0;
    for _ in 0..4000 {
        let mut b = c.bytes.clone();
        for _ in 0..=r.below(3) {
            let i = r.below(b.len() as u32) as usize;
            b[i] ^= 1 << r.below(8);
        }
        // Without repairing the checksum, almost everything is refused by it.
        let raw = load(&b);
        if b != c.bytes {
            assert!(
                raw.is_err()
                    || b[b.len() - 4..] != c.bytes[b.len() - 4..]
                    || fnv1a(&b[..b.len() - 4])
                        == u32::from_le_bytes(b[b.len() - 4..].try_into().unwrap())
            );
        }
        // With it repaired the loader has to validate the contents itself.
        fix_checksum(&mut b);
        if let Ok(t) = load(&b) {
            loaded += 1;
            // Whatever loads is a valid table: it re-encodes and loads again to the same thing.
            let again = write(&t).unwrap();
            assert_eq!(load(&again).unwrap(), t);
            for c in t.classes.iter().chain(t.kinds.iter()) {
                assert!(c.variants.len() <= MAX_VARIANTS);
                for v in &c.variants {
                    assert!(v.state.count_ones() <= 1);
                    assert_eq!(v.decls.mask >> PropId::COUNT, 0);
                }
            }
        }
    }
    assert!(
        loaded > 0,
        "some flips land in values and still make a valid table"
    );
}

#[test]
fn random_garbage_is_refused() {
    let mut r = Rng(0xdead_beef);
    for _ in 0..2000 {
        let n = r.below(300) as usize;
        let mut b: Vec<u8> = (0..n).map(|_| r.below(256) as u8).collect();
        if r.chance(50) && n > 8 {
            b[..4].copy_from_slice(b"WSB1");
            b[4..6].copy_from_slice(&VERSION.to_le_bytes());
            fix_checksum(&mut b);
        }
        let _ = load(&b);
    }
}

#[test]
fn headers_and_limits() {
    let c = compile(".a { opacity: 1; }");
    let mut b = c.bytes.clone();
    b[0] = b'X';
    assert_eq!(load(&b), Err(WsbError::Magic));
    let mut b = c.bytes.clone();
    b[4] = 9;
    fix_checksum(&mut b);
    assert_eq!(load(&b), Err(WsbError::Version(9)));
    assert_eq!(load(&[]), Err(WsbError::Magic));
    assert_eq!(load(&vec![0u8; MAX_TABLE_BYTES + 1]), Err(WsbError::TooBig));
    // Too many classes claimed in the header.
    let mut b = c.bytes.clone();
    b[12..16].copy_from_slice(&(MAX_CLASSES as u32 + 1).to_le_bytes());
    fix_checksum(&mut b);
    assert_eq!(load(&b), Err(WsbError::Limit("classes")));
    // A wrong kind count.
    let mut b = c.bytes.clone();
    b[8..12].copy_from_slice(&3u32.to_le_bytes());
    fix_checksum(&mut b);
    assert_eq!(load(&b), Err(WsbError::Limit("kinds")));
    // Bytes after the last class.
    let mut b = c.bytes.clone();
    let n = b.len();
    b.splice(n - 4..n - 4, [0u8, 0, 0]);
    fix_checksum(&mut b);
    assert_eq!(load(&b), Err(WsbError::Trailing));
    // The checksum.
    let mut b = c.bytes.clone();
    let n = b.len();
    b[n - 1] ^= 1;
    assert_eq!(load(&b), Err(WsbError::Checksum));
    // Every error is a sentence without "failed".
    for e in [
        WsbError::TooBig,
        WsbError::Magic,
        WsbError::Version(2),
        WsbError::Truncated,
        WsbError::Checksum,
        WsbError::Limit("x"),
        WsbError::Name,
        WsbError::Value("x"),
        WsbError::Trailing,
    ] {
        let s = e.to_string();
        assert!(s.ends_with('.') && !s.contains("failed"), "{s}");
    }
}

#[test]
fn four_thousand_ninety_six_classes_load() {
    let mut src = String::new();
    for i in 0..MAX_CLASSES {
        src.push_str(&format!(".c{} {{ opacity: 1; }}\n", name(i)));
    }
    let c = compile(&src);
    assert_eq!(c.table.classes.len(), MAX_CLASSES);
    assert!(c.bytes.len() < MAX_TABLE_BYTES);
    assert_eq!(load(&c.bytes).unwrap().classes.len(), MAX_CLASSES);
    src.push_str(".more { opacity: 1; }\n");
    assert!(wolkit::style::compile_sources(&[("t.wolcss", &src)]).is_err());
}

fn name(i: usize) -> String {
    let mut s = String::new();
    let mut n = i;
    loop {
        s.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
        if n == 0 {
            break;
        }
    }
    s
}

#[test]
fn transition_masks_cannot_name_a_layout_property() {
    let c = compile(".a { transition: opacity fast; }");
    let mut b = c.bytes.clone();
    // Find the 16-byte mask of the transition value and set the `width` bit in it: patch the
    // table through the typed API and write it back instead of poking bytes.
    let mut t = load(&b).unwrap();
    t.classes[0].variants[0].decls.p.motion.transition.props |= PropId::Width.bit();
    b = write(&t).unwrap();
    assert_eq!(load(&b), Err(WsbError::Value("transition")));
}
