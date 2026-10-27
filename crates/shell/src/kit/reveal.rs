//! First surface of a compositor (ADR 0012): content fades in from the black that the loader splash,
//! the console and the compositor show, and leaves the same way, so every hand-off is black to black.
//! Login window, setup assistant and start-up lock screen use it; golden renders start it shown.
use gtk::{glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Same as `--glass-motion` in kit.css.
pub const REVEAL_MS: u32 = 320;
/// Frames the content may wait for `ready` (e.g. the blurred wallpaper) before it shows anyway.
pub const MAX_WAIT_FRAMES: u32 = 90;
/// Greeter compositors start with `cursor:invisible` (login.conf, setup.conf); the pointer appears
/// with the window, never over the black frames before it.
pub const SHOW_POINTER: [&str; 4] = ["hyprctl", "keyword", "cursor:invisible", "false"];

/// Fade-in/fade-out wrapper. Put [`Reveal::root`] as the window child.
pub struct Reveal {
    pub root: gtk::Revealer,
}

/// One-shot completion callback.
type Done = Box<dyn FnOnce()>;

impl Reveal {
    /// `from_black`: `child` starts transparent over the window's black (`window.lock-window`) and
    /// [`show_when`](Self::show_when) fades it in; else it shows at once (a lock over a running
    /// desktop). Golden renders always start shown.
    pub fn new(child: &impl IsA<gtk::Widget>, from_black: bool) -> Reveal {
        let golden = std::env::var_os("WOL_FAKE_NOW").is_some();
        let root = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::Crossfade)
            .transition_duration(REVEAL_MS)
            .reveal_child(golden || !from_black)
            .hexpand(true)
            .vexpand(true)
            .child(child)
            .build();
        root.add_css_class("kit-reveal");
        Reveal { root }
    }

    /// Fades in once the window has a frame on screen (so the fade itself is seen) and `ready()`
    /// holds, or after [`MAX_WAIT_FRAMES`]; then `shown()` runs once (pointer, boot mark).
    pub fn show_when(&self, ready: impl Fn() -> bool + 'static, shown: impl FnOnce() + 'static) {
        let frames = Cell::new(0u32);
        let shown = RefCell::new(Some(shown));
        self.root.add_tick_callback(move |r, _| {
            frames.set(frames.get() + 1);
            if frames.get() < 2 || (!ready() && frames.get() < MAX_WAIT_FRAMES) {
                return glib::ControlFlow::Continue;
            }
            r.set_reveal_child(true);
            if let Some(f) = shown.take() {
                f()
            }
            glib::ControlFlow::Break
        });
    }

    /// Fades back to black, then `done` (e.g. exit: the next compositor starts on black too).
    pub fn hide_then(&self, done: impl FnOnce() + 'static) {
        let done: Rc<RefCell<Option<Done>>> = Rc::new(RefCell::new(Some(Box::new(done))));
        let finish = move || {
            if let Some(f) = done.borrow_mut().take() {
                f()
            }
        };
        if !self.root.is_mapped() || !self.root.is_child_revealed() {
            finish();
            return;
        }
        let f2 = finish.clone();
        self.root.connect_child_revealed_notify(move |r| {
            if !r.is_child_revealed() {
                f2()
            }
        });
        // Animations off (gtk-enable-animations) or a stalled frame clock: never stay up.
        glib::timeout_add_local_once(
            std::time::Duration::from_millis(u64::from(REVEAL_MS) + 250),
            finish,
        );
        self.root.set_reveal_child(false);
    }
}

/// Shows the pointer on a greeter compositor ([`SHOW_POINTER`]).
pub fn show_pointer() {
    super::spawn(&SHOW_POINTER.map(String::from));
}

/// Fades every window's [`Reveal`] to black, then exits 0 (login window, setup assistant done).
pub fn exit_faded() {
    let reveals: Vec<gtk::Revealer> = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Window>().ok()?.child())
        .filter_map(|c| c.downcast::<gtk::Revealer>().ok())
        .filter(|r| r.has_css_class("kit-reveal"))
        .collect();
    let Some((first, rest)) = reveals.split_first() else {
        std::process::exit(0)
    };
    for r in rest {
        r.set_reveal_child(false);
    }
    Reveal {
        root: first.clone(),
    }
    .hide_then(|| std::process::exit(0));
}

/// One boot timeline line to syslog (`grep wolos-boot /var/log/messages`), next to wol-login's.
pub fn boot_mark(stage: &str) {
    if std::env::var_os("WOL_FAKE_NOW").is_some() {
        return;
    }
    let line = shared::login::boot_mark_now(stage);
    super::spawn(&["logger", "-p", "user.notice", "-t", "wolos", &line].map(String::from));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_and_motion_match_the_compositor_and_the_stylesheet() {
        assert_eq!(
            SHOW_POINTER.join(" "),
            "hyprctl keyword cursor:invisible false"
        );
        let css = include_str!("kit.css");
        assert!(css.contains(&format!("--glass-motion: {REVEAL_MS}ms;")));
    }
}
