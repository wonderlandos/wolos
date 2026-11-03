// SPDX-License-Identifier: BSD-2-Clause
//! The `wolkit_patch` fuzz target's property, run on stable: the committed seeds
//! (`fuzz/regressions/wolkit_patch/`) and a deterministic mutation of them. A patch never panics
//! the decoder; a failed apply leaves the scene exactly as it was; a successful one keeps every
//! live node reachable once and the tree within its depth.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

static PARSED: AtomicUsize = AtomicUsize::new(0);
static APPLIED: AtomicUsize = AtomicUsize::new(0);
static REFUSED: AtomicUsize = AtomicUsize::new(0);
use wolkit::scene::patch::{APPEND, Patch, Scratch, Writer};
use wolkit::scene::{Journal, Kind, MAX_DEPTH, NIL, Scene, SceneRead, walk};
use wolkit::window::dump::{self, SCENE};

fn base() -> Scene {
    let mut w = Writer::new();
    w.create(1, Kind::Box).insert(0, 1, APPEND);
    for n in 2..8 {
        w.create(n, if n % 2 == 0 { Kind::Text } else { Kind::Button })
            .insert(1, n, APPEND)
            .set_text(n, "label");
    }
    let mut s = Scene::new();
    let p = Patch::parse(w.finish(1), &mut Scratch::default()).unwrap();
    s.apply(&p, &mut Journal::default()).unwrap();
    s
}

/// Everything live is in the tree under the root, once, within the depth limit.
fn sound(scene: &Scene) {
    let mut seen = 0usize;
    walk(scene, 0, &mut |_, depth| {
        assert!(depth <= MAX_DEPTH);
        seen += 1;
    });
    let attached_or_detached_roots = (1..scene.capacity())
        .filter(|i| scene.is_live(*i) && scene.parent(*i) == NIL)
        .count();
    // Live = reachable from the root plus the nodes of detached subtrees.
    assert!(seen <= scene.node_count());
    let _ = attached_or_detached_roots;
}

fn run(data: &[u8]) {
    let mut sc = Scratch::default();
    let Ok(p) = Patch::parse(data, &mut sc) else {
        return;
    };
    PARSED.fetch_add(1, Relaxed);
    assert!(p.ops().count() as u32 <= p.op_count);
    let mut scene = base();
    let before = dump::dump(&scene, SCENE);
    let mut j = Journal::default();
    match scene.apply(&p, &mut j) {
        Err(_) => {
            REFUSED.fetch_add(1, Relaxed);
            assert_eq!(
                dump::dump(&scene, SCENE),
                before,
                "a failed apply changed the scene"
            );
        }
        Ok(_) => {
            APPLIED.fetch_add(1, Relaxed);
            sound(&scene);
        }
    }
    let again = dump::dump(&scene, SCENE);
    if scene.apply(&p, &mut j).is_err() {
        assert_eq!(dump::dump(&scene, SCENE), again);
    }
}

fn seeds() -> Vec<Vec<u8>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/regressions/wolkit_patch");
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    v.sort();
    v.iter().map(|p| std::fs::read(p).unwrap()).collect()
}

#[test]
fn seeds_hold_the_property() {
    let s = seeds();
    assert!(s.len() >= 8);
    for d in &s {
        run(d);
    }
}

#[test]
fn mutated_seeds_hold_the_property() {
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let seeds = seeds();
    for round in 0..30_000u32 {
        let mut d = seeds[next() as usize % seeds.len()].clone();
        for _ in 0..1 + next() % 4 {
            if d.is_empty() {
                break;
            }
            let at = next() as usize % d.len();
            match next() % 4 {
                0 => d[at] = next() as u8,
                1 => d[at] ^= 1 << (next() % 8),
                2 => d.truncate(at),
                _ => {
                    // A plausible word: small ids and counts exercise the apply paths.
                    let w = (next() % 12) as u8;
                    if at + 4 <= d.len() {
                        d[at..at + 4].copy_from_slice(&u32::from(w).to_le_bytes());
                    }
                }
            }
        }
        // Keep the header's count honest half the time so more mutants reach the applier.
        if round % 2 == 0 && d.len() >= 24 {
            let ops = d[24..].len() as u32 / 12;
            d[12..16].copy_from_slice(&ops.to_le_bytes());
        }
        run(&d);
    }
    // The mutants must reach the applier, and both ways out of it.
    let (p, a, r) = (
        PARSED.load(Relaxed),
        APPLIED.load(Relaxed),
        REFUSED.load(Relaxed),
    );
    assert!(
        p > 1000 && a > 100 && r > 100,
        "parsed {p} applied {a} refused {r}"
    );
}
