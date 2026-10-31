// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
//! Search (ADR 0020, 1.9): an app can search what Spotlight searches, and can answer Spotlight
//! with rows of its own. A provider lasts as long as its connection to wold; its callbacks run
//! inside `wol_context_dispatch`.
use crate::client;
use crate::context::{lock, wol_context};
use crate::error::{Error, UserData, guard, input, need, wol_error, wol_status};
use crate::info::{copy_info, wol_info};
use crate::kit::arc_of;
use crate::string::{into_raw, release, retain, wol_string};
use shared::portal::{Call, MAX_RESULT_KEY, MAX_RESULTS, MAX_TITLE, SearchResult};
use std::ffi::{c_char, c_void};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

/// Called for a query: add rows to the reply (borrowed for the call), then return.
pub type wol_search_fn = Option<
    unsafe extern "C" fn(
        *mut wol_search_provider,
        *mut wol_search_reply,
        *const wol_string,
        UserData,
    ),
>;
/// The person opened a row: its key, borrowed for the call.
pub type wol_search_open_fn =
    Option<unsafe extern "C" fn(*mut wol_search_provider, *const wol_string, UserData)>;

pub struct wol_search_provider {
    context: Arc<wol_context>,
    on_query: wol_search_fn,
    on_open: wol_search_open_fn,
    user_data: usize,
    _held: UnixStream,
}

pub struct wol_search_reply {
    results: Mutex<Vec<SearchResult>>,
}

/// wold asks this provider.
pub(crate) fn asked(p: &Arc<wol_search_provider>, id: u32, text: &str) {
    let reply = wol_search_reply {
        results: Mutex::default(),
    };
    if let Some(f) = p.on_query {
        let t = wol_string::new(text);
        // SAFETY: the app's callback on its thread (inside dispatch); the handles live for the
        // call.
        unsafe {
            f(
                Arc::as_ptr(p).cast_mut(),
                (&raw const reply).cast_mut(),
                Arc::as_ptr(&t),
                p.user_data as UserData,
            )
        };
    }
    let results = std::mem::take(&mut *lock(&reply.results));
    // A lost reply only leaves this app's rows out of one search.
    let _ = client::run(p.context.wold(), &Call::SearchReply { id, results });
}

/// wold says the person opened a row.
pub(crate) fn opened(p: &Arc<wol_search_provider>, key: &str) {
    if let Some(f) = p.on_open {
        let k = wol_string::new(key);
        // SAFETY: as in [`asked`].
        unsafe {
            f(
                Arc::as_ptr(p).cast_mut(),
                Arc::as_ptr(&k),
                p.user_data as UserData,
            )
        };
    }
}

/// # Safety
/// `context` is live; `text` points at `len` bytes; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_search_query(
    context: *mut wol_context,
    text: *const c_char,
    len: usize,
    out: *mut *mut wol_info,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let text = input(text, len, MAX_TITLE, "The text")?.to_owned();
        unsafe { copy_info(context, &Call::SearchQuery { text }, out) }
    })
}

/// # Safety
/// `context` is live; `out`, `error` writable or NULL. `user_data` is passed back untouched.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_search_provide(
    context: *mut wol_context,
    on_query: wol_search_fn,
    on_open: wol_search_open_fn,
    user_data: *mut c_void,
    out: *mut *mut wol_search_provider,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let context = arc_of(context)?;
        if on_query.is_none() {
            return Err(Error::invalid("A provider needs a query callback."));
        }
        if context.search_provider().is_some() {
            return Err(Error::invalid("This context already provides search."));
        }
        // Listening first, so no query is lost.
        context.ensure_bus()?;
        let held = client::hold(context.wold(), &Call::SearchProvide {})?;
        let p = Arc::new(wol_search_provider {
            context: context.clone(),
            on_query,
            on_open,
            user_data: user_data as usize,
            _held: held,
        });
        context.set_search_provider(&p);
        unsafe { *out = into_raw(p) };
        Ok(())
    })
}

/// # Safety
/// `reply` is the one a query callback got, during that call; strings point at their lengths;
/// `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_search_reply_add(
    reply: *mut wol_search_reply,
    key: *const c_char,
    key_len: usize,
    title: *const c_char,
    title_len: usize,
    detail: *const c_char,
    detail_len: usize,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let r =
            unsafe { reply.as_ref() }.ok_or_else(|| Error::invalid("reply must not be NULL."))?;
        let row = SearchResult {
            key: input(key, key_len, MAX_RESULT_KEY, "The key")?.to_owned(),
            title: input(title, title_len, MAX_TITLE, "The title")?.to_owned(),
            detail: input(detail, detail_len, MAX_TITLE, "The detail")?.to_owned(),
        };
        row.check().map_err(Error::invalid)?;
        let mut all = lock(&r.results);
        if all.len() >= MAX_RESULTS {
            return Err(Error::invalid(format!(
                "A reply has at most {MAX_RESULTS} rows."
            )));
        }
        all.push(row);
        Ok(())
    })
}

/// # Safety
/// `provider` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_search_provider_retain(
    provider: *mut wol_search_provider,
) -> *mut wol_search_provider {
    unsafe { retain(provider) }
}

/// # Safety
/// `provider` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_search_provider_release(provider: *mut wol_search_provider) {
    unsafe { release(provider) }
}
