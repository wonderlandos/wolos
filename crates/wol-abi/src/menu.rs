// SPDX-License-Identifier: GPL-2.0-or-later
//! Window menus and toolbars (ADR 0020, 1.2): data the Kit host draws, like the parts in `kit`.
//! The host sends back the key chosen; libwol runs the handler only for a key the app added.
use crate::context::{lock, wol_context};
use crate::error::{Error, Result, UserData, guard, input, need, wol_error, wol_status};
use crate::kit::{self, arc_of, wol_window};
use crate::string::{borrow, into_raw, release, retain, wol_string};
use shared::view::{
    MAX_KEY, MAX_LABEL, MAX_MENU_ENTRIES, MAX_SHORTCUT, MAX_TOOLS, Menu, MenuEntry, Tool, Toolbar,
    key_ok, label_ok, shortcut_ok,
};
use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

/// The key is borrowed for the call.
pub type wol_menu_fn = Option<unsafe extern "C" fn(*mut wol_menu, *const wol_string, UserData)>;
pub type wol_toolbar_fn =
    Option<unsafe extern "C" fn(*mut wol_toolbar, *const wol_string, UserData)>;

type Handler<T> = Option<(
    unsafe extern "C" fn(*mut T, *const wol_string, UserData),
    usize,
)>;

pub struct wol_menu {
    id: u32,
    title: String,
    entries: Mutex<Vec<MenuEntry>>,
    handler: Handler<wol_menu>,
    window: Mutex<Weak<wol_window>>,
    /// On a window already.
    placed: AtomicBool,
}

pub struct wol_toolbar {
    id: u32,
    tools: Mutex<Vec<Tool>>,
    handler: Handler<wol_toolbar>,
    window: Mutex<Weak<wol_window>>,
    placed: AtomicBool,
}

fn run<T>(h: Handler<T>, target: &Arc<T>, key: &str) {
    if let Some((f, ud)) = h {
        let k = wol_string::new(key);
        // SAFETY: the app's callback and user data on its thread (inside dispatch); both handles
        // are borrowed and live for the call.
        unsafe {
            f(
                Arc::as_ptr(target).cast_mut(),
                Arc::as_ptr(&k),
                ud as UserData,
            )
        };
    }
}

fn redraw(window: &Mutex<Weak<wol_window>>) {
    if let Some(w) = lock(window).upgrade() {
        kit::refresh_window(&w);
    }
}

fn key_of(ptr: *const c_char, len: usize) -> Result<&'static str> {
    let k = input(ptr, len, MAX_KEY, "The key")?;
    if key_ok(k) {
        Ok(k)
    } else {
        Err(Error::invalid(
            "A key is 1 to 32 lowercase letters, digits, '-' or '_'.",
        ))
    }
}

fn label_of(ptr: *const c_char, len: usize, what: &str) -> Result<String> {
    let l = input(ptr, len, MAX_LABEL, what)?;
    if label_ok(l) {
        Ok(l.to_owned())
    } else {
        Err(Error::invalid(format!(
            "{what} is one line of 1 to {MAX_LABEL} bytes."
        )))
    }
}

fn no_such_key() -> Error {
    Error::invalid("There is no item with that key.")
}

impl wol_menu {
    pub(crate) fn id(&self) -> u32 {
        self.id
    }

    pub(crate) fn data(&self) -> Menu {
        Menu {
            id: self.id,
            title: self.title.clone(),
            entries: lock(&self.entries).clone(),
        }
    }

    /// Place on `window`, once.
    pub(crate) fn attach(&self, window: &Arc<wol_window>) -> Result<()> {
        if self.placed.swap(true, Ordering::AcqRel) {
            return Err(Error::invalid("This menu is already on a window."));
        }
        *lock(&self.window) = Arc::downgrade(window);
        Ok(())
    }

    fn set(&self, key: &str, f: impl FnOnce(&mut bool, &mut bool)) -> Result<()> {
        {
            let mut e = lock(&self.entries);
            let found = e.iter_mut().find_map(|x| match x {
                MenuEntry::Item {
                    key: k,
                    enabled,
                    checked,
                    ..
                } if k == key => Some((enabled, checked)),
                _ => None,
            });
            let (enabled, checked) = found.ok_or_else(no_such_key)?;
            f(enabled, checked);
        }
        redraw(&self.window);
        Ok(())
    }
}

/// The person chose `key`. A key the app never added (a confused host) runs nothing.
pub(crate) fn menu_chosen(m: &Arc<wol_menu>, key: &str) {
    let enabled = lock(&m.entries)
        .iter()
        .any(|e| matches!(e, MenuEntry::Item { key: k, enabled: true, .. } if k == key));
    if enabled {
        run(m.handler, m, key);
    }
}

impl wol_toolbar {
    pub(crate) fn id(&self) -> u32 {
        self.id
    }

    pub(crate) fn data(&self) -> Toolbar {
        Toolbar {
            id: self.id,
            tools: lock(&self.tools).clone(),
        }
    }

    pub(crate) fn attach(&self, window: &Arc<wol_window>) -> Result<()> {
        if self.placed.swap(true, Ordering::AcqRel) {
            return Err(Error::invalid("This toolbar is already on a window."));
        }
        *lock(&self.window) = Arc::downgrade(window);
        Ok(())
    }

    pub(crate) fn detach(&self) {
        *lock(&self.window) = Weak::new();
        self.placed.store(false, Ordering::Release);
    }
}

pub(crate) fn toolbar_chosen(t: &Arc<wol_toolbar>, key: &str) {
    let enabled = lock(&t.tools)
        .iter()
        .any(|x| matches!(x, Tool::Button { key: k, enabled: true, .. } if k == key));
    if enabled {
        run(t.handler, t, key);
    }
}

/// A menu for the window's menu bar; `on_choose` runs with the key of the chosen item.
///
/// # Safety
/// `context` is live; `title` points at `len` bytes; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_menu_create(
    context: *mut wol_context,
    title: *const c_char,
    len: usize,
    on_choose: wol_menu_fn,
    user_data: *mut c_void,
    out: *mut *mut wol_menu,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = unsafe { borrow(context) }?;
        let title = label_of(title, len, "The menu title")?;
        let on_choose = on_choose.ok_or_else(|| Error::invalid("A menu needs a callback."))?;
        let m = wol_menu {
            id: ctx.next_id(),
            title,
            entries: Mutex::default(),
            handler: Some((on_choose, user_data as usize)),
            window: Mutex::default(),
            placed: AtomicBool::new(false),
        };
        unsafe { *out = into_raw(Arc::new(m)) };
        Ok(())
    })
}

/// # Safety
/// `menu` is live; each string points at its length (`shortcut` may be NULL with 0); `error`
/// writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_menu_add_item(
    menu: *mut wol_menu,
    key: *const c_char,
    key_len: usize,
    label: *const c_char,
    label_len: usize,
    shortcut: *const c_char,
    shortcut_len: usize,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let m = unsafe { borrow(menu) }?;
        let key = key_of(key, key_len)?;
        let label = label_of(label, label_len, "The item label")?;
        let shortcut = input(shortcut, shortcut_len, MAX_SHORTCUT, "The shortcut")?;
        if !shortcut.is_empty() && !shortcut_ok(shortcut) {
            return Err(Error::invalid("A shortcut is like cmd+s or cmd+shift+n."));
        }
        {
            let mut e = lock(&m.entries);
            if e.len() >= MAX_MENU_ENTRIES {
                return Err(Error::invalid(format!(
                    "A menu has at most {MAX_MENU_ENTRIES} entries."
                )));
            }
            if e.iter()
                .any(|x| matches!(x, MenuEntry::Item { key: k, .. } if k == key))
            {
                return Err(Error::invalid("Each item needs its own key."));
            }
            e.push(MenuEntry::Item {
                key: key.into(),
                label,
                shortcut: shortcut.into(),
                enabled: true,
                checked: false,
            });
        }
        redraw(&m.window);
        Ok(())
    })
}

/// # Safety
/// `menu` is live; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_menu_add_separator(
    menu: *mut wol_menu,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let m = unsafe { borrow(menu) }?;
        {
            let mut e = lock(&m.entries);
            if e.len() >= MAX_MENU_ENTRIES {
                return Err(Error::invalid(format!(
                    "A menu has at most {MAX_MENU_ENTRIES} entries."
                )));
            }
            e.push(MenuEntry::Separator);
        }
        redraw(&m.window);
        Ok(())
    })
}

/// # Safety
/// `menu` is live; `key` points at `key_len` bytes; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_menu_set_enabled(
    menu: *mut wol_menu,
    key: *const c_char,
    key_len: usize,
    enabled: i32,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let m = unsafe { borrow(menu) }?;
        m.set(key_of(key, key_len)?, |e, _| *e = enabled != 0)
    })
}

/// # Safety
/// `menu` is live; `key` points at `key_len` bytes; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_menu_set_checked(
    menu: *mut wol_menu,
    key: *const c_char,
    key_len: usize,
    checked: i32,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let m = unsafe { borrow(menu) }?;
        m.set(key_of(key, key_len)?, |_, c| *c = checked != 0)
    })
}

/// # Safety
/// `menu` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_menu_retain(menu: *mut wol_menu) -> *mut wol_menu {
    unsafe { retain(menu) }
}

/// # Safety
/// `menu` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_menu_release(menu: *mut wol_menu) {
    unsafe { release(menu) }
}

/// Add a menu to the window's menu bar, after the others. A menu goes on one window.
///
/// # Safety
/// `window` and `menu` are live; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_window_add_menu(
    window: *mut wol_window,
    menu: *mut wol_menu,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || kit::add_menu(&arc_of(window)?, arc_of(menu)?))
}

/// `on_choose` runs with the key of the chosen button.
///
/// # Safety
/// `context` is live; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_toolbar_create(
    context: *mut wol_context,
    on_choose: wol_toolbar_fn,
    user_data: *mut c_void,
    out: *mut *mut wol_toolbar,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = unsafe { borrow(context) }?;
        let on_choose = on_choose.ok_or_else(|| Error::invalid("A toolbar needs a callback."))?;
        let t = wol_toolbar {
            id: ctx.next_id(),
            tools: Mutex::default(),
            handler: Some((on_choose, user_data as usize)),
            window: Mutex::default(),
            placed: AtomicBool::new(false),
        };
        unsafe { *out = into_raw(Arc::new(t)) };
        Ok(())
    })
}

/// `icon` is an icon name, or empty to show `label`; the label is the tooltip either way.
///
/// # Safety
/// `toolbar` is live; each string points at its length (`icon` may be NULL with 0); `error`
/// writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_toolbar_add_button(
    toolbar: *mut wol_toolbar,
    key: *const c_char,
    key_len: usize,
    label: *const c_char,
    label_len: usize,
    icon: *const c_char,
    icon_len: usize,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let t = unsafe { borrow(toolbar) }?;
        let key = key_of(key, key_len)?;
        let label = label_of(label, label_len, "The button label")?;
        let icon = input(icon, icon_len, 128, "The icon")?;
        if !icon.is_empty() && (icon.starts_with('/') || !shared::view::image_source_ok(icon)) {
            return Err(Error::invalid("A tool icon is an icon name."));
        }
        {
            let mut tools = lock(&t.tools);
            if tools.len() >= MAX_TOOLS {
                return Err(Error::invalid(format!(
                    "A toolbar has at most {MAX_TOOLS} parts."
                )));
            }
            if tools
                .iter()
                .any(|x| matches!(x, Tool::Button { key: k, .. } if k == key))
            {
                return Err(Error::invalid("Each button needs its own key."));
            }
            tools.push(Tool::Button {
                key: key.into(),
                label,
                icon: icon.into(),
                enabled: true,
            });
        }
        redraw(&t.window);
        Ok(())
    })
}

/// # Safety
/// `toolbar` is live; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_toolbar_add_spacer(
    toolbar: *mut wol_toolbar,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let t = unsafe { borrow(toolbar) }?;
        {
            let mut tools = lock(&t.tools);
            if tools.len() >= MAX_TOOLS {
                return Err(Error::invalid(format!(
                    "A toolbar has at most {MAX_TOOLS} parts."
                )));
            }
            tools.push(Tool::Spacer);
        }
        redraw(&t.window);
        Ok(())
    })
}

/// # Safety
/// `toolbar` is live; `key` points at `key_len` bytes; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_toolbar_set_enabled(
    toolbar: *mut wol_toolbar,
    key: *const c_char,
    key_len: usize,
    enabled: i32,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let t = unsafe { borrow(toolbar) }?;
        let key = key_of(key, key_len)?;
        {
            let mut tools = lock(&t.tools);
            let found = tools.iter_mut().find_map(|x| match x {
                Tool::Button {
                    key: k, enabled, ..
                } if k == key => Some(enabled),
                _ => None,
            });
            *found.ok_or_else(no_such_key)? = enabled != 0;
        }
        redraw(&t.window);
        Ok(())
    })
}

/// # Safety
/// `toolbar` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_toolbar_retain(toolbar: *mut wol_toolbar) -> *mut wol_toolbar {
    unsafe { retain(toolbar) }
}

/// # Safety
/// `toolbar` is NULL or a reference the caller owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_toolbar_release(toolbar: *mut wol_toolbar) {
    unsafe { release(toolbar) }
}

/// A window has one toolbar; NULL removes it.
///
/// # Safety
/// `window` is live; `toolbar` is NULL or live; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_window_set_toolbar(
    window: *mut wol_window,
    toolbar: *mut wol_toolbar,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let t = if toolbar.is_null() {
            None
        } else {
            Some(arc_of(toolbar)?)
        };
        kit::set_toolbar(&arc_of(window)?, t)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::null_mut;
    use std::sync::atomic::AtomicUsize;

    fn ctx() -> *mut wol_context {
        into_raw(Arc::new(wol_context::new("org.example.t").unwrap()))
    }

    #[test]
    fn menu_rules() {
        static CHOSEN: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn on(_: *mut wol_menu, k: *const wol_string, ud: UserData) {
            assert_eq!(unsafe { (*k).as_str() }, "open");
            assert_eq!(ud as usize, 9);
            CHOSEN.fetch_add(1, Ordering::Relaxed);
        }
        let c = ctx();
        let mut m = null_mut();
        unsafe {
            let create = |t: &std::ffi::CStr, f: wol_menu_fn, o: *mut *mut wol_menu| {
                wol_menu_create(
                    c,
                    t.as_ptr(),
                    t.count_bytes(),
                    f,
                    9 as UserData,
                    o,
                    null_mut(),
                )
            };
            let mut x = null_mut();
            assert_eq!(create(c" ", Some(on), &mut x), 1, "a title");
            assert_eq!(create(c"File", None, &mut x), 1, "a callback");
            assert_eq!(create(c"File", Some(on), &mut m), 0);
            let add = |k: &std::ffi::CStr, l: &std::ffi::CStr, s: &std::ffi::CStr| {
                wol_menu_add_item(
                    m,
                    k.as_ptr(),
                    k.count_bytes(),
                    l.as_ptr(),
                    l.count_bytes(),
                    s.as_ptr(),
                    s.count_bytes(),
                    null_mut(),
                )
            };
            assert_eq!(add(c"open", c"Open", c"cmd+o"), 0);
            assert_eq!(add(c"open", c"Twice", c""), 1, "unique keys");
            assert_eq!(add(c"Save", c"Save", c""), 1, "lowercase keys");
            assert_eq!(add(c"save", c"Save", c"ctrl+s"), 1, "known modifiers");
            assert_eq!(add(c"save", c"a\nb", c""), 1, "one line");
            assert_eq!(add(c"save", c"Save", c""), 0);
            assert_eq!(wol_menu_add_separator(m, null_mut()), 0);
            let set = |k: &std::ffi::CStr, on: i32| {
                wol_menu_set_enabled(m, k.as_ptr(), k.count_bytes(), on, null_mut())
            };
            assert_eq!(set(c"nope", 0), 1, "unknown key");
            assert_eq!(set(c"save", 0), 0);
            assert_eq!(
                wol_menu_set_checked(m, c"open".as_ptr(), 4, 1, null_mut()),
                0
            );
            let d = (*m).data();
            assert!(d.validate().is_ok());
            assert!(matches!(
                d.entries[0],
                MenuEntry::Item { checked: true, .. }
            ));
            assert!(matches!(
                d.entries[1],
                MenuEntry::Item { enabled: false, .. }
            ));

            let a = arc_of(m).unwrap();
            menu_chosen(&a, "open");
            menu_chosen(&a, "save");
            menu_chosen(&a, "forged");
            assert_eq!(
                CHOSEN.load(Ordering::Relaxed),
                1,
                "only an enabled item the app added runs"
            );
            drop(a);
            wol_menu_release(m);
            crate::context::wol_context_release(c);
        }
    }

    #[test]
    fn toolbar_rules() {
        static CHOSEN: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn on(_: *mut wol_toolbar, _: *const wol_string, _: UserData) {
            CHOSEN.fetch_add(1, Ordering::Relaxed);
        }
        let c = ctx();
        let mut t = null_mut();
        unsafe {
            let mut x = null_mut();
            assert_eq!(
                wol_toolbar_create(c, None, null_mut(), &mut x, null_mut()),
                1
            );
            assert_eq!(
                wol_toolbar_create(c, Some(on), null_mut(), &mut t, null_mut()),
                0
            );
            let add = |k: &std::ffi::CStr, i: &std::ffi::CStr| {
                wol_toolbar_add_button(
                    t,
                    k.as_ptr(),
                    k.count_bytes(),
                    c"Save".as_ptr(),
                    4,
                    i.as_ptr(),
                    i.count_bytes(),
                    null_mut(),
                )
            };
            assert_eq!(add(c"save", c"document-save-symbolic"), 0);
            assert_eq!(add(c"save", c""), 1, "unique keys");
            assert_eq!(add(c"pic", c"/etc/shadow.png"), 1, "no paths");
            assert_eq!(wol_toolbar_add_spacer(t, null_mut()), 0);
            let a = arc_of(t).unwrap();
            toolbar_chosen(&a, "save");
            assert_eq!(
                wol_toolbar_set_enabled(t, c"save".as_ptr(), 4, 0, null_mut()),
                0
            );
            toolbar_chosen(&a, "save");
            toolbar_chosen(&a, "forged");
            assert_eq!(CHOSEN.load(Ordering::Relaxed), 1);
            drop(a);
            wol_toolbar_release(t);
            crate::context::wol_context_release(c);
        }
    }

    #[test]
    fn a_window_shows_its_menus_once() {
        let c = ctx();
        unsafe {
            let (mut w, mut w2, mut m, mut t) = (null_mut(), null_mut(), null_mut(), null_mut());
            unsafe extern "C" fn on(_: *mut wol_menu, _: *const wol_string, _: UserData) {}
            unsafe extern "C" fn tb(_: *mut wol_toolbar, _: *const wol_string, _: UserData) {}
            assert_eq!(
                kit::wol_window_create(c, c"A".as_ptr(), 1, &mut w, null_mut()),
                0
            );
            assert_eq!(
                kit::wol_window_create(c, c"B".as_ptr(), 1, &mut w2, null_mut()),
                0
            );
            assert_eq!(
                wol_menu_create(
                    c,
                    c"F".as_ptr(),
                    1,
                    Some(on),
                    null_mut(),
                    &mut m,
                    null_mut()
                ),
                0
            );
            assert_eq!(
                wol_toolbar_create(c, Some(tb), null_mut(), &mut t, null_mut()),
                0
            );
            assert_eq!(wol_window_add_menu(w, m, null_mut()), 0);
            assert_eq!(wol_window_add_menu(w2, m, null_mut()), 1, "one window");
            assert_eq!(wol_window_set_toolbar(w, t, null_mut()), 0);
            assert_eq!(wol_window_set_toolbar(w2, t, null_mut()), 1, "one window");
            assert_eq!(wol_window_set_toolbar(w, null_mut(), null_mut()), 0);
            assert_eq!(wol_window_set_toolbar(w2, t, null_mut()), 0, "free again");
            wol_menu_release(m);
            wol_toolbar_release(t);
            kit::wol_window_release(w);
            kit::wol_window_release(w2);
            crate::context::wol_context_release(c);
        }
    }
}
