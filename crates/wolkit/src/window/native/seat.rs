// SPDX-License-Identifier: BSD-2-Clause
//! The keyboard: the compositor's keymap (through the system keymap library, the one C library
//! the native backend links: it parses keymaps, not fonts or images), modifier state, compose
//! and dead keys from the locale's compose table, and which keys repeat. A key becomes a keysym,
//! the `MOD_*` bits and the text it types; Command reaches the app as Control because the
//! compositor maps it so (the shell's app menu does the same).
//!
//! The library's objects are not `Send`; [`Keyboard`] is, because the engine's lock gives one
//! thread at a time exclusive use of a window (see the `unsafe impl` below).

use crate::window::{MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_SUPER};
use std::os::fd::OwnedFd;
use xkbcommon::xkb;

/// Largest keymap accepted (real ones are 50 to 150 KiB).
pub const MAX_KEYMAP: u32 = 4 << 20;

/// One decoded key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub keysym: u32,
    pub mods: u32,
    pub text: String,
    pub repeats: bool,
}

struct Map {
    keymap: xkb::Keymap,
    state: xkb::State,
}

pub struct Keyboard {
    ctx: xkb::Context,
    map: Option<Map>,
    compose: Option<xkb::compose::State>,
}

// SAFETY: the keymap library's context, keymap, state and compose objects are reference counted
// C objects that are not safe to *share* between threads, but may be used from any one thread at
// a time. A `Keyboard` is owned by one native window, which the engine only touches with its lock
// held, so no two threads ever use these objects at once.
unsafe impl Send for Keyboard {}

impl Keyboard {
    /// A keyboard with no keymap yet; `locale` picks the compose table (dead keys, compose key).
    pub fn new(locale: &str) -> Keyboard {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let compose = xkb::compose::Table::new_from_locale(
            &ctx,
            std::ffi::OsStr::new(locale),
            xkb::compose::COMPILE_NO_FLAGS,
        )
        .ok()
        .map(|t| xkb::compose::State::new(&t, xkb::compose::STATE_NO_FLAGS));
        Keyboard {
            ctx,
            map: None,
            compose,
        }
    }

    /// Install the compositor's keymap. A keymap that cannot be read leaves keys undecoded (they
    /// are dropped) until the next one.
    pub fn set_keymap(&mut self, fd: OwnedFd, size: u32) {
        if size == 0 || size > MAX_KEYMAP {
            self.map = None;
            return;
        }
        let Ok(len) = usize::try_from(size) else {
            return;
        };
        // SAFETY: the compositor sent `fd` for exactly this: a keymap of `size` bytes it keeps
        // unchanged (the protocol requires a read-only mapping to be valid); the library maps it
        // read-only, copies what it parses, and the size is bounded above.
        let parsed = unsafe {
            xkb::Keymap::new_from_fd(
                &self.ctx,
                fd,
                len,
                xkb::KEYMAP_FORMAT_TEXT_V1,
                xkb::KEYMAP_COMPILE_NO_FLAGS,
            )
        };
        self.map = match parsed {
            Ok(Some(keymap)) => {
                let state = xkb::State::new(&keymap);
                Some(Map { keymap, state })
            }
            _ => None,
        };
    }

    /// Install a keymap from its text (tests, and compositors that send none).
    pub fn set_keymap_names(&mut self, layout: &str, variant: &str) -> bool {
        let km = xkb::Keymap::new_from_names(
            &self.ctx,
            "",
            "",
            layout,
            variant,
            None,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        );
        self.map = km.map(|keymap| {
            let state = xkb::State::new(&keymap);
            Map { keymap, state }
        });
        self.map.is_some()
    }

    pub fn has_keymap(&self) -> bool {
        self.map.is_some()
    }

    pub fn modifiers(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        if let Some(m) = &mut self.map {
            m.state.update_mask(depressed, latched, locked, 0, 0, group);
        }
    }

    /// The `MOD_*` bits in force.
    pub fn mods(&self) -> u32 {
        let Some(m) = &self.map else {
            return 0;
        };
        let on = |name: &str| m.state.mod_name_is_active(name, xkb::STATE_MODS_EFFECTIVE);
        let mut v = 0;
        for (name, bit) in [
            (xkb::MOD_NAME_SHIFT, MOD_SHIFT),
            (xkb::MOD_NAME_CTRL, MOD_CTRL),
            (xkb::MOD_NAME_ALT, MOD_ALT),
            (xkb::MOD_NAME_LOGO, MOD_SUPER),
        ] {
            if on(name) {
                v |= bit;
            }
        }
        v
    }

    pub fn reset_compose(&mut self) {
        if let Some(c) = &mut self.compose {
            c.reset();
        }
    }

    /// Decode a key (`key` is the evdev code from `wl_keyboard.key`). `None` while a compose or
    /// dead-key sequence is in progress (nothing to deliver yet), or without a keymap.
    pub fn key(&mut self, key: u32, pressed: bool) -> Option<Decoded> {
        let m = self.map.as_ref()?;
        let code = xkb::Keycode::new(key.checked_add(8)?);
        let sym = m.state.key_get_one_sym(code);
        let mut text = if pressed {
            m.state.key_get_utf8(code)
        } else {
            String::new()
        };
        let repeats = m.keymap.key_repeats(code);
        let mods = self.mods();
        let mut keysym = sym.raw();
        if pressed && let Some(c) = &mut self.compose {
            c.feed(sym);
            match c.status() {
                xkb::compose::Status::Composing => return None,
                xkb::compose::Status::Composed => {
                    text = c.utf8().unwrap_or_default();
                    keysym = c.keysym().map_or(keysym, |k| k.raw());
                    c.reset();
                }
                xkb::compose::Status::Cancelled => {
                    c.reset();
                    return None;
                }
                xkb::compose::Status::Nothing => {}
            }
        }
        // Control characters are not text (Return, Tab, Backspace and Control+letter).
        if text.chars().any(char::is_control) {
            text.clear();
        }
        Some(Decoded {
            keysym,
            mods,
            text,
            repeats,
        })
    }
}
