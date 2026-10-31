//! Windows and Kit parts as data (ADR 0020). libwol keeps a tree of parts and sends it to the Kit
//! host (the shell) as a `shared::view` message; the host draws it with the Kit and sends what the
//! person does back. Apps link only this BSD library; the Kit itself stays in the host process.
//! A change made through the API redraws the window it is in; a change the person made is only
//! recorded (the host already shows it).
use crate::context::{lock, wol_context};
use crate::error::{Error, Result, Status, UserData, guard, input, need, wol_error, wol_status};
use crate::menu::{wol_menu, wol_toolbar};
use crate::string::{borrow, into_raw, release, retain, wol_string};
use shared::view::{FromHost, IMAGE_SIZES, MAX_TEXT, MAX_TITLE, Node, Role, SPACING, ToHost};
use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

pub type wol_click_fn = Option<unsafe extern "C" fn(*mut wol_widget, UserData)>;
/// Text field, toggle and slider: called after the person changed the value.
pub type wol_change_fn = Option<unsafe extern "C" fn(*mut wol_widget, UserData)>;
pub type wol_window_fn = Option<unsafe extern "C" fn(*mut wol_window, UserData)>;

type Callback<T> = Option<(unsafe extern "C" fn(*mut T, UserData), usize)>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    Stack,
    Row,
    List,
}

enum Kind {
    Container {
        kind: Layout,
        spacing: u8,
        children: Vec<Arc<wol_widget>>,
    },
    Label {
        text: String,
    },
    Button {
        label: String,
        on_click: Callback<wol_widget>,
    },
    Spacer,
    TextField {
        text: String,
        placeholder: String,
        on_change: Callback<wol_widget>,
    },
    Toggle {
        on: bool,
        label: String,
        on_change: Callback<wol_widget>,
    },
    Slider {
        value: f64,
        min: f64,
        max: f64,
        on_change: Callback<wol_widget>,
    },
    Image {
        source: String,
        size: u16,
    },
}

pub struct wol_widget {
    id: u32,
    kind: Mutex<Kind>,
    /// In a container or a window already.
    placed: AtomicBool,
    parent: Mutex<Weak<wol_widget>>,
    window: Mutex<Weak<wol_window>>,
}

pub struct wol_window {
    id: u32,
    title: String,
    context: Arc<wol_context>,
    content: Mutex<Option<Arc<wol_widget>>>,
    shown: AtomicBool,
    on_close: Mutex<Callback<wol_window>>,
    menus: Mutex<Vec<Arc<wol_menu>>>,
    toolbar: Mutex<Option<Arc<wol_toolbar>>>,
}

impl wol_window {
    pub fn id(&self) -> u32 {
        self.id
    }
}

fn callback<T>(f: Option<unsafe extern "C" fn(*mut T, UserData)>, ud: *mut c_void) -> Callback<T> {
    f.map(|f| (f, ud as usize))
}

fn call<T>(cb: Callback<T>, target: &Arc<T>) {
    if let Some((f, ud)) = cb {
        // SAFETY: the app's callback and user data, a borrowed live object, on the app's thread
        // (inside dispatch).
        unsafe { f(Arc::as_ptr(target).cast_mut(), ud as UserData) };
    }
}

impl wol_widget {
    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn node(&self) -> Node {
        match &*lock(&self.kind) {
            Kind::Container {
                kind,
                spacing,
                children,
            } => {
                let children = children.iter().map(|c| c.node()).collect();
                match kind {
                    Layout::Stack => Node::Stack {
                        spacing: *spacing,
                        children,
                    },
                    Layout::Row => Node::Row {
                        spacing: *spacing,
                        children,
                    },
                    Layout::List => Node::List { children },
                }
            }
            Kind::Label { text } => Node::Label {
                text: text.clone(),
                role: Role::Body,
            },
            Kind::Button { label, .. } => Node::Button {
                id: self.id,
                label: label.clone(),
            },
            Kind::Spacer => Node::Spacer,
            Kind::TextField {
                text, placeholder, ..
            } => Node::TextField {
                id: self.id,
                text: text.clone(),
                placeholder: placeholder.clone(),
            },
            Kind::Toggle { on, label, .. } => Node::Toggle {
                id: self.id,
                on: *on,
                label: label.clone(),
            },
            Kind::Slider {
                value, min, max, ..
            } => Node::Slider {
                id: self.id,
                value: *value,
                min: *min,
                max: *max,
            },
            Kind::Image { source, size } => Node::Image {
                source: source.clone(),
                size: *size,
            },
        }
    }

    fn contains(&self, other: &wol_widget) -> bool {
        std::ptr::eq(self, other)
            || match &*lock(&self.kind) {
                Kind::Container { children, .. } => children.iter().any(|c| c.contains(other)),
                _ => false,
            }
    }

    /// The window this part is shown in, through its parents.
    fn root_window(&self) -> Option<Arc<wol_window>> {
        if let Some(w) = lock(&self.window).upgrade() {
            return Some(w);
        }
        lock(&self.parent).upgrade()?.root_window()
    }
}

/// Redraw the window `w` is in, if it is on screen. Errors are dropped: the model changed either
/// way, and the next show sends it.
fn refresh(w: &wol_widget) {
    if let Some(win) = w.root_window()
        && win.shown.load(Ordering::Acquire)
    {
        let _ = show(&win);
    }
}

/// Redraw `win` if it is on screen (see [`refresh`]).
pub(crate) fn refresh_window(win: &Arc<wol_window>) {
    if win.shown.load(Ordering::Acquire) {
        let _ = show(win);
    }
}

pub(crate) fn add_menu(win: &Arc<wol_window>, menu: Arc<wol_menu>) -> Result<()> {
    if lock(&win.menus).len() >= shared::view::MAX_MENUS {
        return Err(Error::invalid(format!(
            "A window has at most {} menus.",
            shared::view::MAX_MENUS
        )));
    }
    menu.attach(win)?;
    lock(&win.menus).push(menu);
    refresh_window(win);
    Ok(())
}

pub(crate) fn set_toolbar(win: &Arc<wol_window>, toolbar: Option<Arc<wol_toolbar>>) -> Result<()> {
    if let Some(t) = &toolbar {
        t.attach(win)?;
    }
    if let Some(old) = std::mem::replace(&mut *lock(&win.toolbar), toolbar) {
        old.detach();
    }
    refresh_window(win);
    Ok(())
}

/// The person chose a menu item or a toolbar button of `win`.
pub(crate) fn chosen(win: &Arc<wol_window>, source: u32, key: &str) {
    let menu = lock(&win.menus).iter().find(|m| m.id() == source).cloned();
    if let Some(m) = menu {
        return crate::menu::menu_chosen(&m, key);
    }
    let toolbar = lock(&win.toolbar).clone().filter(|t| t.id() == source);
    if let Some(t) = toolbar {
        crate::menu::toolbar_chosen(&t, key);
    }
}

/// Something the person did in the host: record it, then run the part's callback.
pub(crate) fn from_host(w: &Arc<wol_widget>, e: &FromHost) {
    let cb = {
        let mut k = lock(&w.kind);
        match (&mut *k, e) {
            (Kind::Button { on_click, .. }, FromHost::Clicked { .. }) => *on_click,
            (
                Kind::TextField {
                    text, on_change, ..
                },
                FromHost::Edited { text: t, .. },
            ) if t.len() <= MAX_TEXT => {
                text.clone_from(t);
                *on_change
            }
            (Kind::Toggle { on, on_change, .. }, FromHost::Toggled { on: o, .. }) => {
                *on = *o;
                *on_change
            }
            (
                Kind::Slider {
                    value,
                    min,
                    max,
                    on_change,
                },
                FromHost::Slid { value: v, .. },
            ) if v.is_finite() => {
                *value = v.clamp(*min, *max);
                *on_change
            }
            _ => None,
        }
    };
    call(cb, w);
}

/// The person closed the window.
pub(crate) fn closed(win: &Arc<wol_window>) {
    win.shown.store(false, Ordering::Release);
    let cb = *lock(&win.on_close);
    call(cb, win);
}

/// An owned Arc from a borrowed handle (+1).
pub(crate) fn arc_of<T>(p: *mut T) -> Result<Arc<T>> {
    if p.is_null() {
        return Err(Error::invalid("A handle is NULL."));
    }
    // SAFETY: live handle from Arc::into_raw (contract); we take a new reference.
    unsafe {
        Arc::increment_strong_count(p.cast_const());
        Ok(Arc::from_raw(p.cast_const()))
    }
}

pub(crate) fn text(ptr: *const c_char, len: usize, max: usize, what: &str) -> Result<String> {
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
        parent: Mutex::default(),
        window: Mutex::default(),
    });
    ctx.register(&w);
    // SAFETY: `out` checked by the caller.
    unsafe { *out = into_raw(w) };
}

fn spacing_of(spacing: u32) -> Result<u8> {
    u8::try_from(spacing)
        .ok()
        .filter(|s| SPACING.contains(s))
        .ok_or_else(|| Error::invalid(format!("Spacing is one of {SPACING:?}.")))
}

fn show(win: &Arc<wol_window>) -> Result<()> {
    let content = lock(&win.content)
        .as_ref()
        .map(|c| c.node())
        .unwrap_or(Node::Stack {
            spacing: 0,
            children: Vec::new(),
        });
    content.validate().map_err(Error::invalid)?;
    let menus: Vec<_> = lock(&win.menus).iter().map(|m| m.data()).collect();
    let toolbar = lock(&win.toolbar).as_ref().map(|t| t.data());
    shared::view::validate_chrome(&menus, toolbar.as_ref()).map_err(Error::invalid)?;
    let m = ToHost::Show {
        window: win.id,
        title: win.title.clone(),
        content,
        menus,
        toolbar,
    };
    win.context.send_kit(&m.to_line())?;
    win.context.register_window(win);
    win.shown.store(true, Ordering::Release);
    Ok(())
}

// ---- 1.0 ----

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
            shown: AtomicBool::new(false),
            on_close: Mutex::default(),
            menus: Mutex::default(),
            toolbar: Mutex::default(),
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
        let win = arc_of(window)?;
        let new = if content.is_null() {
            None
        } else {
            Some(arc_of(content)?)
        };
        if let Some(w) = &new {
            if w.placed.swap(true, Ordering::AcqRel) {
                return Err(Error::invalid(
                    "This part is already in a stack or a window.",
                ));
            }
            *lock(&w.window) = Arc::downgrade(&win);
        }
        if let Some(old) = std::mem::replace(&mut *lock(&win.content), new) {
            old.placed.store(false, Ordering::Release);
            *lock(&old.window) = Weak::new();
        }
        if win.shown.load(Ordering::Acquire) {
            show(&win)?;
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
    guard(error, || show(&arc_of(window)?))
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
        let spacing = spacing_of(spacing)?;
        make(
            ctx,
            Kind::Container {
                kind: Layout::Stack,
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
        if on_click.is_none() {
            return Err(Error::invalid("A button needs a click callback."));
        }
        make(
            ctx,
            Kind::Button {
                label,
                on_click: callback(on_click, user_data),
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
        let p = arc_of(parent)?;
        let c = arc_of(child)?;
        if c.contains(&p) {
            return Err(Error::invalid("A part cannot hold itself."));
        }
        {
            let mut kind = lock(&p.kind);
            let Kind::Container { children, .. } = &mut *kind else {
                return Err(Error::invalid(
                    "Only a stack, row or list holds other parts.",
                ));
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
            *lock(&c.parent) = Arc::downgrade(&p);
            children.push(c);
        }
        refresh(&p);
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

// ---- 1.1 ----

fn container(
    context: *mut wol_context,
    kind: Layout,
    spacing: u32,
    out: *mut *mut wol_widget,
) -> Result<()> {
    need(out, "out")?;
    let ctx = unsafe { borrow(context) }?;
    let spacing = spacing_of(spacing)?;
    make(
        ctx,
        Kind::Container {
            kind,
            spacing,
            children: Vec::new(),
        },
        out,
    );
    Ok(())
}

/// # Safety
/// `context` is live; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_row_create(
    context: *mut wol_context,
    spacing: u32,
    out: *mut *mut wol_widget,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || container(context, Layout::Row, spacing, out))
}

/// # Safety
/// `context` is live; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_list_create(
    context: *mut wol_context,
    out: *mut *mut wol_widget,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || container(context, Layout::List, 0, out))
}

/// # Safety
/// `context` is live; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_spacer_create(
    context: *mut wol_context,
    out: *mut *mut wol_widget,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        make(unsafe { borrow(context) }?, Kind::Spacer, out);
        Ok(())
    })
}

/// # Safety
/// `context` is live; `placeholder` points at `len` bytes; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_text_field_create(
    context: *mut wol_context,
    placeholder: *const c_char,
    len: usize,
    on_change: wol_change_fn,
    user_data: *mut c_void,
    out: *mut *mut wol_widget,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = unsafe { borrow(context) }?;
        let placeholder = text(placeholder, len, MAX_TEXT, "The placeholder")?;
        make(
            ctx,
            Kind::TextField {
                text: String::new(),
                placeholder,
                on_change: callback(on_change, user_data),
            },
            out,
        );
        Ok(())
    })
}

/// # Safety
/// `context` is live; `label` points at `len` bytes (may be empty); `out`, `error` writable or
/// NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_toggle_create(
    context: *mut wol_context,
    label: *const c_char,
    len: usize,
    on: i32,
    on_change: wol_change_fn,
    user_data: *mut c_void,
    out: *mut *mut wol_widget,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = unsafe { borrow(context) }?;
        let label = text(label, len, MAX_TEXT, "The toggle label")?;
        make(
            ctx,
            Kind::Toggle {
                on: on != 0,
                label,
                on_change: callback(on_change, user_data),
            },
            out,
        );
        Ok(())
    })
}

/// # Safety
/// `context` is live; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_slider_create(
    context: *mut wol_context,
    min: f64,
    max: f64,
    value: f64,
    on_change: wol_change_fn,
    user_data: *mut c_void,
    out: *mut *mut wol_widget,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = unsafe { borrow(context) }?;
        let ok = min.is_finite() && max.is_finite() && value.is_finite() && min < max;
        if !ok || value < min || value > max {
            return Err(Error::invalid(
                "A slider needs min < max and a value between them.",
            ));
        }
        make(
            ctx,
            Kind::Slider {
                value,
                min,
                max,
                on_change: callback(on_change, user_data),
            },
            out,
        );
        Ok(())
    })
}

/// # Safety
/// `context` is live; `source` points at `len` bytes; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_image_create(
    context: *mut wol_context,
    source: *const c_char,
    len: usize,
    size: u32,
    out: *mut *mut wol_widget,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let ctx = unsafe { borrow(context) }?;
        let source = input(source, len, 1024, "The image source")?;
        if !shared::view::image_source_ok(source) {
            return Err(Error::invalid(
                "An image is an icon name or an absolute path to a PNG, SVG or JPEG file.",
            ));
        }
        let size = u16::try_from(size)
            .ok()
            .filter(|s| IMAGE_SIZES.contains(s))
            .ok_or_else(|| Error::invalid(format!("Image size is one of {IMAGE_SIZES:?}.")))?;
        make(
            ctx,
            Kind::Image {
                source: source.into(),
                size,
            },
            out,
        );
        Ok(())
    })
}

/// Label text, button label or text field text.
///
/// # Safety
/// `widget` is live; `text` points at `len` bytes; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_widget_set_text(
    widget: *mut wol_widget,
    text_utf8: *const c_char,
    len: usize,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let w = unsafe { borrow(widget) }?;
        let new = text(text_utf8, len, MAX_TEXT, "The text")?;
        {
            let mut k = lock(&w.kind);
            match &mut *k {
                Kind::Label { text } => *text = new,
                Kind::Button { label, .. } if !new.trim().is_empty() => *label = new,
                Kind::Button { .. } => return Err(Error::invalid("A button needs a label.")),
                Kind::TextField { text, .. } if !new.contains('\n') => *text = new,
                Kind::TextField { .. } => {
                    return Err(Error::invalid("A text field holds one line."));
                }
                _ => {
                    return Err(Error::invalid(
                        "Only labels, buttons and text fields have text.",
                    ));
                }
            }
        }
        refresh(w);
        Ok(())
    })
}

/// # Safety
/// `widget` is live; `out`, `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_widget_copy_text(
    widget: *const wol_widget,
    out: *mut *mut wol_string,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        need(out, "out")?;
        let w = unsafe { borrow(widget) }?;
        let s = match &*lock(&w.kind) {
            Kind::Label { text } | Kind::TextField { text, .. } => text.clone(),
            Kind::Button { label, .. } => label.clone(),
            _ => {
                return Err(Error::invalid(
                    "Only labels, buttons and text fields have text.",
                ));
            }
        };
        unsafe { *out = into_raw(wol_string::new(&s)) };
        Ok(())
    })
}

/// 1 when on, 0 when off or not a toggle.
///
/// # Safety
/// `widget` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_toggle_get_on(widget: *const wol_widget) -> i32 {
    unsafe { borrow(widget) }.map_or(0, |w| match &*lock(&w.kind) {
        Kind::Toggle { on, .. } => i32::from(*on),
        _ => 0,
    })
}

/// # Safety
/// `widget` is live; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_toggle_set_on(
    widget: *mut wol_widget,
    on: i32,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let w = unsafe { borrow(widget) }?;
        match &mut *lock(&w.kind) {
            Kind::Toggle { on: o, .. } => *o = on != 0,
            _ => return Err(Error::invalid("This part is not a toggle.")),
        }
        refresh(w);
        Ok(())
    })
}

/// The value, or 0 when not a slider.
///
/// # Safety
/// `widget` is NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_slider_get_value(widget: *const wol_widget) -> f64 {
    unsafe { borrow(widget) }.map_or(0.0, |w| match &*lock(&w.kind) {
        Kind::Slider { value, .. } => *value,
        _ => 0.0,
    })
}

/// # Safety
/// `widget` is live; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_slider_set_value(
    widget: *mut wol_widget,
    value: f64,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let w = unsafe { borrow(widget) }?;
        match &mut *lock(&w.kind) {
            Kind::Slider {
                value: v, min, max, ..
            } if value.is_finite() && value >= *min && value <= *max => *v = value,
            Kind::Slider { .. } => {
                return Err(Error::invalid("The value is outside the slider's range."));
            }
            _ => return Err(Error::invalid("This part is not a slider.")),
        }
        refresh(w);
        Ok(())
    })
}

/// Called when the person closes the window (NULL: no callback).
///
/// # Safety
/// `window` is live; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_window_set_close_handler(
    window: *mut wol_window,
    on_close: wol_window_fn,
    user_data: *mut c_void,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let w = unsafe { borrow(window) }?;
        *lock(&w.on_close) = callback(on_close, user_data);
        Ok(())
    })
}

/// Take the window off screen (it can be shown again).
///
/// # Safety
/// `window` is live; `error` writable or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wol_window_close(
    window: *mut wol_window,
    error: *mut *mut wol_error,
) -> wol_status {
    guard(error, || {
        let w = unsafe { borrow(window) }?;
        if w.shown.swap(false, Ordering::AcqRel) {
            w.context
                .send_kit(&ToHost::Close { window: w.id }.to_line())?;
        }
        Ok(())
    })
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
            let mut x = null_mut();
            assert_eq!(
                wol_button_create(c, c"OK".as_ptr(), 2, None, null_mut(), &mut x, null_mut()),
                1
            );
            assert_eq!(
                wol_label_create(c, c"a\x07".as_ptr(), 2, &mut x, null_mut()),
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
            crate::context::wol_context_release(c);
        }
    }

    #[test]
    fn one_one_parts_and_values() {
        let c = ctx();
        unsafe {
            let (mut row, mut list, mut sp, mut tf, mut tg, mut sl, mut im) = (
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            );
            assert_eq!(wol_row_create(c, 4, &mut row, null_mut()), 0);
            assert_eq!(wol_list_create(c, &mut list, null_mut()), 0);
            assert_eq!(wol_spacer_create(c, &mut sp, null_mut()), 0);
            assert_eq!(
                wol_text_field_create(
                    c,
                    c"Name".as_ptr(),
                    4,
                    Some(nop),
                    null_mut(),
                    &mut tf,
                    null_mut()
                ),
                0
            );
            assert_eq!(
                wol_toggle_create(
                    c,
                    c"Wi-Fi".as_ptr(),
                    5,
                    1,
                    None,
                    null_mut(),
                    &mut tg,
                    null_mut()
                ),
                0
            );
            let mut bad = null_mut();
            assert_eq!(
                wol_slider_create(c, 1.0, 0.0, 0.5, None, null_mut(), &mut bad, null_mut()),
                1
            );
            assert_eq!(
                wol_slider_create(c, 0.0, 10.0, 5.0, None, null_mut(), &mut sl, null_mut()),
                0
            );
            assert_eq!(
                wol_image_create(c, c"x/../y.png".as_ptr(), 10, 32, &mut bad, null_mut()),
                1
            );
            assert_eq!(
                wol_image_create(
                    c,
                    c"weather-clear-symbolic".as_ptr(),
                    22,
                    31,
                    &mut bad,
                    null_mut()
                ),
                1
            );
            assert_eq!(
                wol_image_create(
                    c,
                    c"weather-clear-symbolic".as_ptr(),
                    22,
                    32,
                    &mut im,
                    null_mut()
                ),
                0
            );
            for w in [tf, sp, tg] {
                assert_eq!(wol_widget_add_child(row, w, null_mut()), 0);
            }
            for w in [row, sl, im] {
                assert_eq!(wol_widget_add_child(list, w, null_mut()), 0);
            }
            assert!((*list).node().validate().is_ok());

            assert_eq!(wol_widget_set_text(tf, c"Ada".as_ptr(), 3, null_mut()), 0);
            assert_eq!(wol_widget_set_text(tf, c"a\nb".as_ptr(), 3, null_mut()), 1);
            assert_eq!(wol_widget_set_text(sl, c"x".as_ptr(), 1, null_mut()), 1);
            let mut s = null_mut();
            assert_eq!(wol_widget_copy_text(tf, &mut s, null_mut()), 0);
            assert_eq!((*s).as_str(), "Ada");
            crate::string::wol_string_release(s);
            assert_eq!(wol_toggle_get_on(tg), 1);
            assert_eq!(wol_toggle_set_on(tg, 0, null_mut()), 0);
            assert_eq!(wol_toggle_get_on(tg), 0);
            assert_eq!(wol_toggle_set_on(sl, 0, null_mut()), 1);
            assert_eq!(wol_slider_set_value(sl, 11.0, null_mut()), 1);
            assert_eq!(wol_slider_set_value(sl, 2.5, null_mut()), 0);
            assert_eq!(wol_slider_get_value(sl), 2.5);

            // What the person does lands in the model.
            let tfa = arc_of(tf).unwrap();
            from_host(
                &tfa,
                &FromHost::Edited {
                    window: 1,
                    widget: tfa.id,
                    text: "Grace".into(),
                },
            );
            let sla = arc_of(sl).unwrap();
            from_host(
                &sla,
                &FromHost::Slid {
                    window: 1,
                    widget: sla.id,
                    value: 99.0,
                },
            );
            assert_eq!(wol_slider_get_value(sl), 10.0, "clamped");
            let mut s = null_mut();
            wol_widget_copy_text(tf, &mut s, null_mut());
            assert_eq!((*s).as_str(), "Grace");
            crate::string::wol_string_release(s);
            drop((tfa, sla));
            for w in [row, list, sp, tf, tg, sl, im] {
                wol_widget_release(w);
            }
            crate::context::wol_context_release(c);
        }
    }
}
