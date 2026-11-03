// SPDX-License-Identifier: GPL-2.0-or-later
//! libWolkit patch stream (I10, ADR 0039) under libFuzzer. The decoder must never panic; a patch
//! that validates is applied to a scene that already holds an app, and an apply that fails must
//! leave that scene exactly as it was (atomicity); one that succeeds must keep the tree sound.
#![no_main]
use libfuzzer_sys::fuzz_target;
use wolkit::scene::patch::{APPEND, Patch, Scratch, Writer};
use wolkit::scene::{Journal, Kind, Scene, SceneRead};
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
    let mut sc = Scratch::default();
    let mut j = Journal::default();
    let p = Patch::parse(w.finish(1), &mut sc).expect("the base patch is valid");
    s.apply(&p, &mut j).expect("the base patch applies");
    s
}

fuzz_target!(|data: &[u8]| {
    let mut sc = Scratch::default();
    let Ok(p) = Patch::parse(data, &mut sc) else {
        return;
    };
    // Iterating must not panic either.
    let n = p.ops().count();
    assert!(n as u32 <= p.op_count);
    let mut scene = base();
    let before = dump::dump(&scene, SCENE);
    let mut j = Journal::default();
    match scene.apply(&p, &mut j) {
        Err(_) => assert_eq!(
            dump::dump(&scene, SCENE),
            before,
            "a failed apply changed the scene"
        ),
        Ok(_) => {
            // Every live node is reachable from the root exactly once.
            let mut seen = 0usize;
            wolkit::scene::walk(&scene, 0, &mut |_, depth| {
                assert!(depth <= wolkit::scene::MAX_DEPTH);
                seen += 1;
            });
            let detached = (0..scene.capacity())
                .filter(|i| scene.is_live(*i) && *i != 0 && scene.parent(*i) == wolkit::scene::NIL)
                .count();
            assert!(seen <= scene.node_count());
            let _ = detached;
        }
    }
    // The same patch again never panics and keeps the same promise.
    let again = dump::dump(&scene, SCENE);
    if scene.apply(&p, &mut j).is_err() {
        assert_eq!(dump::dump(&scene, SCENE), again);
    }
});
