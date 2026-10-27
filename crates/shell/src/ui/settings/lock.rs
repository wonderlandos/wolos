//! Lock Screen: when the screen locks (hypridle timeouts, journaled so the change can be undone).
use super::*;

pub(super) fn page(_: &Rc<Env>) -> gtk::Box {
    let p = w::page("Lock Screen");
    let file = config(prefs::IDLE_FILE);
    let (secs, sleep) = prefs::parse_hypridle(read(&file).as_deref());
    let cur = Rc::new(std::cell::Cell::new((secs, sleep)));
    let save = Rc::new({
        let cur = cur.clone();
        move || {
            let (s, z) = cur.get();
            let _ = save_journaled("idle", prefs::IDLE_FILE, Some(&prefs::hypridle_conf(s, z)));
            // hypridle reads its config at start: restart it on the new file.
            background(vec![argv(&["pkill", "-x", "hypridle"])]);
            glib::timeout_add_local_once(std::time::Duration::from_millis(300), || {
                kit::spawn(&argv(&["hypridle"]))
            });
        }
    });
    let labels: Vec<&str> = prefs::IDLE_CHOICES.iter().map(|c| c.0).collect();
    let sel = prefs::IDLE_CHOICES
        .iter()
        .position(|c| c.1 == secs)
        .unwrap_or(2);
    let (c1, s1, c2, s2) = (cur.clone(), save.clone(), cur.clone(), save.clone());
    p.append(&w::section(
        "",
        &[
            row(
                "Lock screen when inactive",
                &dropdown(&labels, sel, move |i| {
                    c1.set((prefs::IDLE_CHOICES[i].1, c1.get().1));
                    s1();
                }),
            ),
            row(
                "Lock when the computer sleeps",
                &w::toggle(sleep, move |on| {
                    c2.set((c2.get().0, on));
                    s2();
                }),
            ),
        ],
    ));
    p.append(&w::footnote(
        "The display turns off one minute after the screen locks.",
    ));
    p
}
