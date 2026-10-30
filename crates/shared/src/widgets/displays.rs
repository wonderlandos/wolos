//! Which display a desktop widget lives on. `widgets.json` stays one file: each placed widget
//! may name its display (`display`), matched by the display's description (stable across ports)
//! or its connector. A widget without one, or whose display is not plugged in, shows on the main
//! display; it keeps its own display name, so it goes back when that display returns. The first
//! save after the per-display layout arrived writes the main display's name on unnamed widgets.
use super::{Desktop, Placed};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Display {
    /// "HDMI-A-1".
    pub connector: String,
    /// "Dell Inc. DELL U2720Q 3H8K1R2"; empty when the compositor gives none.
    pub description: String,
}

impl Display {
    /// What a widget saves to name this display.
    pub fn key(&self) -> String {
        if self.description.is_empty() {
            self.connector.clone()
        } else {
            self.description.clone()
        }
    }
    pub fn is(&self, key: &str) -> bool {
        key == self.description || key == self.connector
    }
}

/// Index into `displays` of the display `p` shows on (0, the main display, when its own is
/// missing or unnamed).
pub fn home(p: &Placed, displays: &[Display]) -> usize {
    p.display
        .as_deref()
        .and_then(|k| displays.iter().position(|d| d.is(k)))
        .unwrap_or(0)
}

/// The part of `desk` display `i` shows. Grants are shared; the main display also carries the
/// entries and keys this version cannot read, so they are written back once.
pub fn subset(desk: &Desktop, displays: &[Display], i: usize) -> Desktop {
    let main_key = displays.first().map(Display::key);
    Desktop {
        widgets: desk
            .widgets
            .iter()
            .filter(|p| home(p, displays) == i)
            .cloned()
            .map(|mut p| {
                if p.display.is_none() {
                    p.display = main_key.clone();
                }
                p
            })
            .collect(),
        grants: desk.grants.clone(),
        unreadable: if i == 0 {
            desk.unreadable.clone()
        } else {
            vec![]
        },
        extra: if i == 0 {
            desk.extra.clone()
        } else {
            Default::default()
        },
    }
}

/// Folds one display's layout back into the whole: the widgets it held before (`owned`) are
/// replaced by what it holds now, grants add up. Returns what it owns now.
pub fn merge(master: &mut Desktop, owned: &BTreeSet<String>, part: &Desktop) -> BTreeSet<String> {
    let now: BTreeSet<String> = part.widgets.iter().map(|p| p.id.clone()).collect();
    master
        .widgets
        .retain(|p| !owned.contains(&p.id) && !now.contains(&p.id));
    master.widgets.extend(part.widgets.iter().cloned());
    for g in &part.grants {
        if !master.grants.contains(g) {
            master.grants.push(g.clone());
        }
    }
    if !part.extra.is_empty() {
        master.extra = part.extra.clone();
    }
    if !part.unreadable.is_empty() {
        master.unreadable = part.unreadable.clone();
    }
    now
}

#[cfg(test)]
mod tests {
    use super::super::{Builtin, Grant, Kind, Size};
    use super::*;

    fn d(connector: &str, description: &str) -> Display {
        Display {
            connector: connector.into(),
            description: description.into(),
        }
    }

    fn w(id: &str, display: Option<&str>) -> Placed {
        Placed {
            id: id.into(),
            kind: Kind::Builtin(Builtin::Clock),
            size: Size::Small,
            col: 0,
            row: 0,
            options: Default::default(),
            display: display.map(String::from),
        }
    }

    fn ids(desk: &Desktop) -> Vec<&str> {
        desk.widgets.iter().map(|p| p.id.as_str()).collect()
    }

    const DELL: &str = "Dell Inc. DELL U2720Q 3H8K1R2";

    #[test]
    fn widgets_find_their_display_by_description_or_connector() {
        let ds = [d("eDP-1", "BOE 0x0BCA"), d("HDMI-A-1", DELL)];
        assert_eq!(home(&w("a", Some(DELL)), &ds), 1);
        assert_eq!(
            home(&w("a", Some("HDMI-A-1")), &ds),
            1,
            "connector still matches"
        );
        assert_eq!(home(&w("a", None), &ds), 0, "unnamed: main display");
        assert_eq!(
            home(&w("a", Some("LG 27GL850")), &ds),
            0,
            "unplugged: main for now"
        );
        assert_eq!(d("DP-2", "").key(), "DP-2");
        assert_eq!(ds[1].key(), DELL);
    }

    #[test]
    fn the_single_grid_moves_to_the_main_display() {
        let desk = Desktop {
            widgets: vec![w("clock", None), w("tv", Some(DELL)), w("away", Some("LG"))],
            grants: vec![Grant {
                widget: "x".into(),
                host: "h".into(),
            }],
            ..Default::default()
        };
        let ds = [d("eDP-1", "BOE 0x0BCA"), d("HDMI-A-1", DELL)];
        let main = subset(&desk, &ds, 0);
        assert_eq!(ids(&main), ["clock", "away"]);
        assert_eq!(
            main.widgets[0].display.as_deref(),
            Some("BOE 0x0BCA"),
            "migrated"
        );
        assert_eq!(
            main.widgets[1].display.as_deref(),
            Some("LG"),
            "keeps its own display"
        );
        let tv = subset(&desk, &ds, 1);
        assert_eq!(ids(&tv), ["tv"]);
        assert_eq!(tv.grants, desk.grants);
    }

    #[test]
    fn saving_one_display_keeps_the_others() {
        let mut master = Desktop {
            widgets: vec![
                w("a", Some("eDP-1")),
                w("b", Some(DELL)),
                w("c", Some(DELL)),
            ],
            ..Default::default()
        };
        let ds = [d("eDP-1", ""), d("HDMI-A-1", DELL)];
        let mut part = subset(&master, &ds, 1);
        let owned: BTreeSet<String> = ids(&part).into_iter().map(String::from).collect();
        // On the Dell: "b" removed, "n" added.
        part.widgets.retain(|p| p.id != "b");
        part.widgets.push(w("n", Some(DELL)));
        part.grants.push(Grant {
            widget: "n".into(),
            host: "api".into(),
        });
        let now = merge(&mut master, &owned, &part);
        assert_eq!(ids(&master), ["a", "c", "n"]);
        assert_eq!(master.grants.len(), 1);
        assert_eq!(now.into_iter().collect::<Vec<_>>(), ["c", "n"]);
    }
}
