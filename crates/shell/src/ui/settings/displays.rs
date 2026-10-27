//! Displays: resolution, scale, Night Shift per monitor.
use super::*;

// ---- Displays ----

fn resolutions(m: &crate::system::Monitor) -> Vec<(u32, u32, f64)> {
    let mut v: Vec<(u32, u32, f64)> = Vec::new();
    for mode in &m.modes {
        if let Some((w, h, r)) = wol_core::controls::parse_mode(mode)
            && !v.iter().any(|x| x.0 == w && x.1 == h)
        {
            v.push((w, h, r));
        }
    }
    if v.is_empty() {
        v.push((m.width, m.height, m.refresh));
    }
    v
}

const SCALES: [(&str, f64); 5] = [
    ("100%", 1.0),
    ("125%", 1.25),
    ("150%", 1.5),
    ("175%", 1.75),
    ("200%", 2.0),
];

pub(super) fn page(ctx: &Rc<Ctx>) -> gtk::Box {
    let p = w::page("Displays");
    let mons = ctx.sys.monitors();
    if mons.is_empty() {
        p.append(&w::empty_state(
            "video-display-symbolic",
            "No displays found",
            "Displays appear here while Hyprland is running.",
            None,
        ));
        return p;
    }
    let status = status_label();
    for m in mons {
        let res = resolutions(&m);
        let cur = Rc::new(std::cell::Cell::new((
            res.iter()
                .position(|r| r.0 == m.width && r.1 == m.height)
                .unwrap_or(0),
            SCALES
                .iter()
                .position(|s| (s.1 - m.scale).abs() < 0.01)
                .unwrap_or(0),
        )));
        let apply = Rc::new({
            let (cur, ctx, name, res, status) = (
                cur.clone(),
                ctx.clone(),
                m.name.clone(),
                res.clone(),
                status.clone(),
            );
            move || {
                let (r, s) = cur.get();
                let (w, h, hz) = res[r];
                let Some(rule) = wol_core::controls::monitor_rule(&name, w, h, hz, SCALES[s].1)
                else {
                    return;
                };
                let rel = format!("{}/displays.conf", prefs::HYPR_DIR);
                let text = prefs::displays_conf(read(&config(&rel)).as_deref(), &rule);
                let _ = save_journaled("displays", &rel, Some(&text));
                let st = status.clone();
                ctx.act(
                    Action::Monitor(rule),
                    Rc::new(move |r| {
                        say(&st, r.err().map_or("", |e| e.message()));
                    }),
                );
            }
        });
        let labels: Vec<String> = res.iter().map(|r| format!("{} × {}", r.0, r.1)).collect();
        let lref: Vec<&str> = labels.iter().map(String::as_str).collect();
        let scales: Vec<&str> = SCALES.iter().map(|s| s.0).collect();
        let (c1, a1, c2, a2) = (cur.clone(), apply.clone(), cur.clone(), apply.clone());
        let title = if m.description.is_empty() {
            m.name.clone()
        } else {
            m.description.clone()
        };
        let night_ctx = ctx.clone();
        let night_status = status.clone();
        p.append(&w::section(
            &title,
            &[
                row(
                    "Resolution",
                    &dropdown(&lref, cur.get().0, move |i| {
                        c1.set((i, c1.get().1));
                        a1();
                    }),
                ),
                row(
                    "Scale",
                    &dropdown(&scales, cur.get().1, move |i| {
                        c2.set((c2.get().0, i));
                        a2();
                    }),
                ),
                row_sub(
                    "Night Shift",
                    "Warmer colours after dark",
                    &w::toggle(ctx.sys.night_light().unwrap_or(false), move |on| {
                        let st = night_status.clone();
                        night_ctx.act(
                            Action::NightLight(on),
                            Rc::new(move |r| say(&st, r.err().map_or("", |e| e.message()))),
                        );
                    }),
                ),
            ],
        ));
    }
    p.append(&status);
    p
}
