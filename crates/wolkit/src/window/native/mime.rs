// SPDX-License-Identifier: BSD-2-Clause
//! Which type to take from a clipboard or drag offer, and the bounds on what is read.

/// Types a text paste takes, best first.
pub const TEXT: &[&str] = &[
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
    "TEXT",
];

/// Types the engine offers when it copies text.
pub const OFFER: &[&str] = &["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"];

/// Types a drop prefers: files first, then text, then whatever came first.
pub const DROP: &[&str] = &["text/uri-list", "text/plain;charset=utf-8", "text/plain"];

/// Most mime types kept per offer, and the longest one.
pub const MAX_TYPES: usize = 64;
pub const MAX_TYPE_LEN: usize = 255;

/// Bytes a paste reads at most (a text field holds 64 KiB).
pub const MAX_PASTE: usize = crate::scene::patch::MAX_STRING;

/// The best of `offered` for a text paste.
pub fn text_type(offered: &[String]) -> Option<&str> {
    TEXT.iter()
        .find(|t| offered.iter().any(|o| o == *t))
        .copied()
}

/// The type a drop delivers.
pub fn drop_type(offered: &[String]) -> Option<&str> {
    DROP.iter()
        .find(|t| offered.iter().any(|o| o == *t))
        .copied()
        .or_else(|| offered.first().map(String::as_str))
}

/// Keep a newly offered type if it is sane and there is room.
pub fn keep(list: &mut Vec<String>, t: &str) {
    if list.len() < MAX_TYPES
        && !t.is_empty()
        && t.len() <= MAX_TYPE_LEN
        && !list.iter().any(|x| x == t)
    {
        list.push(t.to_owned());
    }
}

/// Text from a paste: invalid UTF-8 replaced, cut at the field's limit on a boundary.
pub fn paste_text(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    crate::text::edit::truncate_bytes(&s, MAX_PASTE).to_owned()
}
