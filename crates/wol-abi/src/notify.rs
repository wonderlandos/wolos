//! Notifications: a service call. wold checks the app's permission and hands the notification to
//! the person's session; libwol only validates and asks.
use crate::client;
use crate::context::wol_context;
use crate::error::{Error, guard, input, wol_error, wol_status};
use crate::string::borrow;
use shared::portal::{Call, MAX_BODY, MAX_TITLE};
use std::ffi::c_char;

/// Ask wold to show a notification.
pub fn post(ctx: &wol_context, title: &str, body: &str) -> crate::error::Result<()> {
    let a = client::call(
        ctx.wold(),
        &Call::NotificationPost {
            title: title.into(),
            body: body.into(),
        },
    );
    match (a.ok, a.code) {
        (true, _) => Ok(()),
        (false, code) => Err(Error::new(
            code.map_or(crate::error::Status::Internal, Into::into),
            a.message.unwrap_or_default(),
        )),
    }
}

/// # Safety
/// `context` is live; `title`/`body` point at their lengths in bytes (or are NULL with 0);
/// `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_notification_post(
    context: *mut wol_context,
    title: *const c_char,
    title_len: usize,
    body: *const c_char,
    body_len: usize,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let ctx = unsafe { borrow(context) }?;
        let title = input(title, title_len, MAX_TITLE, "The title")?;
        let body = input(body, body_len, MAX_BODY, "The body")?;
        post(ctx, title, body)
    })
}
