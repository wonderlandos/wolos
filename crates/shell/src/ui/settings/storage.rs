//! General > Storage: how full the disk is and what uses it (ZFS datasets grouped), encryption and
//! restore points (`wol_core::storage`).
use super::*;
use wol_core::storage::{Storage, human};

/// (name, size, share of the whole disk) per category, largest first.
pub(crate) fn parts(s: &Storage) -> Vec<(String, String, f64)> {
    let total = s.total.max(1) as f64;
    s.categories
        .iter()
        .filter(|c| c.bytes > 0)
        .map(|c| (c.name.clone(), human(c.bytes), c.bytes as f64 / total))
        .collect()
}

fn render(into: &gtk::Box, s: Option<Storage>) {
    let Some(s) = s else {
        into.append(&w::empty_state(
            "drive-harddisk-symbolic",
            "Storage Unavailable",
            "The startup disk could not be read.",
            None,
        ));
        return;
    };
    let free = s.total.saturating_sub(s.used);
    let head = w::vbox(w::S2);
    head.add_css_class("list-row");
    head.add_css_class("usage-row");
    let line = w::hbox(w::S2);
    let name = w::label(if s.zfs { "Startup disk" } else { &s.name }, "title");
    name.set_hexpand(true);
    line.append(&name);
    line.append(&w::label(
        &format!("{} of {} used", human(s.used), human(s.total)),
        "secondary",
    ));
    head.append(&line);
    let mut p = parts(&s);
    if p.is_empty() && s.used > 0 {
        p.push((
            "Used".into(),
            human(s.used),
            s.used as f64 / s.total.max(1) as f64,
        ));
    }
    head.append(&kit::pickers::usage_bar(&p));
    into.append(&w::section("", &[head.upcast()]));
    let mut rows = vec![info("Available", &human(free))];
    if let Some(enc) = s.encrypted {
        rows.push(info("Encryption", if enc { "On" } else { "Off" }));
    }
    if s.zfs {
        rows.push(info(
            "Restore points",
            &match s.boot_environments {
                0 => "None yet".to_string(),
                1 => "1 restore point".to_string(),
                n => format!("{n} restore points"),
            },
        ));
        if s.snapshots > 0 {
            rows.push(info("Kept for restore points", &human(s.snapshots)));
        }
    }
    into.append(&w::section("", &rows));
    if s.zfs {
        into.append(&w::footnote(
            "Restore points keep earlier versions of the system so an update can be undone. The oldest go first when space runs low.",
        ));
    }
}

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let p = general::subpage(e, "Storage");
    let body = w::vbox(w::S6);
    p.append(&body);
    let m = e.machine();
    later(&body, move || m.storage(), render);
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use wol_core::storage::Category;

    #[test]
    fn parts_are_shares_of_the_whole_disk() {
        let s = Storage {
            total: 1000,
            used: 600,
            categories: vec![
                Category {
                    name: "Apps".into(),
                    bytes: 500,
                },
                Category {
                    name: "Empty".into(),
                    bytes: 0,
                },
                Category {
                    name: "System".into(),
                    bytes: 100,
                },
            ],
            ..Default::default()
        };
        let p = parts(&s);
        assert_eq!(p.len(), 2, "empty categories are left out");
        assert_eq!((p[0].0.as_str(), p[0].2), ("Apps", 0.5));
        assert_eq!(p[1].2, 0.1);
    }
}
