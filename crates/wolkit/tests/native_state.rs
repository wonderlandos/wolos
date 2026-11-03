// SPDX-License-Identifier: BSD-2-Clause
//! The native backend's state machines without a compositor (`window::native`, WK-W4): frame
//! pacing (draw at most once per frame, nothing when idle, missed frames counted from
//! presentation feedback), configure sizing inside the limits, the text-input-v3 client state
//! (what to send, the commit serial rule) and the clipboard and drop type choice.

use wolkit::window::native::ime::{Out, TextInput, Want, preedit_cursor};
use wolkit::window::native::pacing::{FramePacer, FrameRecord};
use wolkit::window::native::sizing::{self, Configure};
use wolkit::window::native::{data, mime};

#[test]
fn pacing_draws_at_once_when_idle_and_once_per_frame_when_busy() {
    let mut p = FramePacer::default();
    assert!(p.request(), "idle: the first change draws at once");
    p.drew(FrameRecord::default());
    assert!(
        !p.request() && !p.request(),
        "changes during the frame wait"
    );
    assert!(p.on_done(), "the callback draws what waited");
    p.drew(FrameRecord::default());
    assert!(!p.on_done(), "a callback with nothing new asks for nothing");
    assert!(
        !p.is_waiting() && p.request(),
        "and the next change draws at once again"
    );
    p.drew(FrameRecord::default());
    p.set_continuous(true);
    assert!(p.on_done(), "an animation keeps frames coming");
    assert_eq!(p.drawn, 3);
    assert_eq!(p.frames().len(), 3);
}

#[test]
fn presentation_feedback_learns_refresh_and_counts_missed_frames() {
    let mut p = FramePacer::default();
    let ms = 1_000_000u64;
    p.request();
    p.drew(FrameRecord::default());
    p.presented(100 * ms, 8_333_333);
    assert_eq!(p.refresh_ns, 8_333_333);
    // A change during that frame: the callback draws the next one right away (back to back).
    p.request();
    assert!(p.on_done());
    p.drew(FrameRecord::default());
    // Back to back, then the next present is three intervals later: two frames missed.
    p.presented(100 * ms + 3 * 8_333_333, 8_333_333);
    assert_eq!(p.missed, 2);
    assert_eq!(
        p.next_vblank(100 * ms + 3 * 8_333_333 + 1),
        100 * ms + 4 * 8_333_333
    );
}

#[test]
fn configure_sizes_are_held_inside_the_limits_unless_tiled() {
    let c = |w: i32, h: i32| Configure {
        width: w,
        height: h,
        ..Default::default()
    };
    let (min, max) = ((200, 100), (800, 600));
    assert_eq!(
        sizing::size_after(&c(0, 0), (320, 240), min, max),
        (320, 240),
        "0 keeps ours"
    );
    assert_eq!(
        sizing::size_after(&c(100, 50), (320, 240), min, max),
        (200, 100)
    );
    assert_eq!(
        sizing::size_after(&c(1000, 900), (320, 240), min, max),
        (800, 600)
    );
    let full = Configure {
        fullscreen: true,
        ..c(1920, 1080)
    };
    assert_eq!(
        sizing::size_after(&full, (320, 240), min, max),
        (1920, 1080)
    );
    let st: Vec<u8> = [1u32, 4].iter().flat_map(|v| v.to_ne_bytes()).collect();
    let parsed = Configure::with_states(10, 20, &st);
    assert!(parsed.maximized && parsed.activated && !parsed.fullscreen);
    assert_eq!(
        sizing::limits((10, 20), (0, 0), Some((300, 200))),
        ((300, 200), (300, 200))
    );
    assert_eq!(sizing::device(301, 1.5), 452);
}

#[test]
fn text_input_enables_after_enter_and_answers_only_current_serials() {
    let mut t = TextInput::default();
    let want = Want {
        on: true,
        surrounding: Some(("ab".into(), 2, 2)),
        rect: Some([1, 2, 3, 4]),
        ..Default::default()
    };
    let mut out = Vec::new();
    t.update(&want, &mut out);
    assert!(
        out.is_empty(),
        "nothing before the input method enters the surface"
    );
    t.entered = true;
    t.update(&want, &mut out);
    assert_eq!(out[0], Out::Enable);
    assert_eq!(out.last(), Some(&Out::Commit));
    assert_eq!(t.commits(), 1);
    out.clear();
    t.update(&want, &mut out);
    assert!(out.is_empty(), "the same state is not sent twice");
    let moved = Want {
        surrounding: Some(("abc".into(), 3, 3)),
        ..want.clone()
    };
    assert!(!t.done(7, &moved, &mut out), "an old serial gets no answer");
    assert!(t.done(1, &moved, &mut out));
    assert_eq!(
        out,
        vec![
            Out::Surrounding {
                text: "abc".into(),
                cursor: 3,
                anchor: 3
            },
            Out::Commit
        ]
    );
    out.clear();
    t.update(&Want::default(), &mut out);
    assert_eq!(out, vec![Out::Disable, Out::Commit]);
    assert_eq!(preedit_cursor("añb", 1, 3), Some((1, 3)));
    assert_eq!(preedit_cursor("añb", 2, 2), None, "inside a character");
    assert_eq!(preedit_cursor("ab", -1, -1), None, "hidden");
}

#[test]
fn paste_and_drop_types_are_chosen_and_bounded() {
    let offered = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        mime::text_type(&offered(&["STRING", "text/plain;charset=utf-8"])),
        Some("text/plain;charset=utf-8")
    );
    assert_eq!(mime::text_type(&offered(&["image/png"])), None);
    assert_eq!(
        mime::drop_type(&offered(&["text/plain", "text/uri-list"])),
        Some("text/uri-list")
    );
    assert_eq!(mime::drop_type(&offered(&["image/png"])), Some("image/png"));
    let mut list = Vec::new();
    for i in 0..100 {
        mime::keep(&mut list, &format!("type/{i}"));
    }
    mime::keep(&mut list, &"x".repeat(300));
    assert_eq!(list.len(), mime::MAX_TYPES);
    let big = vec![b'a'; mime::MAX_PASTE + 10];
    assert_eq!(mime::paste_text(&big).len(), mime::MAX_PASTE);
    assert_eq!(mime::paste_text(b"ok\xff"), "ok\u{fffd}");
    let (mut r, w) = data::pair().unwrap();
    let (head, tail) = data::drop_payload(w, 1.0, 2.0, "text/plain");
    assert_eq!(head.len(), 16);
    assert_eq!(tail, b"text/plain");
    // The descriptor in the record is live (the app owns it); the reader end is nonblocking.
    use std::io::Read;
    let mut b = [0u8; 1];
    assert!(matches!(r.read(&mut b), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock));
    let fd = i32::from_le_bytes(head[0..4].try_into().unwrap());
    assert!(fd >= 0);
}
