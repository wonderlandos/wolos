//! Windows and Kit parts as data (ADR 0020). libwol keeps a tree of parts and sends it to the Kit
//! host (the shell) as a `shared::view` message; the host draws it with the Kit and sends clicks
//! back. Apps link only this BSD library; the Kit itself stays in the host process.
use crate::context::{lock, wol_context};
use crate::error::{Error, Result, Status, UserData, guard, input, need, wol_error, wol_status};
use crate::string::{borrow, into_raw, release, retain};
use shared::view::{MAX_TEXT, MAX_TITLE, Node, Role, SPACING, ToHost};
use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub type wol_click_fn = Option<unsafe extern "C" fn(*mut wol_widget, UserData)>;

enum Kind {
    Stack {
        spacing: u8,
        children: Vec<Arc<wol_widget>>,
    },
    Label {
        text: String,
    },
    Button {
        label: String,
        on_click: unsafe extern "C" fn(*mut wol_widget, UserData),
        user_data: usize,
    },
}

pub struct wol_widget {
    id: u32,
    kind: Mutex<Kind>,
    /// In a stack or a window already.
    placed: AtomicBool,
}

pub struct wol_window {
    id: u32,
    title: String,
    context: Arc<wol_context>,
    content: Mutex<Option<Arc<wol_widget>>>,
}

impl wol_widget {
    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn node(&self) -> Node {
        match &*lock(&self.kind) {
            Kind::Stack { spacing, children } => Node::Stack {
                spacing: *spacing,
                children: children.iter().map(|c| c.node()).collect(),
            },
            Kind::Label { text } => Node::Label {
                text: text.clone(),
                role: Role::Body,
            },
            Kind::Button { label, .. } => Node::Button {
                id: self.id,
                label: label.clone(),
            },
        }
    }

    fn contains(&self, other: &wol_widget) -> bool {
        std::ptr::eq(self, other)
            || match &*lock(&self.kind) {
                Kind::Stack { children, .. } => children.iter().any(|c| c.contains(other)),
                _ => false,
            }
    }
}

/// A click from the host: run the button's callback (inside dispatch, on the app's thread).
pub(crate) fn clicked(w: &Arc<wol_widget>) {
    let cb = match &*lock(&w.kind) {
        Kind::Button {
            on_click,
            user_data,
            ..
        } => Some((*on_click, *user_data)),
        _ => None,
    };
    if let Some((f, ud)) = cb {
        // SAFETY: the app's callback, its user data, a borrowed live widget.
        unsafe { f(Arc::as_ptr(w).cast_mut(), ud as UserData) };
    }
}

/// An owned Arc from a borrowed handle (+1).
fn arc_of<T>(p: *mut T) -> Result<Arc<T>> {
    if p.is_null() {
        return Err(Error::invalid("A handle is NULL."));
    }
    // SAFETY: live handle from Arc::into_raw (contract); we take a new reference.
    unsafe {
        Arc::increment_strong_count(p.cast_const());
        Ok(Arc::from_raw(p.cast_const()))
    }
}

fn text(ptr: *const c_char, len: usize, max: usize, what: &str) -> Result<String> {
    let s = input(ptr, len, max, what)?;
    if s.chars().any(|c| c.is_control() && c != '\n') {
        return Err(Error::invalid(format!("{what} has control characters.")));
    }
    Ok(s.to_owned())
}

fn make(ctx: &wol_context, kind: Kind, out: *mut *mut wol_widget) {
    let w = Arc::new(wol_widget {
        id: ctx.next_id(),
        kind: Mutex::new(kind),
        placed: AtomicBool::new(false),
    });
    ctx.register(&w);
    // SAFETY: `out` checked by the caller.
    unsafe { *out = into_raw(w) };
}

/// # Safety
/// `context` is live; `title` points at `len` bytes; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_window_create(
    context: *mut wol_context,
    title: *const c_char,
    len: usize,
    out: *mut *mut wol_window,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let context = arc_of(context)?;
        let title = input(title, len, MAX_TITLE, "The window title")?;
        if title.chars().any(char::is_control) {
            return Err(Error::invalid("A window title is one line."));
        }
        let w = wol_window {
            id: context.next_id(),
            title: title.into(),
            context,
            content: Mutex::default(),
        };
        unsafe { *out = into_raw(Arc::new(w)) };
        Ok(())
    })
}

/// # Safety
/// `window` is live; `content` is NULL (clear) or a live widget; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_window_set_content(
    window: *mut wol_window,
    content: *mut wol_widget,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let win = unsafe { borrow(window) }?;
        let new = if content.is_null() {
            None
        } else {
            Some(arc_of(content)?)
        };
        if let Some(w) = &new
            && w.placed.swap(true, Ordering::AcqRel)
        {
            return Err(Error::invalid(
                "This part is already in a stack or a window.",
            ));
        }
        if let Some(old) = std::mem::replace(&mut *lock(&win.content), new) {
            old.placed.store(false, Ordering::Release);
        }
        Ok(())
    })
}

/// Show the window, or bring it up to date with its parts.
///
/// # Safety
/// `window` is live; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_window_show(
    window: *mut wol_window,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let win = unsafe { borrow(window) }?;
        let content = lock(&win.content)
            .as_ref()
            .map(|c| c.node())
            .unwrap_or(Node::Stack {
                spacing: 0,
                children: Vec::new(),
            });
        content.validate().map_err(Error::invalid)?;
        let m = ToHost::Show {
            window: win.id,
            title: win.title.clone(),
            content,
        };
        win.context.send_kit(&m.to_line())
    })
}

/// # Safety
/// `window` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_window_retain(window: *mut wol_window) -> *mut wol_window {
    unsafe { retain(window) }
}

/// # Safety
/// `window` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_window_release(window: *mut wol_window) {
    unsafe { release(window) }
}

/// # Safety
/// `context` is live; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_stack_create(
    context: *mut wol_context,
    spacing: u32,
    out: *mut *mut wol_widget,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = unsafe { borrow(context) }?;
        let spacing = u8::try_from(spacing)
            .ok()
            .filter(|s| SPACING.contains(s))
            .ok_or_else(|| Error::invalid(format!("Spacing is one of {SPACING:?}.")))?;
        make(
            ctx,
            Kind::Stack {
                spacing,
                children: Vec::new(),
            },
            out,
        );
        Ok(())
    })
}

/// # Safety
/// `context` is live; `text` points at `len` bytes; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_label_create(
    context: *mut wol_context,
    text_utf8: *const c_char,
    len: usize,
    out: *mut *mut wol_widget,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = unsafe { borrow(context) }?;
        let text = text(text_utf8, len, MAX_TEXT, "The label text")?;
        make(ctx, Kind::Label { text }, out);
        Ok(())
    })
}

/// # Safety
/// `context` is live; `label` points at `len` bytes; `on_click` is a function; `out`, `error`
/// writable or NULL. `user_data` is passed back untouched.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_button_create(
    context: *mut wol_context,
    label: *const c_char,
    len: usize,
    on_click: wol_click_fn,
    user_data: *mut c_void,
    out: *mut *mut wol_widget,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = unsafe { borrow(context) }?;
        let label = text(label, len, MAX_TEXT, "The button label")?;
        if label.trim().is_empty() {
            return Err(Error::invalid("A button needs a label."));
        }
        let on_click =
            on_click.ok_or_else(|| Error::invalid("A button needs a click callback."))?;
        make(
            ctx,
            Kind::Button {
                label,
                on_click,
                user_data: user_data as usize,
            },
            out,
        );
        Ok(())
    })
}

/// # Safety
/// `parent` and `child` are live; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_widget_add_child(
    parent: *mut wol_widget,
    child: *mut wol_widget,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let p = unsafe { borrow(parent) }?;
        let c = arc_of(child)?;
        if c.contains(p) {
            return Err(Error::invalid("A part cannot hold itself."));
        }
        let mut kind = lock(&p.kind);
        let Kind::Stack { children, .. } = &mut *kind else {
            return Err(Error::invalid("Only a stack holds other parts."));
        };
        if c.placed.swap(true, Ordering::AcqRel) {
            return Err(Error::invalid(
                "This part is already in a stack or a window.",
            ));
        }
        if children.len() >= shared::view::MAX_NODES {
            c.placed.store(false, Ordering::Release);
            return Err(Error::new(Status::InvalidArgument, "This stack is full."));
        }
        children.push(c);
        Ok(())
    })
}

/// # Safety
/// `widget` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_widget_retain(widget: *mut wol_widget) -> *mut wol_widget {
    unsafe { retain(widget) }
}

/// # Safety
/// `widget` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_widget_release(widget: *mut wol_widget) {
    unsafe { release(widget) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::null_mut;

    fn ctx() -> *mut wol_context {
        into_raw(Arc::new(wol_context::new("org.example.t").unwrap()))
    }

    unsafe extern "C" fn nop(_: *mut wol_widget, _: UserData) {}

    #[test]
    fn tree_rules() {
        let c = ctx();
        let (mut s, mut s2, mut l, mut b) = (null_mut(), null_mut(), null_mut(), null_mut());
        unsafe {
            assert_eq!(wol_stack_create(c, 8, &mut s, null_mut()), 0);
            assert_eq!(wol_stack_create(c, 8, &mut s2, null_mut()), 0);
            assert_eq!(
                wol_stack_create(c, 7, &mut l, null_mut()),
                1,
                "off the scale"
            );
            assert_eq!(
                wol_label_create(c, c"Hi".as_ptr(), 2, &mut l, null_mut()),
                0
            );
            assert_eq!(
                wol_button_create(
                    c,
                    c"OK".as_ptr(),
                    2,
                    Some(nop),
                    null_mut(),
                    &mut b,
                    null_mut()
                ),
                0
            );
            assert_eq!(
                wol_button_create(c, c"OK".as_ptr(), 2, None, null_mut(), &mut b, null_mut()),
                1
            );
            assert_eq!(
                wol_label_create(c, c"a\x07".as_ptr(), 2, &mut l, null_mut()),
                1
            );
            assert_eq!(wol_widget_add_child(s, l, null_mut()), 0);
            assert_eq!(wol_widget_add_child(s2, l, null_mut()), 1, "one parent");
            assert_eq!(wol_widget_add_child(s, s, null_mut()), 1, "no self");
            assert_eq!(wol_widget_add_child(s, s2, null_mut()), 0);
            assert_eq!(wol_widget_add_child(s2, s, null_mut()), 1, "no cycle");
            assert_eq!(wol_widget_add_child(b, l, null_mut()), 1, "not a stack");
            assert_eq!(wol_widget_add_child(s2, b, null_mut()), 0);
            let n = (*s).node();
            assert_eq!(
                serde_json::to_string(&n).unwrap(),
                format!(
                    r#"{{"type":"stack","spacing":8,"children":[{{"type":"label","text":"Hi","role":"body"}},{{"type":"stack","spacing":8,"children":[{{"type":"button","id":{},"label":"OK"}}]}}]}}"#,
                    (*b).id
                )
            );
            for w in [s, s2, l, b] {
                wol_widget_release(w);
            }
            wol_context_release_for_test(c);
        }
    }

    unsafe fn wol_context_release_for_test(c: *mut wol_context) {
        unsafe { crate::context::wol_context_release(c) }
    }

    #[test]
    fn show_without_a_host_says_so() {
        let c = ctx();
        let (mut w, mut e) = (null_mut(), null_mut());
        unsafe {
            assert_eq!(
                wol_window_create(c, c"T".as_ptr(), 1, &mut w, null_mut()),
                0
            );
            let rc = wol_window_show(w, &mut e);
            // The test environment may have a session; only check the error shape.
            if rc != 0 {
                assert_eq!(rc, Status::Unavailable as i32);
                crate::error::wol_error_release(e);
            }
            wol_window_release(w);
            wol_context_release_for_test(c);
        }
    }
}
