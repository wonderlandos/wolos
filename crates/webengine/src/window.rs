// SPDX-License-Identifier: BSD-2-Clause
//! The window side of a system web app's host (`wol-webhost`, ADR 0048): the Wolkit engine the
//! window lives in, and the one `surface` node the page paints into. Kept out of the binary so a
//! test can drive both without a display.
use std::sync::Arc;
use wolkit::engine::Engine;
use wolkit::event::{bit, kind};
use wolkit::scene::Kind;
use wolkit::scene::patch::{APPEND, Writer};
use wolkit::scene::prop::{DIM_PERCENT, id};
use wolkit::window::native;

/// The Wolkit node of the page.
pub const SURFACE: u32 = 1;

/// The events the page's surface node asks for.
pub const EVENTS: u32 = bit(kind::CLICK)
    | bit(kind::POINTER_ENTER)
    | bit(kind::POINTER_LEAVE)
    | bit(kind::KEY)
    | bit(kind::SCROLL)
    | bit(kind::FOCUS)
    | bit(kind::SURFACE_RESIZED);

/// The patch that gives the window one `surface` node filling it.
pub fn surface_patch() -> Vec<u8> {
    let full = [DIM_PERCENT, 100f32.to_bits()];
    let mut p = Writer::new();
    p.create(SURFACE, Kind::Surface).insert(0, SURFACE, APPEND);
    p.set_prop(SURFACE, id::WIDTH, &full);
    p.set_prop(SURFACE, id::HEIGHT, &full);
    // What [`crate::input::bridge`] turns into page input. A node binds no event by default, so
    // without this Wolkit sends the host no key, click, hover, focus or size record for the page.
    p.bind_event(SURFACE, EVENTS);
    // The surface takes the keyboard from the start: the window is the page.
    p.set_prop(SURFACE, id::FOCUSABLE, &[1]);
    p.focus(SURFACE);
    p.finish(1).to_vec()
}

/// The session the native backend runs in, from the environment (`var`): the locale for compose
/// (`LC_ALL`, `LC_CTYPE` or `LANG`, the first set) and the launcher's activation token, which
/// raises the window once.
pub fn session(var: impl Fn(&str) -> Option<String>) -> native::Session {
    let text = |k: &str| var(k).filter(|v| !v.is_empty());
    native::Session {
        locale: ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .find_map(|k| text(k))
            .unwrap_or_default(),
        activation_token: text("XDG_ACTIVATION_TOKEN"),
    }
}

/// The Wolkit engine for the host's window. The host calls `Engine::new` itself instead of going
/// through the C entry point (`wolkit_engine_create`), which is what reads `WOLKIT_BACKEND` and
/// the session: so it has to opt in to the native Wayland backend (until then Wolkit picks the
/// Kit backend and `create_window` fails with "The WolOS desktop is not running") and set the
/// session itself (from `var`, the environment as a lookup: the host binary passes the process's).
pub fn native_engine(
    app_id: &str,
    var: impl Fn(&str) -> Option<String>,
) -> Result<Arc<Engine>, String> {
    native::set_opt_in(true);
    native::set_session(session(var));
    Engine::new(app_id, 0).map_err(|e| e.message)
}

/// How long the host's loop may sleep in `Engine::dispatch`: forever (`-1`) when nothing but a
/// wake-up needs it (window input, the engine's `on_wake`, a worker's reply), or until the next
/// time something looks at the clock. `due` is the time left until then, when there is such a
/// thing (a person's consent file is looked at every few seconds); an overdue one is `0`.
///
/// Why: the loop used to wait at most 8 ms per turn, so an idle app woke 125 times a second.
pub fn wait_ms(due: Option<std::time::Duration>) -> i32 {
    match due {
        None => -1,
        // Rounded up: waking a hair early only to sleep again costs a turn.
        Some(d) => i32::try_from(d.as_micros().div_ceil(1000)).unwrap_or(i32::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_loop_sleeps_until_woken_unless_the_clock_needs_it() {
        assert_eq!(wait_ms(None), -1, "forever: no 8 ms poll");
        assert_eq!(wait_ms(Some(Duration::ZERO)), 0);
        assert_eq!(wait_ms(Some(Duration::from_millis(1500))), 1500);
        assert_eq!(wait_ms(Some(Duration::from_micros(1))), 1, "rounded up");
        assert_eq!(wait_ms(Some(Duration::from_secs(u64::MAX / 4))), i32::MAX);
    }

    #[test]
    fn the_session_is_the_first_locale_set_and_the_launchers_token() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        let s = session(env(&[
            ("LANG", "pt_BR.UTF-8"),
            ("LC_CTYPE", ""),
            ("LC_ALL", ""),
        ]));
        assert_eq!(s.locale, "pt_BR.UTF-8", "empty values do not count");
        assert_eq!(s.activation_token, None);
        let s = session(env(&[
            ("LC_ALL", "C.UTF-8"),
            ("LANG", "x"),
            ("XDG_ACTIVATION_TOKEN", "tok"),
        ]));
        assert_eq!(s.locale, "C.UTF-8");
        assert_eq!(s.activation_token.as_deref(), Some("tok"));
    }
}
