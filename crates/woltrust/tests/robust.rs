// SPDX-License-Identifier: BSD-2-Clause
//! Bounded random-input test: parsers and the check never panic on mutated or random bytes, and a
//! mutated signature or chain never turns into a trusted file. A fixed seed keeps it reproducible.
use woltrust::cert::{Cert, parse_chain};
use woltrust::crypto::Wsig;
use woltrust::vectors::{PkiFile, Vector, run};
use woltrust::{Trust, revoked};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(n.max(1)).unwrap_or(1)).unwrap_or(0)
    }
}

fn vector(name: &str) -> (PkiFile, Vector) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("vectors");
    let pki = serde_json::from_slice(&std::fs::read(dir.join("pki.json")).expect("pki.json"))
        .expect("pki parses");
    let v =
        serde_json::from_slice(&std::fs::read(dir.join(format!("{name}.json"))).expect("vector"))
            .expect("parses");
    (pki, v)
}

fn blob(pki: &PkiFile, name: &str) -> Vec<u8> {
    woltrust::b64::decode(&pki.blobs[name], 1 << 20).expect("blob decodes")
}

fn mutate(rng: &mut Rng, src: &[u8]) -> Vec<u8> {
    let mut b = src.to_vec();
    for _ in 0..=rng.below(3) {
        match rng.below(4) {
            0 if !b.is_empty() => {
                let i = rng.below(b.len());
                b[i] ^= 1 << rng.below(8);
            }
            1 if !b.is_empty() => {
                let i = rng.below(b.len());
                b.truncate(i);
            }
            2 => {
                let i = rng.below(b.len() + 1);
                b.insert(i, u8::try_from(rng.below(256)).unwrap_or(0));
            }
            _ if !b.is_empty() => {
                let i = rng.below(b.len());
                b.remove(i);
            }
            _ => {}
        }
    }
    b
}

#[test]
fn random_bytes_never_panic_a_parser() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..3000 {
        let n = rng.below(300);
        let bytes: Vec<u8> = (0..n)
            .map(|_| u8::try_from(rng.below(256)).unwrap_or(0))
            .collect();
        assert!(Cert::parse(&bytes).is_err());
        assert!(parse_chain(&bytes).is_err());
        assert!(Wsig::parse(&bytes).is_err());
        assert!(revoked::parse(&bytes).is_err());
    }
}

#[test]
fn mutated_valid_files_never_panic_or_gain_trust() {
    let (pki, v) = vector("valid-person");
    let (wsig, chain, content) = (
        blob(&pki, "wsig-alice"),
        blob(&pki, "chain-alice"),
        blob(&pki, "content"),
    );
    let mut rng = Rng(42);
    let base_store = run(&v, &pki).expect("replays").1;
    for round in 0..120 {
        let (w, c, m) = match round % 3 {
            0 => (mutate(&mut rng, &wsig), chain.clone(), content.clone()),
            1 => (wsig.clone(), mutate(&mut rng, &chain), content.clone()),
            _ => (wsig.clone(), chain.clone(), mutate(&mut rng, &content)),
        };
        let out = woltrust::verify(
            &base_store,
            &woltrust::Input {
                purpose: woltrust::Purpose::Package,
                content: &m,
                app_id: Some("alice.notes"),
                wsig: Some(&w),
                chain: Some(&c),
                first_seen: None,
                now: woltrust::pki::NOW,
            },
        );
        let unchanged = w == wsig && c == chain && m == content;
        if !unchanged {
            assert_ne!(
                out.trust,
                Trust::Verified,
                "round {round}: a changed file was trusted"
            );
        }
    }
}

#[test]
fn mutated_lists_and_certificates_never_panic() {
    let (pki, v) = vector("revoked-abuse");
    let list = woltrust::b64::decode(&v.store.lists[0].list, 1 << 20).expect("list decodes");
    let lwsig = woltrust::b64::decode(&v.store.lists[0].wsig, 1 << 20).expect("wsig decodes");
    let root = blob(&pki, "root-a");
    let alice = blob(&pki, "alice");
    let mut rng = Rng(7);
    for _ in 0..40 {
        let l = mutate(&mut rng, &list);
        let w = mutate(&mut rng, &lwsig);
        let r = mutate(&mut rng, &root);
        let a = mutate(&mut rng, &alice);
        let store = woltrust::Store::load(&[r], &[a], &[(l, w)], woltrust::pki::NOW, true);
        assert!(store.lists.is_empty() || store.lists.len() == 1);
    }
}
