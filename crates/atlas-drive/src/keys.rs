// SPDX-License-Identifier: GPL-2.0-or-later
//! Key names for `press`: `Enter`, `Control+a`, `Shift+Tab`, `a`.

pub const ALT: u32 = 1;
pub const CTRL: u32 = 2;
pub const META: u32 = 4;
pub const SHIFT: u32 = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct Key {
    pub key: String,
    pub code: String,
    pub vk: u32,
    pub text: String,
}

fn named(name: &str) -> Option<Key> {
    let (key, code, vk, text): (&str, &str, u32, &str) = match name.to_lowercase().as_str() {
        "enter" | "return" => ("Enter", "Enter", 13, "\r"),
        "tab" => ("Tab", "Tab", 9, ""),
        "escape" | "esc" => ("Escape", "Escape", 27, ""),
        "backspace" => ("Backspace", "Backspace", 8, ""),
        "delete" | "del" => ("Delete", "Delete", 46, ""),
        "space" | " " => (" ", "Space", 32, " "),
        "arrowup" | "up" => ("ArrowUp", "ArrowUp", 38, ""),
        "arrowdown" | "down" => ("ArrowDown", "ArrowDown", 40, ""),
        "arrowleft" | "left" => ("ArrowLeft", "ArrowLeft", 37, ""),
        "arrowright" | "right" => ("ArrowRight", "ArrowRight", 39, ""),
        "home" => ("Home", "Home", 36, ""),
        "end" => ("End", "End", 35, ""),
        "pageup" => ("PageUp", "PageUp", 33, ""),
        "pagedown" => ("PageDown", "PageDown", 34, ""),
        "insert" => ("Insert", "Insert", 45, ""),
        "control" | "ctrl" => ("Control", "ControlLeft", 17, ""),
        "alt" => ("Alt", "AltLeft", 18, ""),
        "shift" => ("Shift", "ShiftLeft", 16, ""),
        "meta" | "super" | "cmd" => ("Meta", "MetaLeft", 91, ""),
        _ => return None,
    };
    Some(Key {
        key: key.into(),
        code: code.into(),
        vk,
        text: text.into(),
    })
}

fn single(name: &str) -> Option<Key> {
    if let Some(k) = named(name) {
        return Some(k);
    }
    if let Some(n) = name
        .strip_prefix(['F', 'f'])
        .and_then(|n| n.parse::<u32>().ok())
        && (1..=12).contains(&n)
    {
        return Some(Key {
            key: format!("F{n}"),
            code: format!("F{n}"),
            vk: 111 + n,
            text: String::new(),
        });
    }
    let mut it = name.chars();
    let c = it.next()?;
    if it.next().is_some() {
        return None;
    }
    let (code, vk) = if c.is_ascii_alphabetic() {
        (
            format!("Key{}", c.to_ascii_uppercase()),
            c.to_ascii_uppercase() as u32,
        )
    } else if c.is_ascii_digit() {
        (format!("Digit{c}"), c as u32)
    } else {
        (
            match c {
                '-' => "Minus",
                '=' => "Equal",
                ',' => "Comma",
                '.' => "Period",
                '/' => "Slash",
                ';' => "Semicolon",
                '\'' => "Quote",
                '[' => "BracketLeft",
                ']' => "BracketRight",
                '\\' => "Backslash",
                '`' => "Backquote",
                _ => "",
            }
            .to_string(),
            c as u32,
        )
    };
    Some(Key {
        key: c.to_string(),
        code,
        vk,
        text: c.to_string(),
    })
}

/// `Control+Shift+a` -> (modifier bits, modifier keys in order, the key).
pub fn parse(spec: &str) -> Option<(u32, Vec<Key>, Key)> {
    let parts: Vec<&str> = if spec == "+" {
        vec!["+"]
    } else {
        spec.split('+').collect()
    };
    let (last, mods) = parts.split_last()?;
    let mut bits = 0;
    let mut keys = vec![];
    for m in mods {
        let k = named(m)?;
        bits |= match k.key.as_str() {
            "Alt" => ALT,
            "Control" => CTRL,
            "Meta" => META,
            "Shift" => SHIFT,
            _ => return None,
        };
        keys.push(k);
    }
    let key = single(last)?;
    Some((bits, keys, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_chords() {
        let (b, m, k) = parse("Enter").unwrap();
        assert_eq!((b, m.len(), k.vk), (0, 0, 13));
        let (b, m, k) = parse("Control+Shift+a").unwrap();
        assert_eq!(b, CTRL | SHIFT);
        assert_eq!(m.len(), 2);
        assert_eq!((k.key.as_str(), k.code.as_str(), k.vk), ("a", "KeyA", 65));
        assert_eq!(parse("F5").unwrap().2.vk, 116);
        assert!(parse("Bogus+a").is_none());
        assert!(parse("nonsense").is_none());
        assert_eq!(parse("+").unwrap().2.key, "+");
    }
}
