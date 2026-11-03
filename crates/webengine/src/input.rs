// SPDX-License-Identifier: BSD-2-Clause
//! Input from the wolkit native backend to the page. The host reads its window's event records
//! (`wolkit_engine_read_events`, record v1, `crates/wolkit/src/event/mod.rs`) and hands each to
//! [`bridge`] with the id of the `surface` node the view paints into; what comes back is fed to
//! [`crate::WebView::apply`]. Pure: no engine needed, so the mapping is tested here.
//!
//! What record v1 carries for a surface node today: a click (button, mods, x, y), hover enter and
//! leave with a position, keys (keysym, pressed, mods, repeat), focus, the node's size in device
//! pixels and the theme. Not carried yet (wolkit gap, docs/modules/webengine.md "Gaps"): pointer
//! motion inside the node, separate button down and up, wheel deltas, IME preedit and commit for a
//! surface, and the clipboard. [`Input`] already has those kinds, so the bridge grows without an
//! ABI change when wolkit reports them.
use wolkit::event::{HEADER, kind};

/// One input event for the page, in device pixels of the view.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    MouseMove {
        x: f32,
        y: f32,
    },
    MouseButton {
        button: u32,
        pressed: bool,
        x: f32,
        y: f32,
    },
    Wheel {
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
    },
    /// `key` is a key value (`a`, `Enter`, `ArrowLeft`), `code` a code value (`KeyA`).
    Key {
        key: String,
        code: Option<String>,
        pressed: bool,
        mods: u32,
        repeat: bool,
    },
    ImePreedit(String),
    ImeCommit(String),
    ImeDismiss,
    MouseLeave,
}

/// What a wolkit record means for the view.
#[derive(Debug, Clone, PartialEq)]
pub enum Bridged {
    Input(Input),
    Focus(bool),
    Resize { width: u32, height: u32, scale: f32 },
    Theme { dark: bool },
}

/// Modifier bits are wolkit's (`WOLKIT_MOD_*`) and the engine library's (`WWE_MOD_*`) alike.
pub const MOD_SHIFT: u32 = wolkit::window::MOD_SHIFT;
pub const MOD_CTRL: u32 = wolkit::window::MOD_CTRL;
pub const MOD_ALT: u32 = wolkit::window::MOD_ALT;
pub const MOD_META: u32 = wolkit::window::MOD_SUPER;

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn f32_at(b: &[u8], at: usize) -> Option<f32> {
    u32_at(b, at).map(f32::from_bits)
}

/// The record's meaning for the view painting into `surface_node`, in order. A record for another
/// node, or one this runtime does not use, gives nothing; a short record gives nothing.
pub fn bridge(record: &[u8], surface_node: u32) -> Vec<Bridged> {
    let (Some(len), Some(head)) = (u32_at(record, 0), record.get(4..6)) else {
        return Vec::new();
    };
    let k = u16::from_le_bytes([head[0], head[1]]);
    let node = u32_at(record, 12).unwrap_or(0);
    let p = HEADER;
    if usize::try_from(len).map_or(true, |l| l > record.len()) {
        return Vec::new();
    }
    // Window events reach every node; the rest only the view's own.
    if k != kind::THEME_CHANGED && node != surface_node {
        return Vec::new();
    }
    let out = (|| {
        Some(match k {
            kind::CLICK => {
                // `u32 button`, `u32 mods` (the engine reads modifiers from key events), `f32 x, y`.
                let button = u32_at(record, p)?;
                let (x, y) = (f32_at(record, p + 8)?, f32_at(record, p + 12)?);
                vec![
                    Bridged::Input(Input::MouseMove { x, y }),
                    Bridged::Input(Input::MouseButton {
                        button,
                        pressed: true,
                        x,
                        y,
                    }),
                    Bridged::Input(Input::MouseButton {
                        button,
                        pressed: false,
                        x,
                        y,
                    }),
                ]
            }
            kind::POINTER_ENTER => {
                let (x, y) = (f32_at(record, p)?, f32_at(record, p + 4)?);
                vec![Bridged::Input(Input::MouseMove { x, y })]
            }
            kind::POINTER_LEAVE => vec![Bridged::Input(Input::MouseLeave)],
            kind::KEY => {
                let sym = u32_at(record, p)?;
                let (key, code) = key_of(sym)?;
                vec![Bridged::Input(Input::Key {
                    key,
                    code,
                    pressed: u32_at(record, p + 4)? != 0,
                    mods: u32_at(record, p + 8)?,
                    repeat: u32_at(record, p + 12)? != 0,
                })]
            }
            kind::FOCUS => vec![Bridged::Focus(u32_at(record, p)? != 0)],
            kind::SURFACE_RESIZED => vec![Bridged::Resize {
                width: u32_at(record, p)?,
                height: u32_at(record, p + 4)?,
                scale: f32_at(record, p + 8)?,
            }],
            kind::THEME_CHANGED => vec![Bridged::Theme {
                dark: u32_at(record, p)? != 0,
            }],
            _ => return None,
        })
    })();
    out.unwrap_or_default()
}

/// Key value and code value of an XKB keysym. Codes assume the US layout for printable keys
/// (record v1 has no scancode); a keysym without a meaning for the web gives `None`.
pub fn key_of(sym: u32) -> Option<(String, Option<String>)> {
    let named = |k: &str, c: &str| Some((k.to_owned(), Some(c.to_owned())));
    match sym {
        0xff08 => named("Backspace", "Backspace"),
        0xff09 => named("Tab", "Tab"),
        0xff0d => named("Enter", "Enter"),
        0xff8d => named("Enter", "NumpadEnter"),
        0xff1b => named("Escape", "Escape"),
        0xffff => named("Delete", "Delete"),
        0xff50 => named("Home", "Home"),
        0xff51 => named("ArrowLeft", "ArrowLeft"),
        0xff52 => named("ArrowUp", "ArrowUp"),
        0xff53 => named("ArrowRight", "ArrowRight"),
        0xff54 => named("ArrowDown", "ArrowDown"),
        0xff55 => named("PageUp", "PageUp"),
        0xff56 => named("PageDown", "PageDown"),
        0xff57 => named("End", "End"),
        0xff63 => named("Insert", "Insert"),
        0xff67 => named("ContextMenu", "ContextMenu"),
        0xffbe..=0xffc9 => {
            let n = sym - 0xffbe + 1;
            named(&format!("F{n}"), &format!("F{n}"))
        }
        0xffe1 => named("Shift", "ShiftLeft"),
        0xffe2 => named("Shift", "ShiftRight"),
        0xffe3 => named("Control", "ControlLeft"),
        0xffe4 => named("Control", "ControlRight"),
        0xffe5 => named("CapsLock", "CapsLock"),
        0xffe9 => named("Alt", "AltLeft"),
        0xffea => named("Alt", "AltRight"),
        0xffeb => named("Meta", "MetaLeft"),
        0xffec => named("Meta", "MetaRight"),
        0x20 => Some((" ".into(), Some("Space".into()))),
        0x21..=0x7e | 0xa0..=0xff => {
            let c = char::from_u32(sym)?;
            Some((c.to_string(), us_code(c)))
        }
        // Unicode keysyms (`0x01000000 + code point`).
        0x0100_0100..=0x0110_ffff => Some((char::from_u32(sym - 0x0100_0000)?.to_string(), None)),
        _ => None,
    }
}

fn us_code(c: char) -> Option<String> {
    let c = c.to_ascii_lowercase();
    Some(match c {
        'a'..='z' => format!("Key{}", c.to_ascii_uppercase()),
        '0'..='9' => format!("Digit{c}"),
        '-' | '_' => "Minus".into(),
        '=' | '+' => "Equal".into(),
        '[' | '{' => "BracketLeft".into(),
        ']' | '}' => "BracketRight".into(),
        ';' | ':' => "Semicolon".into(),
        '\'' | '"' => "Quote".into(),
        ',' | '<' => "Comma".into(),
        '.' | '>' => "Period".into(),
        '/' | '?' => "Slash".into(),
        '\\' | '|' => "Backslash".into(),
        '`' | '~' => "Backquote".into(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(k: u16, node: u32, payload: &[u8]) -> Vec<u8> {
        let len = (HEADER + payload.len()).div_ceil(8) * 8;
        let mut r = Vec::with_capacity(len);
        r.extend_from_slice(&(len as u32).to_le_bytes());
        r.extend_from_slice(&k.to_le_bytes());
        r.extend_from_slice(&0u16.to_le_bytes());
        r.extend_from_slice(&1u32.to_le_bytes());
        r.extend_from_slice(&node.to_le_bytes());
        r.extend_from_slice(&0u32.to_le_bytes());
        r.extend_from_slice(payload);
        r.resize(len, 0);
        r
    }

    fn words(w: &[u32]) -> Vec<u8> {
        w.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    #[test]
    fn a_click_is_a_move_a_press_and_a_release_at_its_point() {
        let r = record(
            kind::CLICK,
            7,
            &words(&[0, 0, 12.5f32.to_bits(), 3.0f32.to_bits()]),
        );
        let (x, y) = (12.5, 3.0);
        assert_eq!(
            bridge(&r, 7),
            [
                Bridged::Input(Input::MouseMove { x, y }),
                Bridged::Input(Input::MouseButton {
                    button: 0,
                    pressed: true,
                    x,
                    y
                }),
                Bridged::Input(Input::MouseButton {
                    button: 0,
                    pressed: false,
                    x,
                    y
                }),
            ]
        );
        assert!(bridge(&r, 8).is_empty(), "another node's click");
    }

    #[test]
    fn keys_carry_their_value_code_and_modifiers() {
        let r = record(kind::KEY, 7, &words(&[0x61, 1, MOD_CTRL, 0]));
        assert_eq!(
            bridge(&r, 7),
            [Bridged::Input(Input::Key {
                key: "a".into(),
                code: Some("KeyA".into()),
                pressed: true,
                mods: MOD_CTRL,
                repeat: false,
            })]
        );
        assert_eq!(key_of(0xff0d).unwrap().0, "Enter");
        assert_eq!(key_of(0xffc9).unwrap(), ("F12".into(), Some("F12".into())));
        assert_eq!(key_of(0x41).unwrap(), ("A".into(), Some("KeyA".into())));
        assert_eq!(key_of(0x0100_20ac).unwrap(), ("\u{20ac}".into(), None));
        assert_eq!(
            key_of(0xfe03),
            None,
            "ISO_Level3_Shift has no web meaning here"
        );
    }

    #[test]
    fn focus_size_theme_and_hover_reach_the_view() {
        assert_eq!(
            bridge(&record(kind::FOCUS, 7, &words(&[1])), 7),
            [Bridged::Focus(true)]
        );
        let r = record(
            kind::SURFACE_RESIZED,
            7,
            &words(&[800, 600, 2.0f32.to_bits()]),
        );
        assert_eq!(
            bridge(&r, 7),
            [Bridged::Resize {
                width: 800,
                height: 600,
                scale: 2.0
            }]
        );
        // A window event: whatever node the record names.
        let r = record(kind::THEME_CHANGED, 0, &words(&[1, 0x3b6df0, 0]));
        assert_eq!(bridge(&r, 7), [Bridged::Theme { dark: true }]);
        let r = record(kind::POINTER_LEAVE, 7, &words(&[0, 0]));
        assert_eq!(bridge(&r, 7), [Bridged::Input(Input::MouseLeave)]);
    }

    #[test]
    fn short_or_unknown_records_give_nothing() {
        assert!(bridge(&[], 7).is_empty());
        let mut r = record(kind::KEY, 7, &words(&[0x61, 1, 0, 0]));
        r.truncate(HEADER + 4);
        assert!(bridge(&r, 7).is_empty());
        assert!(bridge(&record(kind::SLIDER, 7, &words(&[0])), 7).is_empty());
    }
}
