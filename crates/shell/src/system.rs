//! System data seam. Topics match IaraOS system-api.md: power, audio, network, brightness.
//! [`CoreSystem`] is the real backend (`wol_core::controls`, Linux and FreeBSD): a worker thread keeps cached
//! snapshots, reads never fork or block, actions run off the UI thread through [`System::spawn`].
//! `Sysfs` reads files only; `Fake` is deterministic for goldens.
use crate::status::{Audio, Battery, BatteryStatus, Network};
use std::fs;
pub use wol_core::controls::{BtDevice, Monitor, Output};

/// One change the user asked for. Root ones go through wol-admind and may answer `NeedsAuth`.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Wifi(bool),
    WifiJoin(String, Option<String>),
    Bluetooth(bool),
    Brightness(u8),
    Volume(u8),
    Muted(bool),
    Output(String),
    NightLight(bool),
    MediaToggle,
    Power(PowerMode),
    /// Hyprland monitor rule (`wol_core::controls::monitor_rule`).
    Monitor(String),
    UserAdd {
        name: String,
        full_name: String,
        admin: bool,
        /// New account password: reaches wol-admind on the socket, then `pw` on stdin.
        password: String,
    },
    PkgUpgrade,
    /// App Store: catalog id or package name (`wol_core::store`).
    PkgInstall(String),
    MediaNext,
    MediaPrevious,
    /// Bluetooth device address, connect (true) or disconnect.
    BtConnect(String, bool),
    KeyboardBrightness(u8),
    ReduceMotion(bool),
    Zoom(bool),
    /// Kit stylesheet contrast overrides (`kit::style::CONTRAST_FILE`), every shell process.
    IncreaseContrast(bool),
    /// Settings: save desktop prefs and apply the changed Hyprland keywords live (`wol_core::hyprprefs`).
    Desktop(Box<wol_core::hyprprefs::DesktopPrefs>),
    /// Settings > Keyboard > Shortcuts: save overrides and rebind live.
    Shortcuts(Box<wol_core::shortcuts::Overrides>),
    /// Pause (true) or resume WolOS shortcuts while a shortcut is being recorded.
    ShortcutCapture(bool),
    /// Settings > Restore: start from this restore point (ZFS boot environment) next time.
    BootEnvironment(String),
    /// Settings: one allow-listed administrator request (wol-admind validates it again).
    Admin(shared::admin::AdminRequest),
}

pub type Done = async_channel::Receiver<Result<(), ActionError>>;

pub trait System: Send + Sync {
    fn battery(&self) -> Option<Battery>;
    fn audio(&self) -> Audio;
    fn network(&self) -> Network;
    /// 0..=100, None without a backlight.
    fn brightness(&self) -> Option<u8>;

    // Control Center topics. Defaults = unavailable; `core` implements them through the System API / admin actions.
    fn wifi_networks(&self) -> Vec<WifiNetwork> {
        Vec::new()
    }
    /// Admin action: may answer `NeedsAuth`; the UI then shows the Authenticate sheet and calls `authorize`.
    fn wifi_join(&self, _ssid: &str, _password: Option<&str>) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn set_wifi(&self, _on: bool) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn bluetooth(&self) -> Option<bool> {
        None
    }
    fn set_bluetooth(&self, _on: bool) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn set_brightness(&self, _percent: u8) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn set_volume(&self, _percent: u8) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn set_muted(&self, _muted: bool) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn outputs(&self) -> Vec<String> {
        Vec::new()
    }
    fn now_playing(&self) -> Option<NowPlaying> {
        None
    }
    /// App icon of the player behind [`System::now_playing`] (cover art placeholder).
    fn now_playing_icon(&self) -> Option<String> {
        None
    }
    fn media_toggle(&self) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn night_light(&self) -> Option<bool> {
        None
    }
    fn set_night_light(&self, _on: bool) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    /// Power source, mode and what this machine can steer (Settings > Battery, Control Center):
    /// `wol_core::power::policy::status`. None where nothing is read (previews).
    fn power_status(&self) -> Option<wol_core::power::policy::Status> {
        None
    }
    /// Firewall, remote login, system log, kernel hardening (Privacy & Security):
    /// `wol_core::security::status`. How each app permission is enforced is
    /// `wol_core::security::enforcement`.
    fn security_status(&self) -> Option<wol_core::security::Status> {
        None
    }
    fn power_mode(&self) -> Option<PowerMode> {
        None
    }
    fn set_power_mode(&self, _m: PowerMode) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    /// Admin authorization (IaraOS ADR 0026): password for the pending admin action.
    fn authorize(&self, _password: &str) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }

    /// Wi-Fi radio state (None = no Wi-Fi hardware/tooling).
    fn wifi_enabled(&self) -> Option<bool> {
        self.network().wifi.then_some(true)
    }
    fn output_devices(&self) -> Vec<Output> {
        self.outputs()
            .into_iter()
            .enumerate()
            .map(|(i, n)| Output {
                id: n.clone(),
                name: n,
                default: i == 0,
            })
            .collect()
    }
    fn monitors(&self) -> Vec<Monitor> {
        Vec::new()
    }
    /// Minutes to empty (discharging) or full (charging); None when unknown.
    fn battery_time(&self) -> Option<u32> {
        None
    }
    /// Paired Bluetooth devices.
    fn bt_devices(&self) -> Vec<BtDevice> {
        Vec::new()
    }
    /// 0..=100, None without a keyboard backlight.
    fn keyboard_brightness(&self) -> Option<u8> {
        None
    }
    fn accessibility(&self) -> Accessibility {
        Accessibility::default()
    }

    // Settings > Assistants & Automation, Restore (archicture/modules/automation.md).
    /// Helper policy and the helpers seen in the audit log.
    fn helpers(&self) -> wol_core::automation::Status {
        wol_core::automation::Status::default()
    }
    /// The off switch for every helper.
    fn set_helpers_enabled(&self, _on: bool) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    /// Allow or deny one capability (`wol_core::automation::CAPABILITIES` id).
    fn set_helper_capability(&self, _id: &str, _allowed: bool) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    /// Always Allow on the approval sheet: `helper` may change `cap` without asking (journaled).
    fn allow_helper_always(&self, _helper: &str, _cap: &str) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    /// Settings changes, newest first.
    fn history(&self) -> Vec<wol_core::journal::Entry> {
        Vec::new()
    }
    /// Undoes one change (None = the latest) and reloads what it touched; Err = why not.
    fn undo(&self, _id: Option<u64>) -> Result<wol_core::journal::Entry, String> {
        Err("Undo is not available here.".into())
    }
    /// Restore points (ZFS boot environments), empty without them.
    fn boot_environments(&self) -> Vec<wol_core::update::be::BootEnv> {
        Vec::new()
    }
    /// A helper's administrator request, run after the user typed `password` on the approval
    /// sheet (`ui::approval`). Always checks the password, never a cached authorization.
    fn approve(&self, _req: shared::shell::ApprovalRequest, _password: String) -> Done {
        let (tx, rx) = async_channel::bounded(1);
        let _ = tx.try_send(Err(ActionError::Unavailable));
        rx
    }
    /// Blocking dispatch of one action (backends call this off the UI thread).
    fn perform(&self, a: &Action) -> Result<(), ActionError> {
        match a {
            Action::Wifi(on) => self.set_wifi(*on),
            Action::WifiJoin(s, p) => self.wifi_join(s, p.as_deref()),
            Action::Bluetooth(on) => self.set_bluetooth(*on),
            Action::Brightness(p) => self.set_brightness(*p),
            Action::Volume(p) => self.set_volume(*p),
            Action::Muted(m) => self.set_muted(*m),
            Action::NightLight(on) => self.set_night_light(*on),
            Action::MediaToggle => self.media_toggle(),
            Action::Power(m) => self.set_power_mode(*m),
            _ => Err(ActionError::Unavailable),
        }
    }
    /// Runs `a` without blocking the caller; the receiver yields the result once.
    fn spawn(&self, a: Action) -> Done {
        let (tx, rx) = async_channel::bounded(1);
        let _ = tx.try_send(self.perform(&a));
        rx
    }
    /// Re-reads audio now (volume keys -> OSD); resolves once `audio()` has the new value.
    fn refresh_audio(&self) -> Done {
        let (tx, rx) = async_channel::bounded(1);
        let _ = tx.try_send(Ok(()));
        rx
    }
    /// Fires after the snapshot changed (None = static data).
    fn watch(&self) -> Option<async_channel::Receiver<()>> {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WifiNetwork {
    pub ssid: String,
    pub signal: u8,
    pub secure: bool,
    pub connected: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// Seconds.
    pub position: u32,
    pub length: Option<u32>,
    /// Cover art URL (`file://` loads; other schemes show the placeholder).
    pub art: Option<String>,
}

/// Accessibility shortcuts; None = not available here (no Hyprland).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Accessibility {
    pub reduce_motion: Option<bool>,
    pub zoom: Option<bool>,
    pub increase_contrast: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerMode {
    LowPower,
    Automatic,
    HighPerformance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionError {
    Unavailable,
    NeedsAuth,
    Denied,
    NotAdmin,
    /// The request was refused before it ran (e.g. a Wi-Fi password of the wrong length).
    Invalid,
    Failed,
    /// Wi-Fi join: the network refused the password.
    WrongPassword,
}

impl From<wol_core::controls::ControlError> for ActionError {
    fn from(e: wol_core::controls::ControlError) -> Self {
        use wol_core::controls::ControlError as C;
        match e {
            C::Unavailable => Self::Unavailable,
            C::NeedsAuth => Self::NeedsAuth,
            C::Denied => Self::Denied,
            C::NotAdmin => Self::NotAdmin,
            C::Invalid => Self::Invalid,
            C::Failed => Self::Failed,
            C::WrongPassword => Self::WrongPassword,
        }
    }
}

impl ActionError {
    /// The sentence for any surface ([`crate::copy::action`]; contexts have their own there).
    pub fn message(self) -> &'static str {
        crate::copy::action(self)
    }
}

/// Control Center subtitles.
pub fn wifi_subtitle(n: crate::status::Network, nets: &[WifiNetwork]) -> String {
    match nets.iter().find(|w| w.connected) {
        Some(w) => w.ssid.clone(),
        None if n.online && !n.wifi => "Ethernet".into(),
        None => "Not Connected".into(),
    }
}

pub fn battery_label(b: Option<crate::status::Battery>) -> Option<String> {
    let b = b?;
    Some(match b.status {
        crate::status::BatteryStatus::Full => format!("{}% · Charged", b.percent),
        crate::status::BatteryStatus::Charging => format!("{}% · Charging", b.percent),
        crate::status::BatteryStatus::Discharging => format!("{}%", b.percent),
    })
}

pub fn signal_icon(signal: u8) -> &'static str {
    crate::status::network(crate::status::Network {
        online: true,
        wifi: true,
        signal: Some(signal),
    })
}

/// Reads files directly (no process spawn). Audio is a fixed stub until `core` lands.
pub struct Sysfs {
    pub root: std::path::PathBuf,
}

impl Default for Sysfs {
    fn default() -> Self {
        Sysfs {
            root: "/sys/class".into(),
        }
    }
}

fn read_trim(p: std::path::PathBuf) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

pub fn parse_battery(capacity: &str, status: &str) -> Option<Battery> {
    let percent = capacity.trim().parse::<u8>().ok()?.min(100);
    let status = match status.trim() {
        "Charging" => BatteryStatus::Charging,
        "Full" => BatteryStatus::Full,
        _ => BatteryStatus::Discharging,
    };
    Some(Battery { percent, status })
}

impl System for Sysfs {
    fn battery(&self) -> Option<Battery> {
        let d = fs::read_dir(self.root.join("power_supply"))
            .ok()?
            .flatten()
            .find(|e| e.file_name().to_string_lossy().starts_with("BAT"))?;
        parse_battery(
            &read_trim(d.path().join("capacity"))?,
            &read_trim(d.path().join("status")).unwrap_or_default(),
        )
    }
    fn audio(&self) -> Audio {
        Audio {
            volume: Some(60),
            muted: false,
            mic: None,
            mic_muted: false,
        }
    }
    fn network(&self) -> Network {
        let Ok(rd) = fs::read_dir(self.root.join("net")) else {
            return Network::default();
        };
        let mut n = Network::default();
        for e in rd.flatten() {
            if e.file_name() == "lo"
                || read_trim(e.path().join("operstate")).as_deref() != Some("up")
            {
                continue;
            }
            n.online = true;
            if e.path().join("wireless").exists() {
                n.wifi = true;
            }
        }
        n
    }
    fn brightness(&self) -> Option<u8> {
        let d = fs::read_dir(self.root.join("backlight"))
            .ok()?
            .flatten()
            .next()?;
        let cur: u32 = read_trim(d.path().join("brightness"))?.parse().ok()?;
        let max: u32 = read_trim(d.path().join("max_brightness"))?.parse().ok()?;
        (max > 0).then(|| (cur * 100 / max) as u8)
    }
}

/// Deterministic data for golden renders and UI work without hardware (`WOL_FAKE_SYSTEM=1`).
pub struct Fake;

impl System for Fake {
    fn battery(&self) -> Option<Battery> {
        Some(Battery {
            percent: 82,
            status: BatteryStatus::Discharging,
        })
    }
    fn audio(&self) -> Audio {
        Audio {
            volume: Some(64),
            muted: false,
            mic: Some(50),
            mic_muted: false,
        }
    }
    fn network(&self) -> Network {
        Network {
            online: true,
            wifi: true,
            signal: Some(80),
        }
    }
    fn brightness(&self) -> Option<u8> {
        Some(72)
    }
    fn wifi_networks(&self) -> Vec<WifiNetwork> {
        vec![
            WifiNetwork {
                ssid: "Wonderland".into(),
                signal: 88,
                secure: true,
                connected: true,
            },
            WifiNetwork {
                ssid: "Rabbit Hole 5G".into(),
                signal: 60,
                secure: true,
                connected: false,
            },
            WifiNetwork {
                ssid: "Tea Party".into(),
                signal: 30,
                secure: false,
                connected: false,
            },
        ]
    }
    fn bluetooth(&self) -> Option<bool> {
        Some(true)
    }
    fn outputs(&self) -> Vec<String> {
        vec!["Speakers".into(), "Headphones".into()]
    }
    fn now_playing(&self) -> Option<NowPlaying> {
        Some(NowPlaying {
            title: "White Rabbit".into(),
            artist: "Jefferson Airplane".into(),
            playing: true,
            position: 61,
            length: Some(152),
            art: None,
        })
    }
    fn now_playing_icon(&self) -> Option<String> {
        Some("wolos-music".into())
    }
    fn night_light(&self) -> Option<bool> {
        Some(false)
    }
    fn wifi_enabled(&self) -> Option<bool> {
        Some(true)
    }
    fn output_devices(&self) -> Vec<Output> {
        vec![
            Output {
                id: "speakers".into(),
                name: "Built-in Speakers".into(),
                default: true,
            },
            Output {
                id: "headphones".into(),
                name: "Headphones".into(),
                default: false,
            },
        ]
    }
    fn battery_time(&self) -> Option<u32> {
        Some(191)
    }
    fn bt_devices(&self) -> Vec<BtDevice> {
        [
            ("AirPods Pro", true),
            ("MX Keys", true),
            ("Magic Trackpad", false),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (n, c))| BtDevice {
            addr: format!("AA:BB:CC:DD:EE:0{i}"),
            name: n.into(),
            connected: c,
        })
        .collect()
    }
    fn keyboard_brightness(&self) -> Option<u8> {
        Some(40)
    }
    fn power_mode(&self) -> Option<PowerMode> {
        Some(PowerMode::Automatic)
    }
    fn accessibility(&self) -> Accessibility {
        Accessibility {
            reduce_motion: Some(false),
            zoom: Some(false),
            increase_contrast: false,
        }
    }
    fn monitors(&self) -> Vec<Monitor> {
        vec![Monitor {
            name: "eDP-1".into(),
            description: "Built-in Display".into(),
            width: 2560,
            height: 1600,
            refresh: 60.0,
            scale: 2.0,
            modes: vec![
                "2560x1600@60.00Hz".into(),
                "1920x1200@60.00Hz".into(),
                "1440x900@60.00Hz".into(),
            ],
        }]
    }
    // Settings > Assistants & Automation and Restore: a helper seen once, a few changes, restore points.
    fn helpers(&self) -> wol_core::automation::Status {
        let mut s = wol_core::automation::status(
            &wol_core::automation::policy::Policy {
                enabled: false,
                ..Default::default()
            },
            &[],
        );
        s.helpers = vec![wol_core::automation::audit::HelperSeen {
            name: "Claude".into(),
            via: "MCP".into(),
            first: 0,
            last: 0,
            actions: 12,
            refused: 1,
        }];
        s
    }
    fn history(&self) -> Vec<wol_core::journal::Entry> {
        let now = wol_core::journal::now();
        [
            (4, 300, "settings", "theme"),
            (3, 2 * 3600, "mcp:Claude", "wallpaper"),
            (2, 26 * 3600, "settings", "desktop"),
            (1, 3 * 86_400, "cli", "shortcuts"),
        ]
        .into_iter()
        .map(|(id, ago, by, what)| wol_core::journal::Entry {
            id,
            t: now - ago,
            by: by.into(),
            what: what.into(),
            changes: vec![],
            undoes: None,
        })
        .collect()
    }
    fn boot_environments(&self) -> Vec<wol_core::update::be::BootEnv> {
        let be = |name: &str, active, created: &str, space: &str| wol_core::update::be::BootEnv {
            name: name.into(),
            active,
            next_boot: active,
            mountpoint: active.then(|| "/".into()),
            space: space.into(),
            created: created.into(),
        };
        vec![
            be("default", true, "2026-10-01 09:12", "9.8G"),
            be(
                "wol-before-update-1792514400",
                false,
                "2026-10-20 18:40",
                "412M",
            ),
            be(
                "wol-before-update-1791789780",
                false,
                "2026-10-12 08:03",
                "380M",
            ),
        ]
    }
}

/// [`Fake`] on a desktop: no battery, no backlights, no Low Power Mode.
pub struct FakeDesktop;

impl System for FakeDesktop {
    fn battery(&self) -> Option<Battery> {
        None
    }
    fn audio(&self) -> Audio {
        Fake.audio()
    }
    fn network(&self) -> Network {
        Network {
            online: true,
            wifi: false,
            signal: None,
        }
    }
    fn brightness(&self) -> Option<u8> {
        None
    }
    fn wifi_networks(&self) -> Vec<WifiNetwork> {
        Fake.wifi_networks()
    }
    fn wifi_enabled(&self) -> Option<bool> {
        Some(true)
    }
    fn bluetooth(&self) -> Option<bool> {
        Some(false)
    }
    fn output_devices(&self) -> Vec<Output> {
        Fake.output_devices()
    }
    fn now_playing(&self) -> Option<NowPlaying> {
        None
    }
    fn night_light(&self) -> Option<bool> {
        Fake.night_light()
    }
    fn accessibility(&self) -> Accessibility {
        Fake.accessibility()
    }
}

// ---- Real backend ----

use std::sync::{Arc, Mutex, Weak};
use wol_core::controls::{A11y, BatteryState, Charge, Controls, Media, Volume, WifiNet};

/// Everything the UI reads; refreshed by the worker, cached to `$XDG_RUNTIME_DIR/wol-controls.json`
/// so a freshly opened Control Center shows real values before the first refresh lands.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Snapshot {
    pub wifi_on: Option<bool>,
    pub nets: Vec<WifiNet>,
    pub volume: Option<Volume>,
    pub outputs: Vec<Output>,
    pub brightness: Option<u8>,
    pub bluetooth: Option<bool>,
    pub monitors: Vec<Monitor>,
    pub night: Option<bool>,
    pub battery: Option<BatteryState>,
    /// FreeBSD default-route interface (network status without sysfs).
    pub route: Option<String>,
    pub media: Option<Media>,
    pub bt_devices: Vec<BtDevice>,
    pub keyboard: Option<u8>,
    pub low_power: Option<bool>,
    pub a11y: Option<A11y>,
}

/// Seconds between refreshes per topic (system-api.md TTLs).
const TTL_AUDIO: u64 = 2;
const TTL_WIFI: u64 = 5;
const TTL_SLOW: u64 = 10;
/// Refresh cadence while a surface watches the snapshot (Control Center, Settings).
pub const WATCHED_PERIOD: std::time::Duration = std::time::Duration::from_secs(1);
/// Cadence with nobody watching: the menu bar polls every 5 s; nothing needs fresher data.
pub const IDLE_PERIOD: std::time::Duration = std::time::Duration::from_secs(10);
/// Control Center-only topics: never refreshed while nobody watches.
const NEVER: u64 = u64::MAX;
/// Topics: audio, wifi, slow, media (Now Playing progress), panel (Bluetooth devices, keyboard
/// backlight, Low Power Mode, accessibility, Night Shift).
const TOPICS: usize = 5;
/// TTLs (s) per topic while watched / unwatched. Unwatched, only the menu bar reads: volume keys
/// refresh audio on demand (`refresh_audio`), so every helper runs once a minute.
const TTLS_WATCHED: [u64; TOPICS] = [TTL_AUDIO, TTL_WIFI, TTL_SLOW, 1, TTL_WIFI];
const TTLS_IDLE: [u64; TOPICS] = [60, 60, 60, NEVER, NEVER];

struct Inner {
    c: Controls,
    files: Sysfs,
    snap: Mutex<Snapshot>,
    /// Admin password for the next root action (cleared after use).
    password: Mutex<String>,
    watchers: Mutex<Vec<async_channel::Sender<()>>>,
    cache: Option<std::path::PathBuf>,
    /// Last refresh per topic; None = never.
    last: Mutex<[Option<std::time::Instant>; TOPICS]>,
    /// One refresh at a time: each writes back a whole snapshot.
    refreshing: Mutex<()>,
}

impl Inner {
    fn lock(&self) -> std::sync::MutexGuard<'_, Snapshot> {
        self.snap.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Next refresh delay: fast only while a live watcher exists.
    fn period(&self) -> std::time::Duration {
        let mut w = self.watchers.lock().unwrap_or_else(|e| e.into_inner());
        w.retain(|t| !t.is_closed());
        if w.is_empty() {
            IDLE_PERIOD
        } else {
            WATCHED_PERIOD
        }
    }

    /// Refreshes every topic whose TTL elapsed (all when `force`).
    fn refresh(&self, force: bool) {
        let now = std::time::Instant::now();
        let due = {
            let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
            let mut due = [false; TOPICS];
            let ttls = if self.period() == WATCHED_PERIOD {
                TTLS_WATCHED
            } else {
                TTLS_IDLE
            };
            for (i, ttl) in ttls.into_iter().enumerate() {
                // 100 ms slack: a 1 s tick must not miss a 2 s TTL by jitter.
                let ready = |t: std::time::Instant| {
                    now.duration_since(t).as_millis() + 100 >= u128::from(ttl) * 1000
                };
                if ttl == NEVER && !force {
                    continue;
                }
                if force || last[i].is_none_or(ready) {
                    last[i] = Some(now);
                    due[i] = true;
                }
            }
            due
        };
        self.refresh_topics(due);
    }

    /// Audio only, now (volume keys); the other topics keep their schedule.
    fn refresh_audio(&self) {
        self.last.lock().unwrap_or_else(|e| e.into_inner())[0] = Some(std::time::Instant::now());
        self.refresh_topics([true, false, false, false, false]);
    }

    fn refresh_topics(&self, due: [bool; TOPICS]) {
        let _one = self.refreshing.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = self.lock().clone();
        let before = s.clone();
        if due[0] {
            s.volume = self.c.volume().or(s.volume);
            s.brightness = self.c.brightness();
        }
        if due[1] {
            s.wifi_on = self.c.wifi_enabled();
            s.nets = self.c.wifi_networks();
        }
        if due[2] {
            s.outputs = self.c.outputs();
            s.bluetooth = self.c.bluetooth();
            s.monitors = self.c.monitors();
            s.battery = self.c.battery();
            if self.c.host.is_freebsd() {
                s.route = self.c.default_iface();
            }
        }
        if due[3] {
            s.media = self.c.now_playing();
        }
        if due[4] {
            s.bt_devices = self.c.bt_devices();
            s.keyboard = self.c.keyboard_brightness();
            s.low_power = self.c.low_power();
            s.a11y = self.c.accessibility();
            s.night = self.c.night_shift();
        }
        if s != before {
            *self.lock() = s.clone();
            if let (Some(p), Ok(j)) = (&self.cache, serde_json::to_string(&s)) {
                let tmp = p.with_extension("tmp");
                if fs::write(&tmp, j).is_ok() {
                    let _ = fs::rename(tmp, p);
                }
            }
            self.watchers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|t| !t.is_closed() && (t.try_send(()).is_ok() || t.is_full()));
        }
    }

    fn take_password(&self) -> String {
        std::mem::take(&mut *self.password.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Blocking; runs on a worker thread.
    fn perform(&self, a: &Action) -> Result<(), ActionError> {
        let c = &self.c;
        let pw = self.take_password();
        let r = match a {
            Action::Wifi(on) => c.set_wifi(*on, &pw),
            Action::WifiJoin(ssid, p) => c.wifi_join(ssid, p.as_deref(), &pw),
            Action::Bluetooth(on) => c.set_bluetooth(*on, &pw),
            Action::Brightness(p) => c.set_brightness(*p),
            Action::Volume(p) => c.set_volume(*p),
            Action::Muted(m) => c.set_muted(*m),
            Action::Output(id) => c.set_output(id, &pw),
            Action::NightLight(on) => c.set_night_shift(*on),
            Action::Monitor(rule) => c.set_monitor(rule),
            Action::UserAdd {
                name,
                full_name,
                admin,
                password,
            } => c.add_user(name, full_name, *admin, password, &pw),
            Action::PkgUpgrade => c.pkg_upgrade(&pw),
            Action::PkgInstall(t) => c.pkg_install(t, &pw),
            Action::MediaToggle => c.media("play-pause"),
            Action::MediaNext => c.media("next"),
            Action::MediaPrevious => c.media("previous"),
            Action::Power(m) => c.set_low_power(*m == PowerMode::LowPower, &pw),
            Action::BtConnect(addr, on) => c.bt_connect(addr, *on),
            Action::KeyboardBrightness(p) => c.set_keyboard_brightness(*p),
            Action::ReduceMotion(on) => c.set_reduce_motion(*on),
            Action::Zoom(on) => c.set_zoom(*on),
            Action::IncreaseContrast(on) => crate::kit::style::set_contrast(*on)
                .map_err(|_| wol_core::controls::ControlError::Failed),
            Action::Desktop(p) => wol_core::hyprprefs::apply_desktop(&*c.host, &config_home(), p)
                .map(drop)
                .map_err(|_| wol_core::controls::ControlError::Failed),
            Action::Shortcuts(o) => {
                wol_core::hyprprefs::apply_shortcuts(&*c.host, &config_home(), o)
                    .map(drop)
                    .map_err(|_| wol_core::controls::ControlError::Failed)
            }
            Action::ShortcutCapture(on) => {
                if wol_core::hyprprefs::capture(&*c.host, *on) {
                    Ok(())
                } else {
                    Err(wol_core::controls::ControlError::Unavailable)
                }
            }
            Action::BootEnvironment(name) => c.boot_environment(name, &pw),
            Action::Admin(req) => admin(req, &pw),
        };
        // Optimistic snapshot so the next read (and other surfaces) agree right away.
        if r.is_ok() {
            let mut s = self.lock();
            match a {
                Action::Volume(p) => {
                    s.volume = Some(Volume {
                        percent: *p,
                        muted: s.volume.is_some_and(|v| v.muted),
                    })
                }
                Action::Muted(m) => s.volume = s.volume.map(|v| Volume { muted: *m, ..v }),
                Action::Brightness(p) => s.brightness = Some(*p),
                Action::Wifi(on) => s.wifi_on = Some(*on),
                Action::Bluetooth(on) => s.bluetooth = Some(*on),
                Action::NightLight(on) => s.night = Some(*on),
                Action::KeyboardBrightness(p) => s.keyboard = Some(*p),
                Action::Power(m) => s.low_power = Some(*m == PowerMode::LowPower),
                Action::ReduceMotion(on) => {
                    if let Some(a) = &mut s.a11y {
                        a.reduce_motion = *on
                    }
                }
                Action::Zoom(on) => {
                    if let Some(a) = &mut s.a11y {
                        a.zoom = *on
                    }
                }
                Action::MediaToggle => {
                    if let Some(m) = &mut s.media {
                        m.playing = !m.playing
                    }
                }
                Action::BtConnect(addr, on) => {
                    for d in s.bt_devices.iter_mut().filter(|d| &d.addr == addr) {
                        d.connected = *on;
                    }
                }
                Action::Output(id) => {
                    for o in &mut s.outputs {
                        o.default = &o.id == id;
                    }
                }
                _ => {}
            }
        }
        r.map_err(ActionError::from)
    }
}

/// One wol-admind request (validated here first: a malformed one never leaves the process).
fn admin(
    req: &shared::admin::AdminRequest,
    pw: &str,
) -> Result<(), wol_core::controls::ControlError> {
    use wol_core::controls::{ControlError, admin_request, from_reply};
    if wol_core::admin::validate(req).is_some() {
        return Err(ControlError::Invalid);
    }
    admin_request(std::path::Path::new(shared::admin::SOCKET), req, pw)
        .map_or(Err(ControlError::Unavailable), from_reply)
}

/// Real system: `wol_core::controls` on a background thread; the UI only reads the snapshot.
pub struct CoreSystem {
    inner: Arc<Inner>,
}

impl CoreSystem {
    pub fn new() -> Self {
        let cache = std::env::var_os("XDG_RUNTIME_DIR")
            .map(|d| std::path::PathBuf::from(d).join("wol-controls.json"));
        Self::with(
            Controls::new(Arc::new(wol_core::system::PosixHost)),
            Sysfs::default(),
            cache,
        )
    }

    /// Injected controls (tests use a `FakeHost`). Refreshes now, then every `Inner::period`.
    pub fn with(c: Controls, files: Sysfs, cache: Option<std::path::PathBuf>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let sys = Self::with_ticks(c, files, cache, rx);
        let weak = Arc::downgrade(&sys.inner);
        let _ = std::thread::Builder::new()
            .name("wol-controls-tick".into())
            .spawn(move || {
                while tx.send(()).is_ok() {
                    let Some(period) = weak.upgrade().map(|i| i.period()) else {
                        break;
                    };
                    std::thread::sleep(period);
                }
            });
        sys
    }

    /// Refresh worker driven by `ticks` (one refresh per message); stops when either side drops.
    pub fn with_ticks(
        c: Controls,
        files: Sysfs,
        cache: Option<std::path::PathBuf>,
        ticks: std::sync::mpsc::Receiver<()>,
    ) -> Self {
        let snap = cache
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let inner = Arc::new(Inner {
            c,
            files,
            snap: Mutex::new(snap),
            password: Default::default(),
            watchers: Default::default(),
            cache,
            last: Default::default(),
            refreshing: Default::default(),
        });
        let weak: Weak<Inner> = Arc::downgrade(&inner);
        let _ = std::thread::Builder::new()
            .name("wol-controls".into())
            .spawn(move || {
                while ticks.recv().is_ok() {
                    let Some(i) = weak.upgrade() else { break };
                    i.refresh(false);
                }
            });
        CoreSystem { inner }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.inner.lock().clone()
    }
}

impl Default for CoreSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl System for CoreSystem {
    fn battery(&self) -> Option<Battery> {
        let Some(b) = self.inner.lock().battery else {
            return self.inner.files.battery();
        };
        Some(Battery {
            percent: b.percent,
            status: match b.charge {
                Charge::Charging => BatteryStatus::Charging,
                Charge::Discharging => BatteryStatus::Discharging,
                Charge::Full => BatteryStatus::Full,
            },
        })
    }
    fn battery_time(&self) -> Option<u32> {
        self.inner.lock().battery.and_then(|b| b.minutes)
    }
    fn now_playing(&self) -> Option<NowPlaying> {
        self.inner.lock().media.as_ref().map(|m| NowPlaying {
            title: m.title.clone(),
            artist: m.artist.clone(),
            playing: m.playing,
            position: m.position,
            length: m.length,
            art: m.art.clone(),
        })
    }
    fn now_playing_icon(&self) -> Option<String> {
        let player = self.inner.lock().media.as_ref()?.player.clone()?;
        Some(player_icon(&player))
    }
    fn bt_devices(&self) -> Vec<BtDevice> {
        self.inner.lock().bt_devices.clone()
    }
    fn keyboard_brightness(&self) -> Option<u8> {
        self.inner.lock().keyboard
    }
    fn power_status(&self) -> Option<wol_core::power::policy::Status> {
        use wol_core::power::policy;
        let host = &*self.inner.c.host;
        let saved = std::fs::read_to_string(policy::STATE_FILE).ok();
        let facts = policy::facts(&|argv: &[&str]| host.run(argv));
        let state = policy::State::parse(saved.as_deref());
        Some(policy::status(&facts, &state))
    }
    fn security_status(&self) -> Option<wol_core::security::Status> {
        let host = &*self.inner.c.host;
        Some(wol_core::security::status(&|argv: &[&str]| host.run(argv)))
    }
    fn power_mode(&self) -> Option<PowerMode> {
        self.inner.lock().low_power.map(|l| {
            if l {
                PowerMode::LowPower
            } else {
                PowerMode::Automatic
            }
        })
    }
    fn accessibility(&self) -> Accessibility {
        let a = self.inner.lock().a11y;
        Accessibility {
            reduce_motion: a.map(|a| a.reduce_motion),
            zoom: a.map(|a| a.zoom),
            increase_contrast: crate::kit::style::contrast_on(),
        }
    }
    fn audio(&self) -> Audio {
        let v = self.inner.lock().volume;
        Audio {
            volume: v.map(|v| v.percent),
            muted: v.is_some_and(|v| v.muted),
            mic: None,
            mic_muted: false,
        }
    }
    fn network(&self) -> Network {
        let mut n = self.inner.files.network();
        let s = self.inner.lock();
        if let Some(iface) = &s.route {
            n.online = true;
            n.wifi |= iface.starts_with("wlan");
        }
        if let Some(w) = s.nets.iter().find(|w| w.connected) {
            n.wifi = true;
            n.online = true;
            n.signal = Some(w.signal);
        }
        n
    }
    fn brightness(&self) -> Option<u8> {
        self.inner.lock().brightness
    }
    fn wifi_enabled(&self) -> Option<bool> {
        self.inner.lock().wifi_on
    }
    fn wifi_networks(&self) -> Vec<WifiNetwork> {
        self.inner
            .lock()
            .nets
            .iter()
            .map(|n| WifiNetwork {
                ssid: n.ssid.clone(),
                signal: n.signal,
                secure: n.secure,
                connected: n.connected,
            })
            .collect()
    }
    fn bluetooth(&self) -> Option<bool> {
        self.inner.lock().bluetooth
    }
    fn outputs(&self) -> Vec<String> {
        self.output_devices().into_iter().map(|o| o.name).collect()
    }
    fn output_devices(&self) -> Vec<Output> {
        self.inner.lock().outputs.clone()
    }
    fn monitors(&self) -> Vec<Monitor> {
        self.inner.lock().monitors.clone()
    }
    /// Probed from hyprsunset: None (tile hidden) when it is not running.
    fn night_light(&self) -> Option<bool> {
        self.inner.lock().night
    }
    fn authorize(&self, password: &str) -> Result<(), ActionError> {
        *self
            .inner
            .password
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = password.into();
        Ok(())
    }
    // Sync setters: same path as `spawn`, for callers that are already off the UI thread.
    fn set_volume(&self, p: u8) -> Result<(), ActionError> {
        self.inner.perform(&Action::Volume(p))
    }
    fn set_brightness(&self, p: u8) -> Result<(), ActionError> {
        self.inner.perform(&Action::Brightness(p))
    }
    fn perform(&self, a: &Action) -> Result<(), ActionError> {
        self.inner.perform(a)
    }
    fn spawn(&self, a: Action) -> Done {
        let (tx, rx) = async_channel::bounded(1);
        let i = self.inner.clone();
        let _ = std::thread::Builder::new()
            .name("wol-action".into())
            .spawn(move || {
                let r = i.perform(&a);
                // Reply with the host's state already read, so callers never see it go stale after.
                i.refresh(true);
                let _ = tx.try_send(r);
            });
        rx
    }
    fn helpers(&self) -> wol_core::automation::Status {
        wol_core::automation::user_status()
    }
    fn set_helpers_enabled(&self, on: bool) -> Result<(), ActionError> {
        wol_core::automation::set_enabled(&journal(), BY, on, wol_core::journal::now())
            .map(drop)
            .map_err(|_| ActionError::Failed)
    }
    fn set_helper_capability(&self, id: &str, allowed: bool) -> Result<(), ActionError> {
        wol_core::automation::set_capability(&journal(), BY, id, allowed, wol_core::journal::now())
            .map(drop)
            .map_err(|_| ActionError::Failed)
    }
    fn allow_helper_always(&self, helper: &str, cap: &str) -> Result<(), ActionError> {
        wol_core::automation::allow_always(&journal(), BY, helper, cap, wol_core::journal::now())
            .map(drop)
            .map_err(|_| ActionError::Failed)
    }
    fn history(&self) -> Vec<wol_core::journal::Entry> {
        let mut e = journal().entries();
        e.reverse();
        e
    }
    fn undo(&self, id: Option<u64>) -> Result<wol_core::journal::Entry, String> {
        let e = journal()
            .undo(id, BY, wol_core::journal::now())
            .map_err(|e| crate::copy::undo(&e).to_string())?;
        after_undo(&self.inner.c, &wol_core::journal::reload_for(&e));
        Ok(e)
    }
    fn boot_environments(&self) -> Vec<wol_core::update::be::BootEnv> {
        wol_core::update::be::read(&*self.inner.c.host).unwrap_or_default()
    }
    fn approve(&self, req: shared::shell::ApprovalRequest, password: String) -> Done {
        let (tx, rx) = async_channel::bounded(1);
        let i = self.inner.clone();
        let _ = std::thread::Builder::new()
            .name("wol-approve".into())
            .spawn(move || {
                let r = approve(&i.c, &req, &password);
                i.refresh(true);
                let _ = tx.try_send(r);
            });
        rx
    }
    fn refresh_audio(&self) -> Done {
        let (tx, rx) = async_channel::bounded(1);
        let i = self.inner.clone();
        let _ = std::thread::Builder::new()
            .name("wol-audio".into())
            .spawn(move || {
                i.refresh_audio();
                let _ = tx.try_send(Ok(()));
            });
        rx
    }
    /// Starts fast refresh and refreshes the due topics now (the tick thread may be asleep for
    /// the idle period; Control Center-only topics were never read while hidden).
    fn watch(&self) -> Option<async_channel::Receiver<()>> {
        let (tx, rx) = async_channel::bounded(1);
        self.inner
            .watchers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(tx);
        let i = self.inner.clone();
        let _ = std::thread::Builder::new()
            .name("wol-controls-open".into())
            .spawn(move || i.refresh(false));
        Some(rx)
    }
}

/// MPRIS player name -> app icon (dock pin, installed entry, else the name), read once per player.
fn player_icon(player: &str) -> String {
    static SEEN: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, icon)) = seen.iter().find(|(p, _)| p == player) {
        return icon.clone();
    }
    let icon = crate::apps::icon_for(player, crate::apps::PINS, &crate::desktop::load_all());
    seen.push((player.to_string(), icon.clone()));
    icon
}

fn config_home() -> std::path::PathBuf {
    crate::kit::config_home().into()
}

/// Journal actor for changes made in Settings, Control Center and the shell.
const BY: &str = "settings";

fn journal() -> wol_core::journal::Journal {
    wol_core::journal::Journal::for_config(&config_home())
}

/// A helper's admin request after the user typed the password on the approval sheet. The session's
/// cached authorization is forgotten first (the password is always checked) and again after (the
/// helper is left nothing to reuse).
pub fn approve(
    c: &Controls,
    req: &shared::shell::ApprovalRequest,
    password: &str,
) -> Result<(), ActionError> {
    use shared::shell::ApprovalRequest as R;
    // User-level changes run in the helper after Allow; never here, and wol-admind never hears of them.
    if !req.is_admin() {
        return Err(ActionError::Unavailable);
    }
    c.revoke();
    let r = match req {
        R::Wifi { on } => c.set_wifi(*on, password),
        R::WifiJoin { ssid, password: p } => c.wifi_join(ssid, p.as_deref(), password),
        R::Bluetooth { on } => c.set_bluetooth(*on, password),
        R::BootEnvironment { name } => c.boot_environment(name, password),
        R::Change { .. } => unreachable!("refused above"),
    };
    c.revoke();
    // No password means no approval: the sheet asks again instead of reporting "unavailable".
    r.map_err(|e| match ActionError::from(e) {
        ActionError::NeedsAuth => ActionError::Denied,
        e => e,
    })
}

/// What an undo touched follows it: other apps restyle, shell prefs and Hyprland reload.
fn after_undo(c: &Controls, r: &wol_core::journal::Reload) {
    use wol_core::theme::{accent, active};
    if let Some(saved) = &r.theme {
        let dark = accent::auto_is_dark(crate::kit::now().hour);
        let id = active::effective(accent::effective(saved.as_deref(), dark).as_deref(), dark);
        if let Err(e) = wol_core::theme::apps::apply(&id, &config_home()) {
            eprintln!("wol: app themes: {e}");
        }
    }
    if r.shell {
        crate::kit::spawn(&["wol".into(), "shell".into(), "reload".into()]);
    }
    if r.hyprland {
        let _ = c.host.run(&["hyprctl", "reload"]);
    }
}

/// `WOL_FAKE_SYSTEM=1` -> `Fake`, else [`CoreSystem`].
pub fn default_system() -> std::rc::Rc<dyn System> {
    if std::env::var_os("WOL_FAKE_SYSTEM").is_some() {
        std::rc::Rc::new(Fake)
    } else {
        std::rc::Rc::new(CoreSystem::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_center_labels() {
        use crate::status::*;
        let nets = [WifiNetwork {
            ssid: "Home".into(),
            signal: 80,
            secure: true,
            connected: true,
        }];
        assert_eq!(
            wifi_subtitle(
                Network {
                    online: true,
                    wifi: true,
                    signal: None
                },
                &nets
            ),
            "Home"
        );
        assert_eq!(
            wifi_subtitle(
                Network {
                    online: true,
                    wifi: false,
                    signal: None
                },
                &[]
            ),
            "Ethernet"
        );
        assert_eq!(wifi_subtitle(Network::default(), &[]), "Not Connected");
        assert_eq!(
            battery_label(Some(Battery {
                percent: 82,
                status: BatteryStatus::Charging
            }))
            .unwrap(),
            "82% · Charging"
        );
        assert_eq!(battery_label(None), None);
        assert_eq!(signal_icon(30), "network-wireless-signal-ok-symbolic");
    }

    /// Helper approvals always check the password and leave no cached authorization behind.
    #[test]
    fn approve_revokes_around_the_admin_request() {
        use shared::admin::AdminReply;
        use shared::shell::ApprovalRequest as R;
        let sent = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = sent.clone();
        let mut c = Controls::new(Arc::new(wol_core::system::fake::FakeHost::new(true)));
        c.admin = Arc::new(move |req, pw| {
            log.lock()
                .unwrap()
                .push(format!("{} {}", req.action.name(), req.args.join(" ")));
            Some(match pw {
                "" => AdminReply::Ok,
                "good" => AdminReply::Ok,
                _ => AdminReply::Denied,
            })
        });
        let be = R::BootEnvironment {
            name: "wol-before-update-1".into(),
        };
        assert_eq!(approve(&c, &be, "good"), Ok(()));
        let change = R::Change {
            capability: "appearance".into(),
            what: "switch to Dark appearance".into(),
        };
        assert_eq!(
            approve(&c, &change, "good"),
            Err(ActionError::Unavailable),
            "user-level changes never run as an admin request"
        );
        assert_eq!(
            approve(&c, &R::Wifi { on: false }, "bad"),
            Err(ActionError::Denied)
        );
        assert_eq!(
            *sent.lock().unwrap(),
            [
                "revoke ",
                "boot-environment wol-before-update-1",
                "revoke ",
                "revoke ",
                "wifi-power wlan0 off",
                "revoke "
            ]
        );
    }

    #[test]
    fn battery_files() {
        assert_eq!(
            parse_battery("55\n", "Charging\n"),
            Some(Battery {
                percent: 55,
                status: BatteryStatus::Charging
            })
        );
        assert_eq!(parse_battery("x", "Full"), None);
    }

    #[test]
    fn reads_fake_sysfs_tree() {
        let root = std::env::temp_dir().join(format!("wol-sysfs-{}", std::process::id()));
        let bat = root.join("power_supply/BAT0");
        fs::create_dir_all(&bat).unwrap();
        fs::write(bat.join("capacity"), "20\n").unwrap();
        fs::write(bat.join("status"), "Discharging\n").unwrap();
        let wl = root.join("net/wlan0/wireless");
        fs::create_dir_all(&wl).unwrap();
        fs::write(root.join("net/wlan0/operstate"), "up\n").unwrap();
        let s = Sysfs { root: root.clone() };
        assert_eq!(s.battery().unwrap().percent, 20);
        assert_eq!(
            s.network(),
            Network {
                online: true,
                wifi: true,
                signal: None
            }
        );
        assert_eq!(s.brightness(), None);
        fs::remove_dir_all(root).unwrap();
    }

    /// Settings > Battery, Control Center and Privacy & Security read the policies' own status.
    #[test]
    fn power_and_security_status_come_from_the_host() {
        use wol_core::system::fake::FakeHost;
        let h = Arc::new(FakeHost::new(false));
        h.command("sysctl -n hw.acpi.acline", "0\n");
        h.command("sysctl -n hw.acpi.battery.life", "64\n");
        h.command("sysrc -n pf_enable", "YES\n");
        let sys = CoreSystem::with_ticks(
            Controls::new(h),
            Sysfs::default(),
            None,
            std::sync::mpsc::channel().1,
        );
        let p = sys.power_status().unwrap();
        assert_eq!(p.source, wol_core::power::policy::Source::Battery);
        assert_eq!(p.battery, Some(64));
        assert_eq!(sys.security_status().unwrap().firewall, Some(true));
        assert!(Fake.power_status().is_none(), "previews read nothing");
    }

    /// Volume keys: the OSD waited for the next poll (up to 10 s). `refresh_audio` reads the
    /// sink now, off the UI thread, and replies once the snapshot has it.
    #[test]
    fn refresh_audio_reads_the_sink_now() {
        use wol_core::system::fake::FakeHost;
        let h = Arc::new(FakeHost::new(false));
        h.command("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.42\n");
        let (_tick, ticks) = std::sync::mpsc::channel();
        let sys = CoreSystem::with_ticks(Controls::new(h), Sysfs::default(), None, ticks);
        assert_eq!(sys.audio().volume, None, "no tick yet");
        assert_eq!(sys.refresh_audio().recv_blocking(), Ok(Ok(())));
        assert_eq!(sys.audio().volume, Some(42));
    }

    /// Live idle: the refresh worker woke every second (10 wakeups/s in wol-shell, plus helper
    /// processes) with nobody looking. Fast refresh only while a watcher (Control Center) is open.
    #[test]
    fn refreshes_fast_only_while_watched() {
        use wol_core::system::fake::FakeHost;
        let (_tick, ticks) = std::sync::mpsc::channel();
        let sys = CoreSystem::with_ticks(
            Controls::new(Arc::new(FakeHost::new(false))),
            Sysfs::default(),
            None,
            ticks,
        );
        assert_eq!(sys.inner.period(), IDLE_PERIOD);
        let w = sys.watch().unwrap();
        assert_eq!(sys.inner.period(), WATCHED_PERIOD);
        drop(w);
        assert_eq!(sys.inner.period(), IDLE_PERIOD);
        assert!(IDLE_PERIOD >= std::time::Duration::from_secs(10));
        assert!(
            TTLS_IDLE[1] >= 60 && TTLS_IDLE[2] >= 60,
            "scans are the expensive helpers"
        );
    }

    #[test]
    fn core_system_reads_snapshot_and_acts_off_thread() {
        use wol_core::system::fake::FakeHost;
        let h = Arc::new(FakeHost::new(false));
        h.command("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.64\n")
            .command("nmcli radio wifi", "enabled\n")
            .command(
                "nmcli -t -f IN-USE,SIGNAL,SECURITY,SSID dev wifi list --rescan no",
                "*:70:WPA2:Home\n :40::Cafe\n",
            )
            .command("wpctl set-volume @DEFAULT_AUDIO_SINK@ 30%", "");
        let root = std::env::temp_dir().join(format!("wol-core-sys-{}", std::process::id()));
        let cache = root.join("snap.json");
        fs::create_dir_all(&root).unwrap();
        let (tick, ticks) = std::sync::mpsc::channel();
        let sys = CoreSystem::with_ticks(
            Controls::new(h.clone()),
            Sysfs { root: root.clone() },
            Some(cache.clone()),
            ticks,
        );
        let w = sys.watch().unwrap();
        tick.send(()).unwrap();
        w.recv_blocking().unwrap();
        assert_eq!(sys.audio().volume, Some(64));
        assert_eq!(sys.wifi_enabled(), Some(true));
        assert_eq!(wifi_subtitle(sys.network(), &sys.wifi_networks()), "Home");
        assert_eq!(sys.network().signal, Some(70));
        h.command("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.30\n");
        let done = sys.spawn(Action::Volume(30));
        assert_eq!(done.recv_blocking(), Ok(Ok(())));
        assert_eq!(sys.audio().volume, Some(30), "refreshed before the reply");
        assert_eq!(
            sys.spawn(Action::UserAdd {
                name: "dinah".into(),
                full_name: "Dinah".into(),
                admin: false,
                password: "n3w-passw0rd".into(),
            })
            .recv_blocking(),
            Ok(Err(ActionError::Unavailable)),
            "no wol-admind on the test host"
        );
        let cached: Snapshot = serde_json::from_str(&fs::read_to_string(&cache).unwrap()).unwrap();
        assert_eq!(cached.nets.len(), 2);
        drop(sys);
        fs::remove_dir_all(root).unwrap();
    }

    fn bare(h: &Arc<wol_core::system::fake::FakeHost>) -> CoreSystem {
        let (_tick, ticks) = std::sync::mpsc::channel();
        let root = std::env::temp_dir().join("wol-no-sysfs");
        CoreSystem::with_ticks(Controls::new(h.clone()), Sysfs { root }, None, ticks)
    }

    /// Control Center-only helpers (playerctl, bluetoothctl, hyprctl getoption) never run while
    /// nobody watches; opening a watcher reads them at once instead of after the idle period.
    #[test]
    fn panel_topics_refresh_only_while_watched() {
        use wol_core::system::fake::FakeHost;
        let h = Arc::new(FakeHost::new(false));
        h.command(
            &format!(
                "playerctl metadata --format {}",
                wol_core::controls::PLAYERCTL_FORMAT
            ),
            "Playing\t61000000\t152000000\tSong\tBand\t\n",
        )
        .command("powerprofilesctl get", "balanced\n");
        let sys = bare(&h);
        sys.inner.refresh(false);
        assert!(
            !h.ran()
                .iter()
                .any(|r| r.starts_with("playerctl") || r.starts_with("powerprofilesctl")),
            "{:?}",
            h.ran()
        );
        assert_eq!(sys.now_playing(), None);
        let w = sys.watch().unwrap();
        w.recv_blocking().unwrap();
        let np = sys.now_playing().unwrap();
        assert_eq!(
            (np.title.as_str(), np.position, np.length),
            ("Song", 61, Some(152))
        );
        assert_eq!(sys.power_mode(), Some(PowerMode::Automatic));
    }

    /// FreeBSD has no /sys: battery and online state come from sysctls and the default route.
    #[test]
    fn freebsd_battery_and_network_without_sysfs() {
        use wol_core::system::fake::FakeHost;
        let h = Arc::new(FakeHost::new(true));
        h.int("hw.acpi.battery.life", 47)
            .int("hw.acpi.battery.time", 95)
            .int("hw.acpi.battery.state", 1)
            .command("route -n get default", "  interface: em0\n");
        let sys = bare(&h);
        assert_eq!(sys.battery(), None);
        sys.inner.refresh(true);
        assert_eq!(
            sys.battery(),
            Some(Battery {
                percent: 47,
                status: BatteryStatus::Discharging
            })
        );
        assert_eq!(sys.battery_time(), Some(95));
        assert_eq!(
            crate::status::network(sys.network()),
            "network-wired-symbolic"
        );
    }

    #[test]
    fn panel_actions_update_the_snapshot() {
        use wol_core::system::fake::FakeHost;
        let h = Arc::new(FakeHost::new(false));
        h.command("hyprctl keyword cursor:zoom_factor 2", "ok")
            .command("hyprctl -j getoption animations:enabled", r#"{"int":1}"#)
            .command(
                "hyprctl -j getoption cursor:zoom_factor",
                r#"{"float":1.0}"#,
            )
            .command("playerctl next", "");
        let sys = bare(&h);
        sys.inner.refresh(true);
        assert_eq!(sys.accessibility().zoom, Some(false));
        assert_eq!(sys.perform(&Action::Zoom(true)), Ok(()));
        assert_eq!(sys.accessibility().zoom, Some(true));
        assert_eq!(sys.perform(&Action::MediaNext), Ok(()));
        assert_eq!(
            sys.perform(&Action::BtConnect("bad".into(), true)),
            Err(ActionError::Invalid),
            "invalid address"
        );
    }

    /// Manual: `cargo test -p shell --lib live_host -- --ignored --nocapture` prints this machine's snapshot.
    #[test]
    #[ignore]
    fn live_host_snapshot() {
        let sys = CoreSystem::new();
        std::thread::sleep(std::time::Duration::from_secs(3));
        let s = sys.snapshot();
        println!("{s:#?}\nnetwork {:?}", sys.network());
        assert!(s.volume.is_some() || s.wifi_on.is_some());
    }
}
