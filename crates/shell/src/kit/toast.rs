//! Toast: one short line at the bottom of an AppWindow with at most one action ("Theme changed ·
//! Undo"). It floats over the content (no reflow), replaces the previous toast, and leaves on its
//! own after [`TOAST_MS`]. Style: kit.css "Toast".
use super::widgets::{self as w, AppWindow, Kind, S3};
use gtk::{glib, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

/// How long a toast stays when nobody uses it.
pub const TOAST_MS: u64 = 6000;

type Action = Rc<RefCell<Option<Box<dyn Fn()>>>>;

pub struct Toast {
    pub revealer: gtk::Revealer,
    text: gtk::Label,
    button: gtk::Button,
    action: Action,
    timer: Rc<RefCell<Option<glib::SourceId>>>,
}

impl Toast {
    /// A hidden toast over `aw`'s content.
    pub fn new(aw: &AppWindow) -> Rc<Self> {
        let text = w::label("", "body");
        let action: Action = Rc::default();
        let button = {
            let a = action.clone();
            w::button("", Kind::Plain, move || {
                if let Some(f) = a.borrow().as_ref() {
                    f();
                }
            })
        };
        let pill = w::hbox(S3);
        pill.add_css_class("toast");
        pill.append(&text);
        pill.append(&button);
        let revealer = gtk::Revealer::builder()
            .child(&pill)
            .transition_type(gtk::RevealerTransitionType::Crossfade)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::End)
            .can_target(true)
            .build();
        w::content_overlay(aw).add_overlay(&revealer);
        Rc::new(Toast {
            revealer,
            text,
            button,
            action,
            timer: Rc::default(),
        })
    }

    /// Shows `text` with an `action` button (none when `action` is empty) running `then`.
    pub fn show(self: &Rc<Self>, text: &str, action: &str, then: impl Fn() + 'static) {
        self.text.set_label(text);
        self.button.set_label(action);
        self.button.set_visible(!action.is_empty());
        let me = Rc::downgrade(self);
        *self.action.borrow_mut() = Some(Box::new(move || {
            if let Some(t) = me.upgrade() {
                t.hide();
            }
            then();
        }));
        self.revealer.set_reveal_child(true);
        if let Some(old) = self.timer.borrow_mut().take() {
            old.remove();
        }
        let me = Rc::downgrade(self);
        let id =
            glib::timeout_add_local_once(std::time::Duration::from_millis(TOAST_MS), move || {
                if let Some(t) = me.upgrade() {
                    t.timer.borrow_mut().take();
                    t.hide();
                }
            });
        *self.timer.borrow_mut() = Some(id);
    }

    pub fn hide(&self) {
        self.revealer.set_reveal_child(false);
        *self.action.borrow_mut() = None;
        if let Some(old) = self.timer.borrow_mut().take() {
            old.remove();
        }
    }

    /// The line shown now (tests).
    pub fn text(&self) -> String {
        self.text.label().to_string()
    }
}
