// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
//! `wol_info`: what a service call answers, as one read-only tree (ADR 0020, 1.4). A service can
//! say more over time (a new key) without a new symbol; the keys are listed in `wol.h`.
//! Every getter accepts NULL and answers NULL or 0, so lookups chain.
use crate::context::wol_context;
use crate::error::{Result, need};
use crate::string::{borrow, into_raw, release, retain, wol_string};
use serde_json::Value;
use shared::portal::Call;
use std::collections::BTreeMap;
use std::ffi::{c_char, c_int};
use std::ptr::null;
use std::sync::Arc;

pub struct wol_info {
    json: Arc<wol_string>,
    text: Option<Arc<wol_string>>,
    number: Option<f64>,
    yes: bool,
    fields: BTreeMap<String, Arc<wol_info>>,
    items: Vec<Arc<wol_info>>,
}

impl wol_info {
    pub fn new(v: &Value) -> Arc<Self> {
        let map = |o: &serde_json::Map<String, Value>| {
            o.iter()
                .map(|(k, v)| (k.clone(), Self::new(v)))
                .collect::<BTreeMap<_, _>>()
        };
        Arc::new(Self {
            json: wol_string::new(&v.to_string()),
            text: v.as_str().map(wol_string::new),
            number: v.as_f64(),
            yes: v.as_bool() == Some(true),
            fields: v.as_object().map(map).unwrap_or_default(),
            items: v
                .as_array()
                .map(|a| a.iter().map(Self::new).collect())
                .unwrap_or_default(),
        })
    }

    /// A handle the app owns.
    pub fn out(v: &Value) -> *mut Self {
        into_raw(Self::new(v))
    }
}

/// A call whose data the app reads as a `wol_info`.
///
/// # Safety
/// `context` is live; `out` writable or NULL.
pub(crate) unsafe fn copy_info(
    context: *mut wol_context,
    call: &Call,
    out: *mut *mut wol_info,
) -> Result<()> {
    need(out, "out")?;
    let ctx = unsafe { borrow(context) }?;
    let data = crate::client::data(ctx.wold(), call)?;
    unsafe { *out = wol_info::out(&data) };
    Ok(())
}

/// Field `key` of an object; NULL when there is none. Borrowed from `info`.
///
/// # Safety
/// `info` is NULL or live; `key` points at `key_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_info_get(
    info: *const wol_info,
    key: *const c_char,
    key_len: usize,
) -> *const wol_info {
    let Some(i) = (unsafe { info.as_ref() }) else {
        return null();
    };
    let Ok(key) = crate::error::input(key, key_len, 256, "The key") else {
        return null();
    };
    i.fields.get(key).map_or(null(), Arc::as_ptr)
}

/// Items in a list; 0 for anything else.
///
/// # Safety
/// `info` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_info_get_count(info: *const wol_info) -> usize {
    unsafe { info.as_ref() }.map_or(0, |i| i.items.len())
}

/// Item `index` of a list; NULL past the end. Borrowed from `info`.
///
/// # Safety
/// `info` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_info_get_item(info: *const wol_info, index: usize) -> *const wol_info {
    unsafe { info.as_ref() }
        .and_then(|i| i.items.get(index))
        .map_or(null(), Arc::as_ptr)
}

/// The text of a string; NULL for anything else. Borrowed from `info`.
///
/// # Safety
/// `info` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_info_get_text(info: *const wol_info) -> *const wol_string {
    unsafe { info.as_ref() }
        .and_then(|i| i.text.as_ref())
        .map_or(null(), Arc::as_ptr)
}

/// 1 and `*out` set for a number; 0 for anything else.
///
/// # Safety
/// `info` is NULL or live; `out` is NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_info_get_number(info: *const wol_info, out: *mut f64) -> c_int {
    match (
        unsafe { info.as_ref() }.and_then(|i| i.number),
        out.is_null(),
    ) {
        (Some(n), false) => {
            unsafe { *out = n };
            1
        }
        _ => 0,
    }
}

/// 1 for `true`; 0 for anything else.
///
/// # Safety
/// `info` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_info_get_bool(info: *const wol_info) -> c_int {
    unsafe { info.as_ref() }.is_some_and(|i| i.yes) as c_int
}

/// The whole value as JSON. Borrowed from `info`.
///
/// # Safety
/// `info` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_info_get_json(info: *const wol_info) -> *const wol_string {
    unsafe { info.as_ref() }.map_or(null(), |i| Arc::as_ptr(&i.json))
}

/// # Safety
/// `info` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_info_retain(info: *mut wol_info) -> *mut wol_info {
    unsafe { retain(info) }
}

/// # Safety
/// `info` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_info_release(info: *mut wol_info) {
    unsafe { release(info) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn s(p: *const wol_string) -> String {
        unsafe { (*p).as_str() }.to_owned()
    }

    #[test]
    fn a_tree_reads_and_every_miss_is_null() {
        let i = wol_info::out(&json!({"a": [1.5, "x"], "b": true, "c": false}));
        unsafe {
            let a = wol_info_get(i, c"a".as_ptr(), 1);
            assert_eq!(wol_info_get_count(a), 2);
            let mut n = 0.0;
            assert_eq!(wol_info_get_number(wol_info_get_item(a, 0), &mut n), 1);
            assert_eq!(n, 1.5);
            assert_eq!(s(wol_info_get_text(wol_info_get_item(a, 1))), "x");
            assert!(wol_info_get_item(a, 2).is_null());
            assert_eq!(wol_info_get_bool(wol_info_get(i, c"b".as_ptr(), 1)), 1);
            assert_eq!(wol_info_get_bool(wol_info_get(i, c"c".as_ptr(), 1)), 0);
            assert_eq!(s(wol_info_get_json(a)), r#"[1.5,"x"]"#);
            let none = wol_info_get(i, c"z".as_ptr(), 1);
            assert!(none.is_null());
            assert!(wol_info_get(none, c"z".as_ptr(), 1).is_null());
            assert!(wol_info_get_text(none).is_null());
            assert!(wol_info_get_json(none).is_null());
            assert_eq!(wol_info_get_count(none), 0);
            assert_eq!(wol_info_get_number(none, &mut n), 0);
            assert_eq!(wol_info_get_number(i, std::ptr::null_mut()), 0);
            assert!(wol_info_get(i, [0xffu8].as_ptr().cast(), 1).is_null());
            wol_info_release(i);
        }
    }
}
