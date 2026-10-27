//! App switcher (Super+Tab): Hyprland clients grouped by app class, most recently used first.
use crate::hypr::Client;

/// One app in the strip; `address` is its most recently focused window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    pub class: String,
    pub name: String,
    pub icon: String,
    pub address: String,
}

/// Groups clients by class (case-insensitive), ordered by each app's most recent focus.
/// Names and icons come from the dock pins, then installed .desktop entries.
pub fn apps(clients: &[Client], installed: &[crate::desktop::Entry]) -> Vec<App> {
    use crate::apps::{PINS, frontmost_name, icon_for};
    let mut sorted: Vec<&Client> = clients.iter().filter(|c| !c.class.is_empty()).collect();
    sorted.sort_by_key(|c| c.focus);
    let mut out: Vec<App> = Vec::new();
    for c in sorted {
        let class = c.class.to_lowercase();
        if out.iter().all(|a| a.class != class) {
            out.push(App {
                name: frontmost_name(&class, "", PINS, installed),
                icon: icon_for(&class, PINS, installed),
                class,
                address: c.address.clone(),
            });
        }
    }
    out
}

/// Apps for the strip from a raw `j/clients` reply. Fails closed: None (switcher stays hidden)
/// when the compositor did not answer, the reply is unreadable, or no app window is open.
pub fn to_show(reply: Option<&str>, installed: &[crate::desktop::Entry]) -> Option<Vec<App>> {
    let a = apps(&crate::hypr::parse_clients(reply?), installed);
    (!a.is_empty()).then_some(a)
}

/// Selection while Super is held. Opening selects the previous app (index 1), like macOS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cycle {
    pub index: usize,
    pub len: usize,
}

impl Cycle {
    pub fn open(len: usize) -> Self {
        Cycle {
            index: usize::from(len > 1),
            len,
        }
    }
    pub fn next(self) -> Self {
        self.step(1)
    }
    pub fn prev(self) -> Self {
        self.step(self.len.saturating_sub(1))
    }
    fn step(self, by: usize) -> Self {
        Cycle {
            index: if self.len == 0 {
                0
            } else {
                (self.index + by) % self.len
            },
            ..self
        }
    }
}

/// Committing a selection, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Hide,
    /// Hyprland `focuswindow` dispatch argument.
    Focus(String),
}

/// Hide first: Hyprland hands focus back to the previous window when a keyboard-exclusive
/// layer unmaps, which would undo a focus sent before the hide.
pub fn commit(selected: Option<&App>) -> Vec<Step> {
    let mut v = vec![Step::Hide];
    if let Some(a) = selected {
        v.push(Step::Focus(format!("address:{}", a.address)));
    }
    v
}

/// What a shell-socket command does to the switcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Open,
    Next,
    Prev,
    /// Super released: focus the selection and close.
    Commit,
    None,
}

/// `show|toggle switcher` opens or advances, `show switcher-prev` goes back, `hide switcher` commits.
pub fn op(verb: crate::protocol::Verb, target: &str, visible: bool) -> Op {
    use crate::protocol::Verb::*;
    match (verb, target, visible) {
        (Show | Toggle, "switcher", false) => Op::Open,
        (Show | Toggle, "switcher", true) => Op::Next,
        (Show | Toggle, "switcher-prev", true) => Op::Prev,
        (Show | Toggle, "switcher-prev", false) => Op::Open,
        (Hide, "switcher" | "switcher-prev", true) => Op::Commit,
        _ => Op::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(addr: &str, class: &str, focus: i64) -> Client {
        Client {
            focus,
            ..Client::new(addr, class, "")
        }
    }

    #[test]
    fn groups_by_class_in_mru_order() {
        let got = apps(
            &[
                c("0x1", "foot", 2),
                c("0x2", "firefox", 0),
                c("0x3", "Foot", 1),
                c("0x4", "org.gnome.Nautilus", 3),
            ],
            &[],
        );
        let v: Vec<(&str, &str)> = got
            .iter()
            .map(|a| (a.class.as_str(), a.address.as_str()))
            .collect();
        assert_eq!(
            v,
            [
                ("firefox", "0x2"),
                ("foot", "0x3"),
                ("org.gnome.nautilus", "0x4")
            ]
        );
        assert!(apps(&[], &[]).is_empty());
        let names: Vec<&str> = got.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Browser", "Terminal", "Files"]);
    }

    #[test]
    fn opens_only_on_a_readable_nonempty_client_list() {
        let one = r#"[{"address":"0x1","class":"foot","title":"~","focusHistoryID":0}]"#;
        for (reply, opens) in [
            (None, false),             // no compositor answer (not Hyprland, socket down)
            (Some("not json"), false), // unreadable
            (Some("[]"), false),       // nothing open: no "No open windows" pill
            (Some(one), true),
        ] {
            assert_eq!(to_show(reply, &[]).is_some(), opens, "{reply:?}");
        }
    }

    #[test]
    fn cycle_wraps_both_ways() {
        for (len, open, next, prev) in [(0, 0, 0, 0), (1, 0, 0, 0), (2, 1, 0, 0), (3, 1, 2, 0)] {
            let o = Cycle::open(len);
            assert_eq!(
                (o.index, o.next().index, o.prev().index),
                (open, next, prev),
                "len {len}"
            );
        }
        assert_eq!(Cycle::open(3).prev().prev().index, 2);
    }

    /// Live Hyprland 0.56: releasing Super left the old window focused.
    #[test]
    fn commit_hides_before_focusing() {
        let a = App {
            class: "foot".into(),
            name: "Terminal".into(),
            icon: "wolos-terminal".into(),
            address: "0x1".into(),
        };
        assert_eq!(
            commit(Some(&a)),
            [Step::Hide, Step::Focus("address:0x1".into())]
        );
        assert_eq!(commit(None), [Step::Hide]);
    }

    #[test]
    fn socket_ops() {
        use crate::protocol::Verb::*;
        for (verb, target, visible, want) in [
            (Show, "switcher", false, Op::Open),
            (Toggle, "switcher", false, Op::Open),
            (Show, "switcher", true, Op::Next),
            (Toggle, "switcher", true, Op::Next),
            (Show, "switcher-prev", true, Op::Prev),
            (Show, "switcher-prev", false, Op::Open),
            (Hide, "switcher", true, Op::Commit),
            (Hide, "switcher", false, Op::None),
            (Reload, "", true, Op::None),
            (Show, "osd", true, Op::None),
        ] {
            assert_eq!(
                op(verb, target, visible),
                want,
                "{verb:?} {target} {visible}"
            );
        }
    }
}
