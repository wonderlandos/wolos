//! Notifications: a service call. wold checks the app's permission and hands the notification to
//! the person's session; libwol only validates and asks. A pressed button comes back from wold as
//! a `notification-action` event for this process only, and runs the handler inside dispatch.
//! Replacing and withdrawing are service calls too: wold allows them only to the process that
//! posted the notification.
use crate::client;
use crate::context::{lock, wol_context};
use crate::error::{Error, Result, Status, UserData, guard, input, need, wol_error, wol_status};
use crate::kit::arc_of;
use crate::string::{borrow, into_raw, release, retain, wol_string};
use shared::portal::{
    Call, MAX_ACTION_KEY, MAX_ACTION_LABEL, MAX_ACTIONS, MAX_BODY, MAX_TITLE, NoteAction,
    action_key_ok,
};
use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// A button was pressed: the notification and the button's key (borrowed for the call).
pub type wol_action_fn =
    Option<unsafe extern "C" fn(*mut wol_notification, *const wol_string, UserData)>;

pub struct wol_notification {
    context: Arc<wol_context>,
    title: Mutex<String>,
    body: Mutex<String>,
    /// wold's id once sent; 0 before that and after a withdraw.
    id: AtomicU32,
    actions: Mutex<Vec<NoteAction>>,
    handler: Mutex<
        Option<(
            unsafe extern "C" fn(*mut wol_notification, *const wol_string, UserData),
            usize,
        )>,
    >,
}

fn answer_error(a: shared::portal::Answer) -> Error {
    Error::new(
        a.code.map_or(Status::Internal, Into::into),
        a.message.unwrap_or_default(),
    )
}

/// Ask wold to show a notification; its id on success.
pub fn post(ctx: &wol_context, title: &str, body: &str, actions: Vec<NoteAction>) -> Result<u32> {
    let a = client::call(
        ctx.wold(),
        &Call::NotificationPost {
            title: title.into(),
            body: body.into(),
            actions,
        },
    );
    if a.ok {
        Ok(a.id.unwrap_or(0))
    } else {
        Err(answer_error(a))
    }
}

/// wold says a button was pressed.
pub(crate) fn pressed(n: &Arc<wol_notification>, key: &str) {
    let known = lock(&n.actions).iter().any(|a| a.key == key);
    let handler = *lock(&n.handler);
    if let (true, Some((f, ud))) = (known, handler) {
        let k = wol_string::new(key);
        // SAFETY: the app's callback and user data on its thread (inside dispatch); both handles
        // are borrowed and live for the call.
        unsafe { f(Arc::as_ptr(n).cast_mut(), Arc::as_ptr(&k), ud as UserData) };
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
        post(ctx, title, body, Vec::new()).map(|_| ())
    })
}

/// A notification with buttons; nothing is shown until `wol_notification_send`.
///
/// # Safety
/// `context` is live; `title`/`body` point at their lengths in bytes (or are NULL with 0);
/// `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_notification_create(
    context: *mut wol_context,
    title: *const c_char,
    title_len: usize,
    body: *const c_char,
    body_len: usize,
    out: *mut *mut wol_notification,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let context = arc_of(context)?;
        let title = input(title, title_len, MAX_TITLE, "The title")?.to_owned();
        let body = input(body, body_len, MAX_BODY, "The body")?.to_owned();
        let n = wol_notification {
            context,
            title: Mutex::new(title),
            body: Mutex::new(body),
            id: AtomicU32::new(0),
            actions: Mutex::default(),
            handler: Mutex::default(),
        };
        unsafe { *out = into_raw(Arc::new(n)) };
        Ok(())
    })
}

/// Add a button. `key` (`[a-z0-9_-]`, unique) comes back to the action handler; key `default`
/// is the notification itself being clicked.
///
/// # Safety
/// `notification` is live; `key`/`label` point at their lengths; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_notification_add_action(
    notification: *mut wol_notification,
    key: *const c_char,
    key_len: usize,
    label: *const c_char,
    label_len: usize,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let n = unsafe { borrow(notification) }?;
        let key = input(key, key_len, MAX_ACTION_KEY, "The key")?;
        let label = input(label, label_len, MAX_ACTION_LABEL, "The button label")?;
        if !action_key_ok(key) {
            return Err(Error::invalid(
                "A key is 1 to 32 lowercase letters, digits, '-' or '_'.",
            ));
        }
        if label.trim().is_empty() || label.chars().any(char::is_control) {
            return Err(Error::invalid("A button label is one line."));
        }
        let mut a = lock(&n.actions);
        if a.len() >= MAX_ACTIONS {
            return Err(Error::invalid(format!(
                "A notification has at most {MAX_ACTIONS} buttons."
            )));
        }
        if a.iter().any(|x| x.key == key) {
            return Err(Error::invalid("Each button needs its own key."));
        }
        a.push(NoteAction {
            key: key.into(),
            label: label.into(),
        });
        Ok(())
    })
}

/// # Safety
/// `notification` is live; `error` writable or NULL. `user_data` is passed back untouched.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_notification_set_action_handler(
    notification: *mut wol_notification,
    on_action: wol_action_fn,
    user_data: *mut c_void,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let n = unsafe { borrow(notification) }?;
        *lock(&n.handler) = on_action.map(|f| (f, user_data as usize));
        Ok(())
    })
}

/// Show it. With buttons, libwol first listens for the answer, so a press is never lost.
///
/// # Safety
/// `notification` is live; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_notification_send(
    notification: *mut wol_notification,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let n = arc_of(notification)?;
        let actions = lock(&n.actions).clone();
        if !actions.is_empty() {
            n.context.ensure_bus()?;
        }
        let (title, body) = (lock(&n.title).clone(), lock(&n.body).clone());
        let id = post(&n.context, &title, &body, actions)?;
        if id != 0 {
            n.context.register_note(id, &n);
        }
        n.id.store(id, Ordering::Release);
        Ok(())
    })
}

/// New text. A notification already sent changes on screen, buttons kept; one not sent yet just
/// has the new text when it is sent.
///
/// # Safety
/// `notification` is live; `title`/`body` point at their lengths in bytes (or are NULL with 0);
/// `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_notification_replace(
    notification: *mut wol_notification,
    title: *const c_char,
    title_len: usize,
    body: *const c_char,
    body_len: usize,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let n = unsafe { borrow(notification) }?;
        let title = input(title, title_len, MAX_TITLE, "The title")?;
        let body = input(body, body_len, MAX_BODY, "The body")?;
        let id = n.id.load(Ordering::Acquire);
        if id != 0 {
            let a = client::call(
                n.context.wold(),
                &Call::NotificationReplace {
                    id,
                    title: title.into(),
                    body: body.into(),
                    actions: lock(&n.actions).clone(),
                },
            );
            if !a.ok {
                return Err(answer_error(a));
            }
        }
        *lock(&n.title) = title.into();
        *lock(&n.body) = body.into();
        Ok(())
    })
}

/// Take it off the screen and out of Notification Center. Doing nothing if it was never sent.
///
/// # Safety
/// `notification` is live; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_notification_withdraw(
    notification: *mut wol_notification,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let n = unsafe { borrow(notification) }?;
        let id = n.id.swap(0, Ordering::AcqRel);
        if id != 0 {
            let a = client::call(n.context.wold(), &Call::NotificationWithdraw { id });
            if !a.ok {
                n.id.store(id, Ordering::Release);
                return Err(answer_error(a));
            }
        }
        Ok(())
    })
}

/// # Safety
/// `notification` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_notification_retain(
    notification: *mut wol_notification,
) -> *mut wol_notification {
    unsafe { retain(notification) }
}

/// # Safety
/// `notification` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_notification_release(notification: *mut wol_notification) {
    unsafe { release(notification) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::null_mut;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn buttons_are_checked_and_only_known_keys_run() {
        static PRESSED: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn on(_: *mut wol_notification, k: *const wol_string, ud: UserData) {
            assert_eq!(unsafe { (*k).as_str() }, "open");
            assert_eq!(ud as usize, 3);
            PRESSED.fetch_add(1, Ordering::Relaxed);
        }
        let c = into_raw(Arc::new(wol_context::new("org.example.t").unwrap()));
        let mut n = null_mut();
        unsafe {
            assert_eq!(
                wol_notification_create(
                    c,
                    c"Done".as_ptr(),
                    4,
                    std::ptr::null(),
                    0,
                    &mut n,
                    null_mut()
                ),
                0
            );
            let add = |k: &std::ffi::CStr, l: &std::ffi::CStr| {
                wol_notification_add_action(
                    n,
                    k.as_ptr(),
                    k.count_bytes(),
                    l.as_ptr(),
                    l.count_bytes(),
                    null_mut(),
                )
            };
            assert_eq!(add(c"open", c"Open"), 0);
            assert_eq!(add(c"open", c"Again"), 1, "unique keys");
            assert_eq!(add(c"Open", c"x"), 1, "lowercase keys");
            assert_eq!(add(c"later", c" "), 1, "a label");
            assert_eq!(add(c"later", c"Later"), 0);
            assert_eq!(add(c"third", c"Third"), 0);
            assert_eq!(add(c"fourth", c"Fourth"), 1, "at most three");
            assert_eq!(
                wol_notification_set_action_handler(n, Some(on), 3 as UserData, null_mut()),
                0
            );
            let a = arc_of(n).unwrap();
            pressed(&a, "open");
            pressed(&a, "forged");
            assert_eq!(PRESSED.load(Ordering::Relaxed), 1);
            drop(a);
            wol_notification_release(n);
            crate::context::wol_context_release(c);
        }
    }
}
