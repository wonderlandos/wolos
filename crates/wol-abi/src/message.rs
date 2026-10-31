// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
//! Messages between apps (ADR 0020, 1.11), routed by wold. An app is addressed by its executable
//! and an endpoint name; wold checks the grant for that one peer and tells the receiver who sent
//! each message. Callbacks run inside `wol_context_dispatch`.
use crate::client;
use crate::context::wol_context;
use crate::error::{Error, Result, UserData, guard, input, need, wol_error, wol_status};
use crate::kit::arc_of;
use crate::string::{into_raw, release, retain, wol_string};
use shared::portal::{Call, MAX_ENDPOINT, MAX_MESSAGE, MAX_PEER};
use std::ffi::{c_char, c_void};
use std::os::unix::net::UnixStream;
use std::sync::Arc;

/// A message came; it is borrowed for the call and kept with `wol_message_retain`.
pub type wol_message_fn =
    Option<unsafe extern "C" fn(*mut wol_endpoint, *mut wol_message, UserData)>;

pub struct wol_endpoint {
    context: Arc<wol_context>,
    on_message: wol_message_fn,
    user_data: usize,
    _held: UnixStream,
}

pub struct wol_message {
    context: Arc<wol_context>,
    id: u32,
    from: Arc<wol_string>,
    body: Arc<wol_string>,
}

/// wold delivered a message for endpoint `e`.
pub(crate) fn delivered(e: &Arc<wol_endpoint>, id: u32, from: &str, body: &str) {
    let Some(f) = e.on_message else { return };
    let m = Arc::new(wol_message {
        context: e.context.clone(),
        id,
        from: wol_string::new(from),
        body: wol_string::new(body),
    });
    // SAFETY: the app's callback on its thread (inside dispatch); both handles outlive the call.
    unsafe {
        f(
            Arc::as_ptr(e).cast_mut(),
            Arc::as_ptr(&m).cast_mut(),
            e.user_data as UserData,
        )
    };
}

fn message<'a>(m: *const wol_message) -> Result<&'a wol_message> {
    // SAFETY: the caller holds a reference (header contract).
    unsafe { m.as_ref() }.ok_or_else(|| Error::invalid("message must not be NULL."))
}

/// # Safety
/// `context` is live; `name` points at `len` bytes; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_endpoint_create(
    context: *mut wol_context,
    name: *const c_char,
    len: usize,
    on_message: wol_message_fn,
    user_data: *mut c_void,
    out: *mut *mut wol_endpoint,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let context = arc_of(context)?;
        let name = input(name, len, MAX_ENDPOINT, "The name")?.to_owned();
        if on_message.is_none() {
            return Err(Error::invalid("An endpoint needs a message callback."));
        }
        // Listening first, so no message is lost.
        context.ensure_bus()?;
        let held = client::hold(context.wold(), &Call::IpcListen { name: name.clone() })?;
        let e = Arc::new(wol_endpoint {
            context: context.clone(),
            on_message,
            user_data: user_data as usize,
            _held: held,
        });
        context.register_endpoint(name, &e);
        unsafe { *out = into_raw(e) };
        Ok(())
    })
}

/// # Safety
/// `endpoint` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_endpoint_retain(endpoint: *mut wol_endpoint) -> *mut wol_endpoint {
    unsafe { retain(endpoint) }
}

/// # Safety
/// `endpoint` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_endpoint_release(endpoint: *mut wol_endpoint) {
    unsafe { release(endpoint) }
}

/// # Safety
/// `context` is live; each pointer has its length; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_message_send(
    context: *mut wol_context,
    peer: *const c_char,
    peer_len: usize,
    name: *const c_char,
    name_len: usize,
    body: *const c_char,
    body_len: usize,
    out: *mut *mut wol_string,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let context = arc_of(context)?;
        let c = Call::IpcSend {
            peer: input(peer, peer_len, MAX_PEER, "The peer")?.to_owned(),
            name: input(name, name_len, MAX_ENDPOINT, "The name")?.to_owned(),
            body: input(body, body_len, MAX_MESSAGE, "The message")?.to_owned(),
        };
        let v = client::data(context.wold(), &c)?;
        let reply = v.get("body").and_then(|b| b.as_str()).unwrap_or_default();
        unsafe { *out = into_raw(wol_string::new(reply)) };
        Ok(())
    })
}

/// # Safety
/// `message` is NULL or live; the answer lives as long as the message.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_message_get_from(message: *const wol_message) -> *const wol_string {
    unsafe { message.as_ref() }.map_or(std::ptr::null(), |m| Arc::as_ptr(&m.from))
}

/// # Safety
/// `message` is NULL or live; the answer lives as long as the message.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_message_get_body(message: *const wol_message) -> *const wol_string {
    unsafe { message.as_ref() }.map_or(std::ptr::null(), |m| Arc::as_ptr(&m.body))
}

/// # Safety
/// `message` is live; `body` points at `len` bytes; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_message_reply(
    message: *const wol_message,
    body: *const c_char,
    len: usize,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let m = self::message(message)?;
        if m.id == 0 {
            return Err(Error::invalid("A published message takes no reply."));
        }
        let c = Call::IpcReply {
            id: m.id,
            body: input(body, len, MAX_MESSAGE, "The reply")?.to_owned(),
        };
        client::run(m.context.wold(), &c)
    })
}

/// # Safety
/// `message` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_message_retain(message: *mut wol_message) -> *mut wol_message {
    unsafe { retain(message) }
}

/// # Safety
/// `message` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_message_release(message: *mut wol_message) {
    unsafe { release(message) }
}

/// # Safety
/// `context` is live; each pointer has its length; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_publish(
    context: *mut wol_context,
    topic: *const c_char,
    topic_len: usize,
    body: *const c_char,
    body_len: usize,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let context = arc_of(context)?;
        let c = Call::IpcPublish {
            topic: input(topic, topic_len, MAX_ENDPOINT, "The topic")?.to_owned(),
            body: input(body, body_len, MAX_MESSAGE, "The message")?.to_owned(),
        };
        client::run(context.wold(), &c)
    })
}
