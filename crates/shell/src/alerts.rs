//! Low battery, full disk and updates-ready notifications, checked with the menu bar poll.
//! Battery levels come from `wol_core::power::policy` (20, 10, 5 %, sleep at 3 %), updates from
//! `wol_core::update::check`; the sentences from [`crate::copy`].
use crate::status::{Battery, BatteryStatus};
use wol_core::power::policy::{self, Alert};
use wol_core::update::check::Updates;

/// Below this share of free space on the startup disk, warn once (until it is back above
/// [`DISK_CLEAR_PERCENT`]).
pub const DISK_WARN_PERCENT: u64 = 5;
pub const DISK_CLEAR_PERCENT: u64 = 10;
/// Menu bar polls between disk checks (5 s each: every 5 minutes; `df` forks).
pub const DISK_EVERY_POLLS: u32 = 60;

/// What one poll asks the session to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// Notification: summary, body, critical (stays until dismissed).
    Show(String, String, bool),
    /// Notify, then lock and sleep now (the battery is about to die).
    Sleep(String, String),
    /// Notify with a Restart action (a driver was installed in the background).
    Restart(String, String),
}

#[derive(Debug, Default)]
pub struct Alerts {
    /// Battery level already warned about while discharging.
    battery: Option<u8>,
    disk_warned: bool,
    polls: u32,
    /// The update check already notified about (its `checked` time).
    updates_shown: u64,
    /// The online install result already shown (its `at` time).
    online_shown: Option<u64>,
}

impl Alerts {
    /// One battery reading: a notice at each level once per discharge.
    pub fn battery(&mut self, b: Option<Battery>) -> Option<Notice> {
        let b = b?;
        let discharging = b.status == BatteryStatus::Discharging;
        let (alert, level) = policy::battery_alert(self.battery, b.percent, discharging);
        self.battery = level;
        Some(match alert? {
            Alert::Low(_) => {
                let (s, body) = crate::copy::battery_low(b.percent);
                Notice::Show(s, body, b.percent <= 5)
            }
            Alert::Sleep => Notice::Sleep(
                crate::copy::BATTERY_SLEEP.0.into(),
                crate::copy::BATTERY_SLEEP.1.into(),
            ),
        })
    }

    /// Free and total bytes of the startup disk: a notice when it gets under 5 % free.
    pub fn disk(&mut self, free: u64, total: u64) -> Option<Notice> {
        if total == 0 {
            return None;
        }
        let pct = free.saturating_mul(100) / total;
        if pct >= DISK_CLEAR_PERCENT {
            self.disk_warned = false;
        }
        if pct >= DISK_WARN_PERCENT || self.disk_warned {
            return None;
        }
        self.disk_warned = true;
        let (s, body) = crate::copy::disk_full(&wol_core::storage::human(free));
        Some(Notice::Show(s, body, false))
    }

    /// The daily update check's findings: a notice once per check that found something.
    pub fn updates(&mut self, u: Option<Updates>) -> Option<Notice> {
        let u = u.filter(|u| u.notify_after(self.updates_shown))?;
        self.updates_shown = u.checked;
        let (s, body) = crate::copy::updates_ready(u.count(), !u.security.is_empty());
        Some(Notice::Show(s, body, false))
    }

    /// A driver installed (or not) when the network came up: one notice per attempt, with
    /// Restart when it worked, a pointer to Settings when it did not.
    pub fn online(&mut self, r: Option<shared::hardware::OnlineInstall>) -> Option<Notice> {
        let r = r?;
        let ((s, body), restart) = wol_core::hardware::online::notice(&r, self.online_shown)?;
        self.online_shown = Some(r.at);
        let (s, body) = (s.to_string(), body.to_string());
        Some(if restart {
            Notice::Restart(s, body)
        } else {
            Notice::Show(s, body, false)
        })
    }

    /// Disk is checked on every [`DISK_EVERY_POLLS`]th poll, starting with the first.
    pub fn disk_due(&mut self) -> bool {
        let due = self.polls.is_multiple_of(DISK_EVERY_POLLS);
        self.polls = self.polls.wrapping_add(1);
        due
    }
}

/// Startup disk (`/`, or `/home` when it is fuller) free and total bytes from `df`.
fn startup_disk() -> Option<(u64, u64)> {
    let disks = wol_core::system::stats::DiskUsage::read(&wol_core::system::PosixHost);
    disks
        .iter()
        .filter(|d| d.mount == "/" || d.mount == "/home")
        .map(|d| {
            (
                d.available.max(0) as u64,
                (d.used + d.available).max(0) as u64,
            )
        })
        .filter(|(_, t)| *t > 0)
        .min_by_key(|(f, t)| f.saturating_mul(100) / t)
}

thread_local! {
    static STATE: std::cell::RefCell<Alerts> = std::cell::RefCell::new(Alerts::default());
}

/// Menu bar poll hook: battery every time, disk and updates every few minutes.
pub fn poll(b: Option<Battery>) {
    let (battery, due) = STATE.with_borrow_mut(|a| (a.battery(b), a.disk_due()));
    let (disk, updates) = if due {
        let u = Updates::read(std::path::Path::new(wol_core::update::check::DIR));
        let online = std::fs::read_to_string(wol_core::hardware::online::ONLINE_PATH)
            .ok()
            .and_then(|t| wol_core::hardware::online::parse(&t));
        let driver = STATE.with_borrow_mut(|a| a.online(online));
        if let Some(n) = driver {
            act(n);
        }
        let disk = startup_disk();
        STATE.with_borrow_mut(|a| (disk.and_then(|(f, t)| a.disk(f, t)), a.updates(u)))
    } else {
        (None, None)
    };
    for n in battery.into_iter().chain(disk).chain(updates) {
        act(n);
    }
}

fn act(n: Notice) {
    let notify = |s: &str, body: &str, critical: bool| {
        let mut argv = vec!["notify-send".to_string(), "-a".into(), "WolOS".into()];
        if critical {
            argv.extend(["-u".into(), "critical".into()]);
        }
        argv.extend([s.to_string(), body.to_string()]);
        crate::kit::spawn(&argv);
    };
    match n {
        Notice::Show(s, body, critical) => notify(&s, &body, critical),
        Notice::Restart(s, body) => {
            // The action button asks first like every Restart (`wol power restart`).
            let script = format!(
                "[ \"$(notify-send -a WolOS -w -A restart={} \"$1\" \"$2\")\" = restart ] && exec wol power restart",
                wol_core::hardware::online::RESTART
            );
            crate::kit::spawn(&["sh".into(), "-c".into(), script, "sh".into(), s, body]);
        }
        Notice::Sleep(s, body) => {
            notify(&s, &body, true);
            crate::kit::spawn(&["wol".into(), "power".into(), "sleep".into()]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bat(percent: u8, status: BatteryStatus) -> Option<Battery> {
        Some(Battery { percent, status })
    }

    #[test]
    fn battery_notices_once_per_level_then_sleep() {
        use BatteryStatus::*;
        let mut a = Alerts::default();
        let mut seen = Vec::new();
        for (p, s) in [
            (25, Discharging),
            (20, Discharging),
            (19, Discharging),
            (10, Discharging),
            (9, Discharging),
            (5, Discharging),
            (3, Discharging),
            (3, Discharging),
            (4, Charging),
            (20, Discharging),
        ] {
            seen.push(match a.battery(bat(p, s)) {
                Some(Notice::Show(_, body, critical)) => format!("{p}:show:{critical}:{body}"),
                Some(Notice::Sleep(..)) => format!("{p}:sleep"),
                Some(Notice::Restart(..)) => unreachable!(),
                None => format!("{p}:-"),
            });
        }
        assert_eq!(
            seen,
            [
                "25:-",
                "20:show:false:Your battery has 20% left. Connect to power, or turn on Low Power Mode.",
                "19:-",
                "10:show:false:Your battery has 10% left. Connect to power, or turn on Low Power Mode.",
                "9:-",
                "5:show:true:Your battery has 5% left. Connect to power, or turn on Low Power Mode.",
                "3:sleep",
                "3:-",
                "4:-",
                "20:show:false:Your battery has 20% left. Connect to power, or turn on Low Power Mode.",
            ]
        );
        assert_eq!(a.battery(None), None, "desktops have no battery");
    }

    #[test]
    fn full_disk_warns_once_until_space_is_back() {
        let mut a = Alerts::default();
        const G: u64 = 1_000_000_000;
        assert_eq!(a.disk(20 * G, 100 * G), None);
        let Some(Notice::Show(s, body, false)) = a.disk(4 * G, 100 * G) else {
            panic!()
        };
        assert_eq!(s, crate::copy::disk_full("4 GB").0);
        assert!(body.contains("4 GB"), "{body}");
        assert_eq!(a.disk(3 * G, 100 * G), None, "once");
        assert_eq!(a.disk(7 * G, 100 * G), None, "still low: no repeat");
        assert_eq!(a.disk(12 * G, 100 * G), None);
        assert!(a.disk(2 * G, 100 * G).is_some(), "full again: warn again");
        assert_eq!(a.disk(0, 0), None);
        let due: Vec<bool> = (0..=DISK_EVERY_POLLS).map(|_| a.disk_due()).collect();
        assert!(due[0] && due[DISK_EVERY_POLLS as usize] && !due[1]);
    }

    #[test]
    fn a_background_driver_install_notifies_once() {
        use shared::hardware::OnlineInstall;
        let r = |ok, graphics| OnlineInstall {
            at: 7,
            packages: vec!["nvidia-drm-kmod".into()],
            ok,
            graphics,
        };
        let mut a = Alerts::default();
        assert_eq!(a.online(None), None);
        let Some(Notice::Restart(s, body)) = a.online(Some(r(true, true))) else {
            panic!()
        };
        assert_eq!(s, "Graphics driver installed");
        assert!(body.starts_with("Restart"), "{body}");
        assert_eq!(a.online(Some(r(true, true))), None, "once");
        let mut b = Alerts::default();
        let Some(Notice::Show(_, body, false)) = b.online(Some(r(false, true))) else {
            panic!()
        };
        assert!(body.contains("Settings"), "{body}");
    }

    #[test]
    fn updates_notify_once_per_check() {
        let mut a = Alerts::default();
        let found = |t: &str, audit: &str| {
            Updates::parse(
                t,
                "curl-8.1 <   needs updating\nfirefox-1 <   needs updating\n",
                audit,
                "",
            )
        };
        assert_eq!(a.updates(None), None, "never checked");
        let Some(Notice::Show(s, body, false)) = a.updates(found("100", "")) else {
            panic!()
        };
        assert_eq!(s, "Updates Available");
        assert_eq!(
            body,
            "WolOS has 2 updates ready. Open Software Update to install them."
        );
        assert_eq!(a.updates(found("100", "")), None, "same check: once");
        let Some(Notice::Show(_, body, _)) = a.updates(found("200", "curl-8.1")) else {
            panic!()
        };
        assert!(body.contains("including security fixes"), "{body}");
        assert_eq!(
            a.updates(Updates::parse("300", "", "", "")),
            None,
            "up to date"
        );
    }

    #[test]
    fn alert_sentences_pass_the_copy_lint() {
        // Bodies are sentences; summaries are titles.
        let (_, b) = crate::copy::battery_low(10);
        let (_, db) = crate::copy::disk_full("2 GB");
        let (_, u1) = crate::copy::updates_ready(1, false);
        let (_, u2) = crate::copy::updates_ready(4, true);
        for t in [b.as_str(), crate::copy::BATTERY_SLEEP.1, &db, &u1, &u2] {
            assert!(
                crate::copy::lint(t).is_empty(),
                "{t}: {:?}",
                crate::copy::lint(t)
            );
        }
    }
}
