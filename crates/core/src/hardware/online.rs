//! A driver only the internet has (NVIDIA: not in the offline repo) that could not be installed at
//! first boot. `wol hw online` installs it once per boot, the first time the network comes up
//! (`/etc/dhclient-exit-hooks`), and leaves the result in [`ONLINE_PATH`]; the session turns a new
//! result into one notification. Bodies follow HIG §3 and pass `shell::copy::lint`.
pub use shared::hardware::ONLINE_PATH;
use shared::hardware::{HardwareSnapshot, Kind, OnlineInstall};

/// (title, body) of one notification.
pub type Notice = (&'static str, &'static str);

pub const GRAPHICS_READY: Notice = (
    "Graphics driver installed",
    "Restart to finish setting up your graphics card.",
);
pub const DRIVER_READY: Notice = (
    "Driver installed",
    "Restart to finish setting up your hardware.",
);
pub const GRAPHICS_LATER: Notice = (
    "Graphics driver not installed yet",
    "WolOS tries again the next time you're online. To install it now, open System Settings › Hardware.",
);
pub const DRIVER_LATER: Notice = (
    "Driver not installed yet",
    "WolOS tries again the next time you're online. To install it now, open System Settings › Hardware.",
);
/// The action a ready notice offers.
pub const RESTART: &str = "Restart";

pub const ALL: [Notice; 4] = [GRAPHICS_READY, DRIVER_READY, GRAPHICS_LATER, DRIVER_LATER];

/// What to install now: the packages the catalogue lists as missing, and whether a GPU asked for
/// them. None when nothing is pending.
pub fn pending(snap: &HardwareSnapshot) -> Option<(Vec<String>, bool)> {
    let (_, packages) = super::fixes(snap);
    let graphics = snap
        .devices
        .iter()
        .any(|d| d.kind == Kind::Gpu && !d.missing_packages.is_empty());
    (!packages.is_empty()).then_some((packages, graphics))
}

/// The notification for a result the session has not shown yet (`shown`: the last `at` it
/// showed), and whether it offers [`RESTART`].
pub fn notice(r: &OnlineInstall, shown: Option<u64>) -> Option<(Notice, bool)> {
    if shown == Some(r.at) || r.packages.is_empty() {
        return None;
    }
    Some(match (r.ok, r.graphics) {
        (true, true) => (GRAPHICS_READY, true),
        (true, false) => (DRIVER_READY, true),
        (false, true) => (GRAPHICS_LATER, false),
        (false, false) => (DRIVER_LATER, false),
    })
}

/// [`ONLINE_PATH`] text -> result (None: no attempt this boot, or unreadable).
pub fn parse(text: &str) -> Option<OnlineInstall> {
    serde_json::from_str(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::hardware::{Device, State};

    fn dev(kind: Kind, packages: &[&str]) -> Device {
        Device {
            kind,
            state: if packages.is_empty() {
                State::Ready
            } else {
                State::NeedsDriver
            },
            missing_packages: packages.iter().map(|p| (*p).to_owned()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn pending_is_the_missing_packages_and_whether_a_gpu_needs_them() {
        let snap = |d: Vec<Device>| HardwareSnapshot { devices: d };
        assert_eq!(
            pending(&snap(vec![
                dev(Kind::Wifi, &[]),
                dev(Kind::Gpu, &["nvidia-drm-kmod"])
            ])),
            Some((vec!["nvidia-drm-kmod".to_owned()], true))
        );
        assert_eq!(
            pending(&snap(vec![dev(Kind::Ethernet, &["realtek-re-kmod"])])),
            Some((vec!["realtek-re-kmod".to_owned()], false))
        );
        assert_eq!(pending(&snap(vec![dev(Kind::Gpu, &[])])), None);
        assert_eq!(pending(&HardwareSnapshot::default()), None);
    }

    #[test]
    fn one_notice_per_attempt_restart_only_after_an_install() {
        let r = |ok, graphics| OnlineInstall {
            at: 7,
            packages: vec!["nvidia-drm-kmod".into()],
            ok,
            graphics,
        };
        assert_eq!(notice(&r(true, true), None), Some((GRAPHICS_READY, true)));
        assert_eq!(notice(&r(true, false), Some(3)), Some((DRIVER_READY, true)));
        assert_eq!(notice(&r(false, true), None), Some((GRAPHICS_LATER, false)));
        assert_eq!(notice(&r(false, false), None), Some((DRIVER_LATER, false)));
        assert_eq!(notice(&r(true, true), Some(7)), None, "already shown");
        let nothing = OnlineInstall {
            packages: vec![],
            ..r(true, true)
        };
        assert_eq!(notice(&nothing, None), None);
    }

    #[test]
    fn result_file_round_trips() {
        let r = OnlineInstall {
            at: 1_793_000_000,
            packages: vec!["nvidia-drm-kmod".into()],
            ok: true,
            graphics: true,
        };
        let text = serde_json::to_string(&r).unwrap();
        assert_eq!(parse(&text), Some(r));
        assert_eq!(parse(""), None);
        assert_eq!(ONLINE_PATH, "/var/run/wolos-hw-online.json");
    }

    /// Plain sentences: the shell lint (crates/shell/tests/copy_lint.rs) checks them too.
    #[test]
    fn copy_is_calm_and_points_at_settings() {
        for (title, body) in ALL {
            assert!(title.len() <= 40 && !title.ends_with('.'), "{title}");
            assert!(body.ends_with('.') && !body.contains("fail"), "{body}");
        }
        for (_, body) in [GRAPHICS_LATER, DRIVER_LATER] {
            assert!(body.contains("System Settings › Hardware"), "{body}");
        }
    }
}
