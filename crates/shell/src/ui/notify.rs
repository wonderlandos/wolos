//! org.freedesktop.Notifications server (zbus) + banners (top right, 5 s) + Notification Center panel.
use crate::kit::{
    surface::{Keyboard, Spec},
    widgets as w,
};
use gtk::{glib, prelude::*};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedValue;

const PATH: &str = "/org/freedesktop/Notifications";
const NAME: &str = "org.freedesktop.Notifications";

#[derive(Clone, Debug)]
pub struct Note {
    pub id: u32,
    pub app: String,
    pub icon: String,
    pub summary: String,
    pub body: String,
    pub critical: bool,
    pub timeout_ms: i32,
    /// A picture file shown as the banner's thumbnail (`image-path` hint), or empty.
    pub image: String,
    /// (key, label) pairs from the client; see [`buttons`] and [`click_action`].
    pub actions: Vec<(String, String)>,
}

/// Answers note `id`: with an action key (ActionInvoked), or None (dismissed).
type Answer = dyn Fn(u32, Option<&str>);

/// Notices from the system itself (`notify-send -a WolOS`) carry the WolOS mark.
const SYSTEM_APP: &str = "WolOS";
const SYSTEM_ICON: &str = "wolos-mark";

/// The action a click on the banner runs (spec: key "default").
pub const DEFAULT_ACTION: &str = "default";
/// Buttons a banner shows at most.
pub const MAX_BUTTONS: usize = 2;

/// Notify's flat action list (key, label, key, label…) as pairs; a dangling key is dropped.
pub fn parse_actions(flat: &[String]) -> Vec<(String, String)> {
    flat.chunks_exact(2)
        .map(|p| (p[0].clone(), p[1].clone()))
        .collect()
}

/// The banner's buttons: the first [`MAX_BUTTONS`] actions other than the default one.
pub fn buttons(n: &Note) -> Vec<&(String, String)> {
    n.actions
        .iter()
        .filter(|a| a.0 != DEFAULT_ACTION)
        .take(MAX_BUTTONS)
        .collect()
}

/// What a click on the banner does: the default action when the client gave one, else dismiss.
pub fn click_action(n: &Note) -> Option<&'static str> {
    n.actions
        .iter()
        .any(|a| a.0 == DEFAULT_ACTION)
        .then_some(DEFAULT_ACTION)
}

/// Server -> UI.
#[derive(Clone, Debug)]
pub enum Event {
    Show(Note),
    /// CloseNotification from a client: drop the banner (signal already sent).
    Close(u32),
    /// Esc (`wol shell hide banners`): every banner on screen leaves, dismissed.
    DismissAll,
}

/// Horizontal travel (px) that swipes a banner away: a drag toward the screen edge, or a
/// two-finger swipe either way (natural scrolling flips the sign).
const SWIPE: f64 = 64.0;

/// A drag of `dx` px (right = toward the edge) or a touchpad swipe of `scroll` px dismisses.
pub fn swiped(dx: f64, scroll: f64) -> bool {
    dx >= SWIPE || scroll.abs() >= SWIPE
}

/// The banner follows a drag to the right and leaves past [`SWIPE`]; a two-finger swipe too.
fn swipe_away(rev: &gtk::Revealer, done: impl Fn() + 'static) {
    let done = Rc::new(done);
    let drag = gtk::GestureDrag::new();
    {
        let r = rev.clone();
        drag.connect_drag_update(move |_, dx, _| r.set_margin_start(dx.max(0.0) as i32));
    }
    {
        let (r, done) = (rev.clone(), done.clone());
        drag.connect_drag_end(move |_, dx, _| {
            r.set_margin_start(0);
            if swiped(dx, 0.0) {
                done();
            }
        });
    }
    rev.add_controller(drag);
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::HORIZONTAL);
    let travel = Rc::new(std::cell::Cell::new(0.0));
    {
        let travel = travel.clone();
        scroll.connect_scroll_begin(move |_| travel.set(0.0));
    }
    scroll.connect_scroll(move |_, dx, _| {
        travel.set(travel.get() + dx);
        if swiped(0.0, travel.get()) {
            travel.set(0.0);
            done();
        }
        glib::Propagation::Stop
    });
    rev.add_controller(scroll);
}

/// NotificationClosed reasons (Desktop Notifications spec 1.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum CloseReason {
    Expired = 1,
    Dismissed = 2,
    Closed = 3,
}

type Open = Arc<Mutex<HashSet<u32>>>;

/// Takes `id` out of the open set; true when it was open (signal it exactly once).
fn take(open: &Open, id: u32) -> bool {
    open.lock().map(|mut o| o.remove(&id)).unwrap_or(false)
}

struct Server {
    tx: async_channel::Sender<Event>,
    next: AtomicU32,
    open: Open,
}

/// The live bus connection: kept for the process lifetime to emit NotificationClosed.
#[derive(Clone)]
pub struct Bus {
    conn: zbus::Connection,
    open: Open,
}

impl Bus {
    /// Owns `org.freedesktop.Notifications` on `address` (None: session bus).
    pub async fn serve(
        address: Option<&str>,
        tx: async_channel::Sender<Event>,
    ) -> zbus::Result<Bus> {
        let open = Open::default();
        let server = Server {
            tx,
            next: AtomicU32::new(1),
            open: open.clone(),
        };
        let b = match address {
            Some(a) => zbus::connection::Builder::address(a)?,
            None => zbus::connection::Builder::session()?,
        };
        let conn = b.name(NAME)?.serve_at(PATH, server)?.build().await?;
        Ok(Bus { conn, open })
    }

    /// Emits NotificationClosed once per notification; false when it was already closed.
    pub async fn close(&self, id: u32, reason: CloseReason) -> zbus::Result<bool> {
        if !take(&self.open, id) {
            return Ok(false);
        }
        let e = SignalEmitter::new(&self.conn, PATH)?;
        Server::notification_closed(&e, id, reason as u32).await?;
        Ok(true)
    }

    /// A button or the banner was clicked: ActionInvoked, then NotificationClosed (dismissed).
    /// False when the notification was already closed.
    pub async fn invoke(&self, id: u32, key: &str) -> zbus::Result<bool> {
        if !take(&self.open, id) {
            return Ok(false);
        }
        let e = SignalEmitter::new(&self.conn, PATH)?;
        Server::action_invoked(&e, id, key).await?;
        Server::notification_closed(&e, id, CloseReason::Dismissed as u32).await?;
        Ok(true)
    }
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id != 0 {
            replaces_id
        } else {
            self.next.fetch_add(1, Ordering::Relaxed)
        };
        let critical = hints.get("urgency").and_then(|v| u8::try_from(v).ok()) == Some(2);
        if let Ok(mut o) = self.open.lock() {
            o.insert(id);
        }
        let _ = self.tx.try_send(Event::Show(Note {
            id,
            app: app_name,
            icon: app_icon,
            summary,
            body,
            critical,
            timeout_ms: expire_timeout,
            image: image_hint(&hints),
            actions: parse_actions(&actions),
        }));
        id
    }
    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] e: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        if take(&self.open, id) {
            Self::notification_closed(&e, id, CloseReason::Closed as u32).await?;
            let _ = self.tx.try_send(Event::Close(id));
        }
        Ok(())
    }
    #[zbus(signal)]
    async fn notification_closed(e: &SignalEmitter<'_>, id: u32, reason: u32) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn action_invoked(e: &SignalEmitter<'_>, id: u32, action_key: &str) -> zbus::Result<()>;
    fn get_capabilities(&self) -> Vec<String> {
        vec!["actions".into(), "body".into(), "persistence".into()]
    }
    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "wol-shell".into(),
            "WolOS".into(),
            env!("CARGO_PKG_VERSION").into(),
            "1.2".into(),
        )
    }
}

/// Banner lifetime: server default 5 s, -1 = default, 0 = until clicked; critical and banners
/// with buttons stay until answered or dismissed (macOS alerts).
pub fn banner_ms(n: &Note) -> Option<u32> {
    match (n.critical || !buttons(n).is_empty(), n.timeout_ms) {
        (true, _) | (_, 0) => None,
        (_, t) if t > 0 => Some(t as u32),
        _ => Some(5000),
    }
}

/// Focus (Do Not Disturb) keeps the banner away; the note still lands in Notification Center.
pub fn quiet(n: &Note, focus: crate::focus::Focus) -> bool {
    focus.active() && !n.critical
}

/// Where a new banner goes, given the banners on screen as (app, stacked count), top first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// New group at the top.
    Top,
    /// Same app already showing: replace that banner, stack count becomes `count`.
    Stack { index: usize, count: u32 },
}

pub fn place(shown: &[(String, u32)], app: &str) -> Place {
    match shown
        .iter()
        .position(|(a, _)| !app.is_empty() && a.eq_ignore_ascii_case(app))
    {
        Some(index) => Place::Stack {
            index,
            count: shown[index].1 + 1,
        },
        None => Place::Top,
    }
}

/// Notification Center row (same layout as a banner, no stack). No buttons: the banner with
/// buttons stays until answered, and an answered note takes no second answer.
fn card(n: &Note) -> gtk::Box {
    banner(
        &Note {
            actions: vec![],
            ..n.clone()
        },
        1,
    )
}

/// The `image-path` hint (older `image_path`) when it names a picture file: the thumbnail.
pub fn image_hint(hints: &HashMap<String, OwnedValue>) -> String {
    ["image-path", "image_path"]
        .iter()
        .filter_map(|k| hints.get(*k).and_then(|v| <&str>::try_from(v).ok()))
        .find_map(|p| match p {
            p if p.starts_with('/') => Some(p.to_string()),
            p if p.starts_with("file://") => glib::filename_from_uri(p)
                .ok()
                .map(|(f, _)| f.to_string_lossy().into_owned()),
            _ => None,
        })
        .unwrap_or_default()
}

/// macOS banner: 344 px glass card, app icon, app name + "now", bold title, body up to 2 lines.
/// `count` > 1 draws the group's stacked edges under the card. Buttons do nothing here (golden,
/// Notification Center); [`banner_with`] wires them.
pub fn banner(n: &Note, count: u32) -> gtk::Box {
    banner_with(n, count, Rc::new(|_: &str| {}))
}

/// [`banner`] whose buttons run `act(key)`.
pub fn banner_with(n: &Note, count: u32, act: Rc<dyn Fn(&str)>) -> gtk::Box {
    let outer = w::vbox(0);
    outer.add_css_class("banner-group");
    let c = w::glass_panel(0);
    c.add_css_class("notification");
    if n.critical {
        c.add_css_class("critical");
    }
    let row = w::hbox(w::S3);
    let icon = if n.icon.is_empty() && n.app == SYSTEM_APP {
        SYSTEM_ICON.to_string()
    } else if n.icon.is_empty() {
        crate::apps::icon_for(&n.app, crate::apps::PINS, &[])
    } else {
        n.icon.clone()
    };
    let img = if icon.starts_with('/') {
        let i = gtk::Image::from_file(&icon);
        i.set_pixel_size(w::ICON_M);
        i
    } else {
        w::app_icon(&icon, w::ICON_M)
    };
    row.append(&img);
    let text = w::vbox(0);
    let head = w::hbox(w::S2);
    let app = w::label(&n.app.to_uppercase(), "banner-app");
    app.set_hexpand(true);
    app.set_max_width_chars(1);
    app.set_ellipsize(gtk::pango::EllipsizeMode::End);
    app.add_css_class("may-ellipsize");
    head.append(&app);
    head.append(&w::label("now", "banner-time"));
    text.append(&head);
    let title = w::label(&n.summary, "banner-title");
    title.add_css_class("may-ellipsize");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(1);
    text.append(&title);
    if !n.body.is_empty() {
        let b = w::label(&n.body, "banner-body");
        b.set_wrap(true);
        b.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        b.set_lines(2);
        b.set_ellipsize(gtk::pango::EllipsizeMode::End);
        b.add_css_class("may-ellipsize");
        b.set_max_width_chars(1);
        text.append(&b);
    }
    if count > 1 {
        text.append(&w::label(
            &format!("{} more notifications", count - 1),
            "banner-more",
        ));
    }
    let actions = buttons(n);
    if !actions.is_empty() {
        let bar = w::hbox(w::S2);
        bar.add_css_class("banner-actions");
        bar.set_halign(gtk::Align::End);
        for (key, label) in actions {
            let (act, key) = (act.clone(), key.clone());
            bar.append(&w::button(label, w::Kind::Default, move || act(&key)));
        }
        text.append(&bar);
    }
    row.append(&text);
    if !n.image.is_empty() {
        // A screenshot's banner shows the picture, as on macOS.
        let thumb = gtk::Picture::for_filename(&n.image);
        thumb.set_content_fit(gtk::ContentFit::Cover);
        thumb.set_size_request(w::ICON_L, w::ICON_L);
        thumb.set_valign(gtk::Align::Center);
        thumb.set_overflow(gtk::Overflow::Hidden);
        thumb.add_css_class("banner-thumb");
        row.append(&thumb);
    }
    c.append(&row);
    outer.append(&c);
    for edge in ["banner-edge-1", "banner-edge-2"]
        .iter()
        .take((count as usize).saturating_sub(1).min(2))
    {
        let e = w::vbox(0);
        e.add_css_class("banner-edge");
        e.add_css_class(edge);
        outer.append(&e);
    }
    outer
}

/// Starts the D-Bus server; returns the Notification Center panel and "dismiss every banner".
pub fn start(app: &gtk::Application) -> (gtk::Window, Rc<dyn Fn()>) {
    let (tx, rx) = async_channel::unbounded::<Event>();
    let dismiss = {
        let tx = tx.clone();
        Rc::new(move || {
            let _ = tx.try_send(Event::DismissAll);
        })
    };
    let bus: Rc<std::cell::OnceCell<Bus>> = Default::default();
    let b2 = bus.clone();
    glib::spawn_future_local(async move {
        match Bus::serve(None, tx).await {
            Ok(b) => {
                let _ = b2.set(b);
            }
            Err(e) => eprintln!("wol-shell: notifications: {e}"),
        }
    });
    let invoke = {
        let bus = bus.clone();
        move |id: u32, key: String| {
            if let Some(b) = bus.get().cloned() {
                glib::spawn_future_local(async move {
                    if let Err(e) = b.invoke(id, &key).await {
                        eprintln!("wol-shell: ActionInvoked: {e}");
                    }
                });
            }
        }
    };
    let closed = move |id: u32, reason: CloseReason| {
        if let Some(b) = bus.get().cloned() {
            glib::spawn_future_local(async move {
                if let Err(e) = b.close(id, reason).await {
                    eprintln!("wol-shell: NotificationClosed: {e}");
                }
            });
        }
    };

    let banners = super::window(
        app,
        "banners",
        Spec::panel(
            "wol-panel-banners",
            [true, false, false, true],
            Keyboard::None,
        ),
    );
    let stack = w::vbox(0);
    stack.add_css_class("banner-stack");
    // Width first: a Fill-height stack would ask the 2-line bodies for width-for-height and widen.
    stack.set_valign(gtk::Align::Start);
    stack.set_halign(gtk::Align::End);
    banners.set_child(Some(&stack));
    let center_list = w::vbox(0);
    let g = w::glass_panel(w::S2);
    g.append(&w::label("Notifications", "title"));
    g.append(
        &gtk::ScrolledWindow::builder()
            .child(&center_list)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build(),
    );
    let center = super::window(
        app,
        "panel",
        Spec::panel(
            "wol-panel-notifications",
            [true, true, false, true],
            Keyboard::OnDemand,
        ),
    );
    center.add_css_class("notifications");
    center.set_child(Some(&g));
    // Esc closes Notification Center, like every other panel.
    let esc = gtk::EventControllerKey::new();
    {
        let c = center.clone();
        esc.connect_key_pressed(move |_, k, _, _| {
            if k == gtk::gdk::Key::Escape {
                c.set_visible(false);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    center.add_controller(esc);

    glib::spawn_future_local(async move {
        // One banner per app, newest group on top; a group's ids leave together.
        struct Group {
            app: String,
            ids: Vec<u32>,
            rev: gtk::Revealer,
            generation: u32,
            /// The note on screen and what a click on it runs ([`click_action`]).
            last: (u32, Option<&'static str>),
        }
        let groups: Rc<RefCell<Vec<Group>>> = Default::default();
        let next_gen = Rc::new(std::cell::Cell::new(0u32));
        // Removes the group holding `rev`; returns its ids.
        let drop_group = {
            let (groups, stack, banners) = (groups.clone(), stack.clone(), banners.clone());
            move |rev: &gtk::Revealer| -> Vec<u32> {
                let mut g = groups.borrow_mut();
                let Some(i) = g.iter().position(|x| &x.rev == rev) else {
                    return vec![];
                };
                let gr = g.remove(i);
                stack.remove(&gr.rev);
                if g.is_empty() {
                    banners.set_visible(false);
                }
                gr.ids
            }
        };
        // Answers note `id`'s group: `key` goes to `id` (ActionInvoked), the rest are dismissed.
        let answer: Rc<Answer> = {
            let (groups, drop_group, closed) = (groups.clone(), drop_group.clone(), closed.clone());
            Rc::new(move |id, key| {
                let rev = groups
                    .borrow()
                    .iter()
                    .find(|g| g.ids.contains(&id))
                    .map(|g| g.rev.clone());
                let Some(rev) = rev else { return };
                for i in drop_group(&rev) {
                    match key {
                        Some(k) if i == id => invoke(i, k.to_string()),
                        _ => closed(i, CloseReason::Dismissed),
                    }
                }
            })
        };
        let act = |id: u32| -> Rc<dyn Fn(&str)> {
            let a = answer.clone();
            Rc::new(move |k: &str| a(id, Some(k)))
        };
        while let Ok(ev) = rx.recv().await {
            let n = match ev {
                Event::Show(n) => n,
                Event::DismissAll => {
                    let all: Vec<gtk::Revealer> =
                        groups.borrow().iter().map(|g| g.rev.clone()).collect();
                    for rev in all {
                        for id in drop_group(&rev) {
                            closed(id, CloseReason::Dismissed);
                        }
                    }
                    continue;
                }
                Event::Close(id) => {
                    let rev = {
                        let mut g = groups.borrow_mut();
                        let Some(gr) = g.iter_mut().find(|x| x.ids.contains(&id)) else {
                            continue;
                        };
                        gr.ids.retain(|&x| x != id);
                        gr.ids.is_empty().then(|| gr.rev.clone())
                    };
                    if let Some(rev) = rev {
                        drop_group(&rev);
                    }
                    continue;
                }
            };
            center_list.prepend(&card(&n));
            if quiet(&n, crate::focus::read()) {
                continue;
            }
            let generation = next_gen.get().wrapping_add(1);
            next_gen.set(generation);
            let shown: Vec<(String, u32)> = groups
                .borrow()
                .iter()
                .map(|g| (g.app.clone(), g.ids.len() as u32))
                .collect();
            let rev = match place(&shown, &n.app) {
                Place::Stack { index, count } => {
                    let mut g = groups.borrow_mut();
                    let gr = &mut g[index];
                    if !gr.ids.contains(&n.id) {
                        gr.ids.push(n.id);
                    }
                    gr.generation = generation;
                    gr.last = (n.id, click_action(&n));
                    debug_assert!(count >= gr.ids.len() as u32);
                    gr.rev
                        .set_child(Some(&banner_with(&n, gr.ids.len() as u32, act(n.id))));
                    gr.rev.clone()
                }
                Place::Top => {
                    let rev = gtk::Revealer::builder()
                        .transition_type(gtk::RevealerTransitionType::SlideLeft)
                        .child(&banner_with(&n, 1, act(n.id)))
                        .build();
                    // A click runs the default action when there is one, else dismisses.
                    let click = gtk::GestureClick::new();
                    let (r2, groups2, answer2) = (rev.clone(), groups.clone(), answer.clone());
                    click.connect_released(move |_, _, _, _| {
                        let last = groups2
                            .borrow()
                            .iter()
                            .find(|g| g.rev == r2)
                            .map(|g| g.last);
                        if let Some((id, key)) = last {
                            answer2(id, key);
                        }
                    });
                    rev.add_controller(click);
                    let (r2, drop2, closed2) = (rev.clone(), drop_group.clone(), closed.clone());
                    swipe_away(&rev, move || {
                        for id in drop2(&r2) {
                            closed2(id, CloseReason::Dismissed);
                        }
                    });
                    stack.prepend(&rev);
                    groups.borrow_mut().insert(
                        0,
                        Group {
                            app: n.app.clone(),
                            ids: vec![n.id],
                            rev: rev.clone(),
                            generation,
                            last: (n.id, click_action(&n)),
                        },
                    );
                    banners.set_visible(true);
                    let r2 = rev.clone();
                    glib::idle_add_local_once(move || r2.set_reveal_child(true));
                    rev
                }
            };
            if let Some(ms) = banner_ms(&n) {
                let (groups, drop_group, closed) =
                    (groups.clone(), drop_group.clone(), closed.clone());
                glib::timeout_add_local_once(
                    std::time::Duration::from_millis(ms as u64),
                    move || {
                        // Only the group's latest note expires it (a newer note has its own timer).
                        let live = groups
                            .borrow()
                            .iter()
                            .any(|g| g.rev == rev && g.generation == generation);
                        if live {
                            for id in drop_group(&rev) {
                                closed(id, CloseReason::Expired);
                            }
                        }
                    },
                );
            }
        }
    });
    (center.upcast(), dismiss)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn n(critical: bool, t: i32) -> Note {
        Note {
            id: 1,
            app: "".into(),
            icon: "".into(),
            summary: "".into(),
            body: "".into(),
            critical,
            timeout_ms: t,
            image: "".into(),
            actions: vec![],
        }
    }

    /// Actions arrive flat (key, label, key, label…); a dangling key is dropped.
    #[test]
    fn actions_pair_up() {
        let flat: Vec<String> = ["default", "Open", "restart", "Restart", "later"]
            .map(String::from)
            .into();
        assert_eq!(
            parse_actions(&flat),
            [
                ("default".to_string(), "Open".to_string()),
                ("restart".to_string(), "Restart".to_string())
            ]
        );
    }

    /// The banner shows at most two buttons; "default" is a click on the banner, not a button.
    #[test]
    fn two_buttons_at_most_and_default_is_the_banner() {
        let a = |pairs: &[(&str, &str)]| {
            let mut x = n(false, -1);
            x.actions = pairs
                .iter()
                .map(|(k, l)| (k.to_string(), l.to_string()))
                .collect();
            x
        };
        let x = a(&[
            ("default", "Open"),
            ("restart", "Restart"),
            ("later", "Later"),
            ("x", "X"),
        ]);
        let keys: Vec<&str> = buttons(&x).iter().map(|b| b.0.as_str()).collect();
        assert_eq!(keys, ["restart", "later"]);
        assert_eq!(click_action(&x), Some("default"));
        let y = a(&[("restart", "Restart")]);
        assert_eq!(buttons(&y).len(), 1);
        assert_eq!(click_action(&y), None, "a plain click dismisses");
        // An action keeps the banner up until answered or dismissed.
        assert_eq!(banner_ms(&y), None);
        assert_eq!(banner_ms(&n(false, -1)), Some(5000));
    }

    /// wol-screenshot passes the picture as `image-path`: the banner shows it as a thumbnail.
    #[test]
    fn image_hint_is_a_file_thumbnail() {
        let hint = |k: &str, v: &str| {
            HashMap::from([(
                k.to_string(),
                OwnedValue::try_from(zbus::zvariant::Value::from(v)).unwrap(),
            )])
        };
        for (k, v, want) in [
            (
                "image-path",
                "/home/a/Desktop/Screenshot.png",
                "/home/a/Desktop/Screenshot.png",
            ),
            ("image_path", "/tmp/b.png", "/tmp/b.png"),
            ("image-path", "file:///tmp/c%20d.png", "/tmp/c d.png"),
            ("image-path", "dialog-information", ""),
            ("urgency", "/tmp/e.png", ""),
        ] {
            assert_eq!(image_hint(&hint(k, v)), want, "{k} {v}");
        }
    }

    /// A banner leaves when dragged toward the screen edge or swiped with two fingers (J13).
    #[test]
    fn banners_swipe_away() {
        for (dx, scroll, gone) in [
            (SWIPE, 0.0, true),
            (SWIPE - 1.0, 0.0, false),
            (-SWIPE * 2.0, 0.0, false),
            (0.0, SWIPE, true),
            (0.0, -SWIPE, true),
            (0.0, SWIPE / 2.0, false),
        ] {
            assert_eq!(swiped(dx, scroll), gone, "{dx} {scroll}");
        }
    }

    #[test]
    fn banners_group_per_app() {
        let shown = vec![("Mail".to_string(), 1), ("Music".to_string(), 3)];
        for (app, want) in [
            ("Mail", Place::Stack { index: 0, count: 2 }),
            ("music", Place::Stack { index: 1, count: 4 }),
            ("Calendar", Place::Top),
            ("", Place::Top),
        ] {
            assert_eq!(place(&shown, app), want, "{app}");
        }
        assert_eq!(place(&[], "Mail"), Place::Top);
    }

    #[test]
    fn banner_lifetimes() {
        for (c, t, want) in [
            (false, -1, Some(5000)),
            (false, 2000, Some(2000)),
            (false, 0, None),
            (true, 3000, None),
        ] {
            assert_eq!(banner_ms(&n(c, t)), want);
        }
    }

    /// Focus on: the note goes to Notification Center without a banner; critical ones still show.
    #[test]
    fn focus_silences_banners() {
        use crate::focus::Focus;
        assert!(quiet(&n(false, -1), Focus::On));
        assert!(quiet(&n(false, -1), Focus::Until(9)));
        assert!(!quiet(&n(true, -1), Focus::On));
        assert!(!quiet(&n(false, -1), Focus::Off));
    }
}
