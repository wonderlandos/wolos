//! Mutation fuzzing of the wold call decoder and the Kit host wire (ADR 0020), deterministic so it runs
//! in every `cargo test`. Seeds are valid lines; each round flips, inserts, deletes, truncates or
//! splices bytes. Invariants: never panics; anything accepted is valid and round-trips.
//! The libFuzzer target over the same functions is `fuzz/fuzz_targets/ipc_decode.rs`.
use shared::portal::{Answer, Call};
use shared::view::{FromHost, ToHost};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const SEEDS: [&str; 24] = [
    r#"{"call":"audio.info"}"#,
    r#"{"call":"audio.set-volume","percent":40}"#,
    r#"{"call":"audio.set-muted","muted":true}"#,
    r#"{"call":"network.info"}"#,
    r#"{"call":"power.info"}"#,
    r#"{"call":"power.keep-awake","reason":"Exporting"}"#,
    r#"{"call":"panel.open","title":"Open"}"#,
    r#"{"call":"panel.save","title":"Save","name":"a.txt"}"#,
    r#"{"call":"prefs.get","key":"window.width"}"#,
    r#"{"call":"prefs.set","key":"k","value":"v"}"#,
    r#"{"call":"prefs.remove","key":"k"}"#,
    r#"{"call":"prefs.keys"}"#,
    r#"{"call":"clipboard.read"}"#,
    r#"{"call":"clipboard.write","text":"hello\nworld"}"#,
    r#"{"call":"session.answer","id":4,"data":{"text":"hi"}}"#,
    r#"{"call":"notification.replace","id":3,"title":"Two","body":"of five"}"#,
    r#"{"call":"notification.withdraw","id":3}"#,
    r#"{"op":"show","window":1,"title":"T","content":{"type":"spacer"},"menus":[{"id":2,"title":"File","entries":[{"type":"item","key":"open","label":"Open","shortcut":"cmd+o"},{"type":"separator"}]}],"toolbar":{"id":3,"tools":[{"type":"button","key":"save","label":"Save"},{"type":"spacer"}]}}"#,
    r#"{"call":"notification.post","title":"Build done","body":"3 warnings\nsee log"}"#,
    r#"{"call":"notification.post","title":"Hi"}"#,
    r#"{"op":"show","window":1,"title":"Hello","content":{"type":"stack","spacing":8,"children":[{"type":"label","text":"Hi","role":"title"},{"type":"button","id":1,"label":"OK"}]}}"#,
    r#"{"op":"close","window":3}"#,
    r#"{"event":"clicked","window":1,"widget":7}"#,
    r#"{"ok":false,"code":"permission-denied","message":"No."}"#,
];
const TOKENS: [&[u8]; 10] = [
    b"\"",
    b"{",
    b"}",
    b"[",
    b"]",
    b",",
    b":",
    b"\\u0000",
    b"\xff",
    b"\"call\":\"x\"",
];

fn mutate(rng: &mut Rng, mut b: Vec<u8>) -> Vec<u8> {
    for _ in 0..=rng.below(4) {
        let i = rng.below(b.len() + 1);
        match rng.below(5) {
            0 if i < b.len() => b[i] ^= 1 << rng.below(8),
            1 => {
                let t = TOKENS[rng.below(TOKENS.len())];
                b.splice(i..i, t.iter().copied());
            }
            2 if i < b.len() => {
                b.remove(i);
            }
            3 => b.truncate(i),
            _ => {
                let other = SEEDS[rng.below(SEEDS.len())].as_bytes();
                let j = rng.below(other.len());
                b.splice(i..i, other[j..].iter().copied());
            }
        }
    }
    b
}

fn check(input: &[u8]) {
    if let Ok(c) = Call::decode(input) {
        assert!(c.validate().is_ok());
        assert_eq!(Call::decode(c.encode().as_bytes()), Ok(c));
    }
    let text = String::from_utf8_lossy(input);
    if let Some(m) = ToHost::parse(&text) {
        assert_eq!(ToHost::parse(&m.to_line()), Some(m.clone()));
        if let ToHost::Show { content, .. } = m {
            assert!(content.validate().is_ok());
        }
    }
    if let Some(e) = FromHost::parse(&text) {
        assert_eq!(FromHost::parse(&e.to_line()), Some(e));
    }
    let a = Answer::parse(&text);
    assert_eq!(a.ok, a.code.is_none());
}

#[test]
fn decoders_survive_mutations() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let rounds = std::env::var("WOL_FUZZ_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(40_000);
    for _ in 0..rounds {
        let seed = SEEDS[rng.below(SEEDS.len())].as_bytes().to_vec();
        check(&mutate(&mut rng, seed));
    }
    for _ in 0..2_000 {
        let n = rng.below(64);
        let junk: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
        check(&junk);
    }
    // Deep nesting must be refused, not overflow the stack.
    let deep = format!(
        r#"{{"op":"show","window":1,"title":"x","content":{}{}}}"#,
        r#"{"type":"stack","children":["#.repeat(5_000),
        "]}".repeat(5_000)
    );
    check(deep.as_bytes());
}

/// libFuzzer crash `ipc_decode` (2026-10-31): without exact float parsing a slider value came back one
/// ulp off after `to_line`, so an accepted line did not round-trip.
#[test]
fn slider_values_round_trip_exactly() {
    for v in [
        "1.7976002080423562e308",
        "1.7976931348623157e308",
        "5e-324",
        "2.2250738585072011e-308",
        "0.1",
        "123456789012345678e-5",
    ] {
        let line = format!(r#"{{"event":"slid","window":1,"widget":2,"value":{v}}}"#);
        let e = FromHost::parse(&line).expect("finite value accepted");
        assert_eq!(FromHost::parse(&e.to_line()), Some(e), "{v}");
    }
}
