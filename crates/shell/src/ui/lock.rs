//! Lock screen (ext-session-lock, ADR 0022/0023) and login window (ADR 0024): clock, avatar, password.
use crate::kit::{self, reveal, widgets as w};
use crate::login::{self, Power, Verdict};
use gtk::{glib, prelude::*};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const WALLPAPER_DIRS: [&str; 2] = [
    "/usr/local/share/wolos/wallpapers",
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../design/wallpapers"),
];
/// The backdrop is decoded small, blurred on the CPU (edge-clamped: no dark rim) and scaled up.
const BACKDROP_PX: (i32, i32) = (480, 270);
const BLUR_RADIUS: usize = 6;
const BLUR_PASSES: usize = 3;

/// Separable box blur, edge clamped, in place; `c` channels per pixel, rows tightly packed.
/// Three passes approximate a Gaussian.
pub fn box_blur(px: &mut [u8], w: usize, h: usize, c: usize, r: usize) {
    let mut line = Vec::new();
    let mut pass = |px: &mut [u8], n: usize, lines: usize, step: usize, next: usize| {
        for l in 0..lines {
            let base = l * next;
            for ch in 0..c {
                line.clear();
                line.extend((0..n).map(|i| px[base + i * step + ch] as u32));
                let at = |i: isize| line[i.clamp(0, n as isize - 1) as usize];
                let d = (2 * r + 1) as u32;
                let mut sum: u32 = (-(r as isize)..=r as isize).map(at).sum();
                for i in 0..n {
                    px[base + i * step + ch] = ((sum + d / 2) / d) as u8;
                    sum += at(i as isize + r as isize + 1);
                    sum -= at(i as isize - r as isize);
                }
            }
        }
    };
    for _ in 0..BLUR_PASSES {
        pass(px, w, h, c, w * c);
        pass(px, h, w, w * c, c);
    }
}

/// Wallpaper -> small blurred texture. Blocking; run off the GTK thread.
fn blurred_wallpaper(path: &Path) -> Option<gtk::gdk::MemoryTexture> {
    let (bw, bh) = BACKDROP_PX;
    let pb = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(path, bw, bh, false).ok()?;
    let (w, h, c) = (
        pb.width() as usize,
        pb.height() as usize,
        pb.n_channels() as usize,
    );
    let stride = pb.rowstride() as usize;
    let bytes = pb.read_pixel_bytes();
    let mut px: Vec<u8> = (0..h)
        .flat_map(|y| &bytes[y * stride..y * stride + w * c])
        .copied()
        .collect();
    box_blur(&mut px, w, h, c, BLUR_RADIUS);
    let format = if c == 4 {
        gtk::gdk::MemoryFormat::R8g8b8a8
    } else {
        gtk::gdk::MemoryFormat::R8g8b8
    };
    Some(gtk::gdk::MemoryTexture::new(
        w as i32,
        h as i32,
        format,
        &glib::Bytes::from_owned(px),
        w * c,
    ))
}

/// Lock/login wallpaper: night in dark, day in light; installed copy first.
pub fn wallpaper_file(dark: bool) -> Option<PathBuf> {
    let name = if dark {
        "wonderland-night.svg"
    } else {
        "wonderland-day.svg"
    };
    WALLPAPER_DIRS
        .iter()
        .map(|d| Path::new(d).join(name))
        .find(|p| p.exists())
}

pub(crate) fn prefers_dark() -> bool {
    gtk::Settings::default().is_some_and(|s| s.is_gtk_application_prefer_dark_theme())
}

thread_local! {
    /// Blurred wallpaper per file: decoded once per process.
    static BACKDROPS: RefCell<std::collections::HashMap<PathBuf, gtk::gdk::MemoryTexture>> =
        Default::default();
}

/// The blurred wallpaper is decoded: a first surface may fade in (`kit::reveal`).
pub(crate) fn backdrop_ready() -> bool {
    BACKDROPS.with_borrow(|c| !c.is_empty())
}

/// Runs `f` on light/dark changes while `owner` lives (the handler is dropped with it).
fn on_dark_change(owner: &impl IsA<gtk::Widget>, f: impl Fn() + 'static) {
    let Some(s) = gtk::Settings::default() else {
        return;
    };
    let id = s.connect_gtk_application_prefer_dark_theme_notify(move |_| f());
    let id = RefCell::new(Some(id));
    owner.connect_destroy(move |_| {
        if let Some(id) = id.take() {
            s.disconnect(id);
        }
    });
}

/// Blurred wallpaper, decoded off the main thread (synchronously in fixed-time golden mode) and cached;
/// follows the light/dark preference.
pub(crate) fn backdrop() -> gtk::Picture {
    let pic = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .build();
    pic.add_css_class("lock-backdrop");
    let weak = pic.downgrade();
    let load = move || {
        let (Some(pic), Some(path)) = (weak.upgrade(), wallpaper_file(prefers_dark())) else {
            return;
        };
        if let Some(t) = BACKDROPS.with_borrow(|c| c.get(&path).cloned()) {
            pic.set_paintable(Some(&t));
            return;
        }
        let store = |path: PathBuf, t: gtk::gdk::MemoryTexture, pic: &gtk::Picture| {
            pic.set_paintable(Some(&t));
            BACKDROPS.with_borrow_mut(|c| c.insert(path, t));
        };
        if std::env::var_os("WOL_FAKE_NOW").is_some() {
            if let Some(t) = blurred_wallpaper(&path) {
                store(path, t, &pic);
            }
            return;
        }
        glib::spawn_future_local(async move {
            let p2 = path.clone();
            if let Ok(Some(t)) = gtk::gio::spawn_blocking(move || blurred_wallpaper(&p2)).await {
                store(path, t, &pic);
            }
        });
    };
    load();
    on_dark_change(&pic, load);
    pic
}

/// Wallpaper + scrim under `content`.
pub(crate) fn screen(content: &impl IsA<gtk::Widget>) -> gtk::Widget {
    let o = gtk::Overlay::new();
    o.add_css_class("lock-screen");
    let tone = {
        let weak = o.downgrade();
        move || {
            let Some(o) = weak.upgrade() else {
                return;
            };
            if prefers_dark() {
                o.remove_css_class("light")
            } else {
                o.add_css_class("light")
            }
        }
    };
    tone();
    on_dark_change(&o, tone);
    o.set_child(Some(&backdrop()));
    let scrim = w::vbox(0);
    scrim.add_css_class("lock-scrim");
    scrim.append(content);
    content.set_vexpand(true);
    o.add_overlay(&scrim);
    o.upcast()
}

/// Clock on top, `bottom` (user panel, power row) at the foot.
fn body(middle: &impl IsA<gtk::Widget>, footer: Option<&gtk::Widget>) -> gtk::Box {
    let b = w::vbox(0);
    b.add_css_class("lock-body");
    b.append(&w::clock_header());
    let s = w::vbox(0);
    s.set_vexpand(true);
    b.append(&s);
    b.append(middle);
    if let Some(f) = footer {
        b.append(f);
    }
    b
}

/// The lock screen for one user panel.
pub fn lock_screen(panel: &w::UserPanel) -> gtk::Widget {
    screen(&body(&panel.root, None))
}

/// Lock screen whose password can't be checked (wol-authcheck missing or broken): Restart
/// appears under it, so nobody is stuck behind it. `shutdown` is the operator group's (every
/// WolOS account); nothing of the session is shown or kept.
fn lock_screen_with_restart(panel: &w::UserPanel) -> (gtk::Widget, gtk::Box) {
    let (restart, _) = action(
        login::power_icon(login::Power::Restart),
        login::power_label(login::Power::Restart),
        || kit::spawn(&login::RESTART_NOW.map(String::from)),
    );
    let foot = w::hbox(0);
    foot.add_css_class("login-power");
    foot.set_halign(gtk::Align::Center);
    foot.append(&restart);
    foot.set_visible(false);
    (screen(&body(&panel.root, Some(foot.upcast_ref()))), foot)
}

/// Login window: user tiles, then the picked user's password pill; power row at the foot.
pub struct Login {
    pub root: gtk::Widget,
    pub stack: gtk::Stack,
    pub panel: Rc<w::UserPanel>,
    pub back: gtk::Button,
    selected: Rc<RefCell<String>>,
    failures: std::cell::Cell<u32>,
}

impl Login {
    /// `users`: (login name, name shown), see [`login::login_users`].
    pub fn new(users: &[(String, String)]) -> Rc<Login> {
        let panel = Rc::new(w::UserPanel::new(
            users.first().map_or("", |u| u.1.as_str()),
        ));
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .vhomogeneous(false)
            .hhomogeneous(false)
            .build();
        let tiles = w::hbox(w::S8);
        tiles.set_halign(gtk::Align::Center);
        tiles.add_css_class("login-tiles");
        stack.add_named(&tiles, Some("users"));
        let pw = w::vbox(w::S4);
        pw.append(&panel.root);
        let (back_box, back) = action("go-previous-symbolic", "Back", || {});
        pw.append(&back_box);
        stack.add_named(&pw, Some("password"));
        let foot = w::hbox(w::S8);
        foot.set_halign(gtk::Align::Center);
        foot.add_css_class("login-power");
        for p in Power::ALL {
            let run = move || {
                gtk::gio::spawn_blocking(move || login::power(login::LOGIN_SOCKET, p));
            };
            foot.append(&action(login::power_icon(p), login::power_label(p), run).0);
        }
        let root = screen(&body(&stack, Some(foot.upcast_ref())));
        let l = Rc::new(Login {
            root,
            stack,
            panel,
            back,
            selected: Default::default(),
            failures: Default::default(),
        });
        for (u, shown) in users {
            let t = gtk::Button::builder().build();
            t.add_css_class("login-tile");
            let c = w::vbox(w::S3);
            let av = w::avatar(shown, w::AVATAR_XL);
            if let Some(tone) = login::read_avatar(u) {
                kit::assistant::set_tone(&av, tone);
            }
            c.append(&av);
            c.append(&w::centered(shown, "lock-name"));
            t.set_child(Some(&c));
            let (l2, u, shown) = (Rc::downgrade(&l), u.clone(), shown.clone());
            t.connect_clicked(move |_| {
                if let Some(l) = l2.upgrade() {
                    l.pick(&u, &shown)
                }
            });
            tiles.append(&t);
        }
        let l2 = Rc::downgrade(&l);
        l.back.connect_clicked(move |_| {
            if let Some(l) = l2.upgrade() {
                l.stack.set_visible_child_name("users")
            }
        });
        if let [(only, shown)] = users {
            l.pick(only, shown);
            if let Some(b) = l.back.parent() {
                b.set_visible(false);
            }
        }
        l
    }

    /// Shows `user`'s password pill, titled with `shown` (full name).
    pub fn pick(&self, user: &str, shown: &str) {
        *self.selected.borrow_mut() = user.to_string();
        self.failures.set(0);
        self.panel.set_user(shown);
        self.stack.set_visible_child_name("password");
        self.panel.field.grab_focus();
    }

    /// Wrong password: after three, the account's hint replaces the message.
    pub fn failed(&self, v: Verdict) {
        if v != Verdict::Denied {
            return;
        }
        self.failures.set(self.failures.get() + 1);
        let user = self.selected();
        if let Some(h) =
            login::hint_message(self.failures.get(), login::read_hint(&user).as_deref())
        {
            self.panel.message.set_label(&h);
        }
    }

    pub fn selected(&self) -> String {
        self.selected.borrow().clone()
    }
}

/// Round glass icon with a caption (power row).
fn action(icon: &str, label: &str, run: impl Fn() + 'static) -> (gtk::Box, gtk::Button) {
    let v = w::vbox(w::S2);
    let b = gtk::Button::builder()
        .child(&w::icon(icon, w::ICON))
        .tooltip_text(label)
        .halign(gtk::Align::Center)
        .build();
    b.add_css_class("lock-round");
    b.connect_clicked(move |_| run());
    v.append(&b);
    v.append(&w::centered(label, "lock-caption"));
    (v, b)
}

pub fn user_name() -> String {
    std::env::var("USER").unwrap_or_else(|_| "user".into())
}

/// Wires Enter -> blocking check off the main thread -> `ok(password)` or shake.
fn wire<C: FnOnce(String) -> Verdict + Send + 'static>(
    panel: Rc<w::UserPanel>,
    check: impl Fn() -> C + 'static,
    ok: impl Fn(String) + Clone + 'static,
    failed: impl Fn(Verdict) + Clone + 'static,
) {
    let check = Rc::new(check);
    let p2 = panel.clone();
    panel.field.connect_activate(move |f| {
        let pw = f.text().to_string();
        let typed = pw.clone();
        f.set_sensitive(false);
        p2.message.set_label("");
        let (p, check, ok, failed) = (p2.clone(), check(), ok.clone(), failed.clone());
        glib::spawn_future_local(async move {
            let v = gtk::gio::spawn_blocking(move || check(pw))
                .await
                .unwrap_or(Verdict::Error);
            p.field.set_sensitive(true);
            if v == Verdict::Ok {
                ok(typed)
            } else {
                p.reject(v.message());
                p.field.grab_focus();
                failed(v);
            }
        });
    });
}

/// `wol-lock [--startup]`: a supervisor that keeps a lock screen up until one reports a real
/// unlock (ADR 0012). A crashed lock screen leaves the session locked (ext-session-lock) and the next
/// one takes it over (`misc:allow_session_lock_restore`). Nothing here ever unlocks.
pub fn run_lock() -> glib::ExitCode {
    if std::env::var_os(login::LOCK_UI_ENV).is_some() {
        let startup = std::env::args().any(|a| a == login::STARTUP);
        return run_lock_screen(startup);
    }
    let ui_args = login::lock_ui_args(&std::env::args().skip(1).collect::<Vec<_>>());
    let exe = std::env::current_exe().unwrap_or_else(|_| "wol-lock".into());
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    let runtime = var("XDG_RUNTIME_DIR");
    let socket = login::wayland_socket(&runtime, &var("WAYLAND_DISPLAY"));
    // One supervisor per session: a second Super+L, or the restarted compositor's --locked-cmd
    // while the previous supervisor already brings a lock screen back.
    let guard = std::fs::File::create(std::path::Path::new(&runtime).join(login::LOCK_GUARD));
    if let Ok(g) = &guard
        && g.try_lock().is_err()
    {
        return glib::ExitCode::SUCCESS;
    }
    let mut attempt = 0;
    loop {
        let out = std::process::Command::new(&exe)
            .args(&ui_args)
            .env(login::LOCK_UI_ENV, "1")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .output();
        let unlocked = out
            .as_ref()
            .is_ok_and(|o| login::reported_unlock(&o.stdout));
        match login::lock_exit(unlocked, socket.exists(), attempt) {
            login::LockExit::Done => {
                return if unlocked {
                    glib::ExitCode::SUCCESS
                } else {
                    glib::ExitCode::FAILURE
                };
            }
            login::LockExit::Respawn(delay) => {
                let st = out
                    .map(|o| o.status.to_string())
                    .unwrap_or_else(|e| e.to_string());
                eprintln!(
                    "wol-lock: lock screen ended without an unlock ({st}); starting a new one"
                );
                std::thread::sleep(delay);
                attempt += 1;
            }
        }
    }
}

/// The lock screen itself. `startup`: fades in from the compositor's black, and the password
/// typed here also opens the login keyring (nobody typed one at boot).
fn run_lock_screen(startup: bool) -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("org.wolos.lock")
        .build();
    app.connect_activate(move |app| {
        kit::style::install("");
        let win = gtk::ApplicationWindow::new(app);
        win.add_css_class("lock-window");
        let panel = Rc::new(w::UserPanel::new(&login::display_name(
            &user_name(),
            &std::fs::read_to_string("/etc/passwd").unwrap_or_default(),
        )));
        // The avatar in the colour picked in Setup, as on the login window.
        if let (Some(tone), Some(av)) = (login::read_avatar(&user_name()), panel.root.first_child())
        {
            kit::assistant::set_tone(&av, tone);
        }
        let (screen, restart) = lock_screen_with_restart(&panel);
        let shown = reveal::Reveal::new(&screen, startup);
        win.set_child(Some(&shown.root));
        if startup {
            shown.show_when(backdrop_ready, || reveal::boot_mark("lock shown"));
        }
        kit::snapshot_on_map(&win, "lock");
        let unlock = lock_surface(app, &win);
        let app2 = app.clone();
        wire(
            panel.clone(),
            || login::authcheck_owned,
            move |password| {
                unlock();
                println!("{}", login::UNLOCKED);
                if !startup {
                    app2.quit();
                    return;
                }
                let app3 = app2.clone();
                glib::spawn_future_local(async move {
                    let _ = gtk::gio::spawn_blocking(move || {
                        let r = zbus::blocking::Connection::session()
                            .and_then(|c| login::unlock_login_keyring(&c, &password));
                        if let Err(e) = r {
                            eprintln!("wol-lock: login keyring: {e}");
                        }
                    })
                    .await;
                    app3.quit();
                });
            },
            move |v| restart.set_visible(v == Verdict::Error),
        );
        panel.field.grab_focus();
    });
    app.run_with_args::<&str>(&[])
}

/// Session lock on every monitor (this window on the first); returns the unlock action.
#[cfg(feature = "layer-shell")]
fn lock_surface(app: &gtk::Application, win: &gtk::ApplicationWindow) -> Rc<dyn Fn()> {
    use gtk::gdk;
    if !gtk4_session_lock::is_supported() {
        win.fullscreen();
        win.present();
        return Rc::new(|| {});
    }
    let lock = gtk4_session_lock::Instance::new();
    let (app, first) = (app.clone(), win.clone());
    let used = std::cell::Cell::new(false);
    lock.connect_monitor(move |l, m: &gdk::Monitor| {
        let w = if !used.replace(true) {
            first.clone()
        } else {
            let w = gtk::ApplicationWindow::new(&app);
            w.add_css_class("lock-window");
            w
        };
        l.assign_window_to_monitor(&w, m);
    });
    lock.lock();
    Rc::new(move || lock.unlock())
}

#[cfg(not(feature = "layer-shell"))]
fn lock_surface(_app: &gtk::Application, win: &gtk::ApplicationWindow) -> Rc<dyn Fn()> {
    eprintln!("wol-lock: built without layer-shell: NOT a secure lock (dev window)");
    win.present();
    Rc::new(|| {})
}

pub fn run_login() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("org.wolos.loginwindow")
        .build();
    app.connect_activate(|app| {
        kit::style::install("");
        let win = gtk::ApplicationWindow::new(app);
        win.add_css_class("lock-window");
        let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
        let shells = std::fs::read_to_string("/etc/shells").unwrap_or_default();
        let l = Login::new(&login::login_users(&passwd, &shells));
        // Fades in from the compositor's black, and out to black for the session (ADR 0012).
        let shown = reveal::Reveal::new(&l.root, true);
        win.set_child(Some(&shown.root));
        shown.show_when(backdrop_ready, || {
            reveal::show_pointer();
            reveal::boot_mark("login window shown");
        });
        let (l2, l4) = (l.clone(), l.clone());
        wire(
            l.panel.clone(),
            move || {
                let user = l2.selected();
                move |pw: String| login::login(login::LOGIN_SOCKET, &user, &pw)
            },
            |_| reveal::exit_faded(),
            move |v| l4.failed(v),
        );
        let keys = gtk::EventControllerKey::new();
        let l3 = l.clone();
        keys.connect_key_pressed(move |_, k, _, _| {
            if k == gtk::gdk::Key::Escape && l3.back.is_mapped() {
                l3.stack.set_visible_child_name("users");
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        win.add_controller(keys);
        kit::snapshot_on_map(&win, "loginwindow");
        win.fullscreen();
        win.present();
    });
    app.run_with_args::<&str>(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_shows_the_full_name() {
        let pw = "root:*:0:0:Charlie &:/root:/bin/sh\nkim:*:1001:1001:Kim Lee,,,:/home/kim:/bin/sh\nbo:*:1002:1002::/home/bo:/bin/sh\n";
        for (user, want) in [("kim", "Kim Lee"), ("bo", "bo"), ("ghost", "ghost")] {
            assert_eq!(login::display_name(user, pw), want);
        }
    }

    #[test]
    fn blur_spreads_and_keeps_flat_areas() {
        let (w, h, c) = (9, 9, 4);
        let mut flat = vec![100u8; w * h * c];
        box_blur(&mut flat, w, h, c, 2);
        assert!(flat.iter().all(|&v| v == 100));
        let mut dot = vec![0u8; w * h * c];
        dot[(4 * w + 4) * c] = 255;
        box_blur(&mut dot, w, h, c, 1);
        let at = |x: usize, y: usize| dot[(y * w + x) * c];
        assert!(at(4, 4) < 255 && at(4, 4) > 0);
        assert_eq!(at(3, 4), at(5, 4));
        assert_eq!(at(4, 3), at(4, 5));
        assert_eq!(at(0, 0), 0);
    }

    #[test]
    fn wallpaper_follows_variant() {
        for (dark, name) in [
            (true, "wonderland-night.svg"),
            (false, "wonderland-day.svg"),
        ] {
            assert!(wallpaper_file(dark).unwrap().ends_with(name));
        }
    }
}
