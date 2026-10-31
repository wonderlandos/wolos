//! What a process is inside an app: Chromium/Electron process roles (`--type=`) or a native app.
//! Per-role starting ceilings follow memory-sentry's ratios, sized for Linux RSS accounting.

const MIB: u64 = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    /// The browser process of a Chromium/Electron app (no `--type=`).
    Main,
    Renderer,
    Gpu,
    /// Network, storage and other utility services.
    Utility,
    /// The audio service (`--utility-sub-type=audio.mojom.AudioService`).
    Audio,
    Zygote,
    Crashpad,
    /// Anything that is not part of a Chromium-style process tree.
    Native,
}

pub const ALL: [Role; 8] = [
    Role::Main,
    Role::Renderer,
    Role::Gpu,
    Role::Utility,
    Role::Audio,
    Role::Zygote,
    Role::Crashpad,
    Role::Native,
];

impl Role {
    /// Child cgroup name inside the app's scope.
    pub fn name(self) -> &'static str {
        match self {
            Role::Main => "main",
            Role::Renderer => "renderer",
            Role::Gpu => "gpu",
            Role::Utility => "utility",
            Role::Audio => "audio",
            Role::Zygote => "zygote",
            Role::Crashpad => "crashpad",
            Role::Native => "native",
        }
    }

    pub fn from_name(n: &str) -> Option<Role> {
        ALL.into_iter().find(|r| r.name() == n)
    }

    /// Per-process ceiling band (start, max) in bytes; the floor is the start. None: native apps
    /// have no fixed size and use [`native_ceiling`].
    pub fn band(self) -> Option<(u64, u64)> {
        let (start, max) = match self {
            Role::Main | Role::Audio => (48, 96),
            Role::Gpu | Role::Renderer => (40, 80),
            Role::Utility => (32, 64),
            Role::Zygote | Role::Crashpad => (16, 32),
            Role::Native => return None,
        };
        Some((start * MIB, max * MIB))
    }
}

/// Role of one process from its argv. `chromium_app`: some process of the same app carries a
/// `--type=` switch, so an untyped process is the browser process, not a native app.
pub fn role(argv: &[String], chromium_app: bool) -> Role {
    if argv.first().is_some_and(|a| {
        a.rsplit('/')
            .next()
            .unwrap_or(a)
            .contains("crashpad_handler")
    }) {
        return Role::Crashpad;
    }
    let switch = |name: &str| {
        argv.iter()
            .skip(1)
            .find_map(|a| a.strip_prefix(name)?.strip_prefix('='))
    };
    match switch("--type") {
        Some("renderer") => Role::Renderer,
        Some("gpu-process") => Role::Gpu,
        Some("zygote") => Role::Zygote,
        Some("crashpad-handler") => Role::Crashpad,
        Some("utility")
            if switch("--utility-sub-type").is_some_and(|s| s.starts_with("audio.mojom")) =>
        {
            Role::Audio
        }
        Some(_) => Role::Utility,
        None if chromium_app => Role::Main,
        None => Role::Native,
    }
}

/// True when this argv belongs to a Chromium-style child (has `--type=`).
pub fn is_chromium_child(argv: &[String]) -> bool {
    argv.iter().skip(1).any(|a| a.starts_with("--type="))
}

/// Native apps: half of what they hold when they go to the background, never below 64 MiB.
pub fn native_ceiling(current: u64) -> u64 {
    (current / 2).max(64 * MIB)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &str) -> Vec<String> {
        s.split(' ').map(String::from).collect()
    }

    #[test]
    fn roles_come_from_the_type_switch() {
        use Role::*;
        for (cmd, chromium, want) in [
            ("/opt/code/code --type=renderer --lang=en", true, Renderer),
            (
                "/usr/lib/chromium/chromium --type=gpu-process --x",
                true,
                Gpu,
            ),
            ("/usr/lib/chromium/chromium --type=zygote", true, Zygote),
            (
                "/usr/lib/chromium/chromium --type=zygote --no-zygote-sandbox",
                true,
                Zygote,
            ),
            (
                "/opt/Discord/Discord --type=utility --utility-sub-type=network.mojom.NetworkService",
                true,
                Utility,
            ),
            (
                "/opt/spotify/spotify --type=utility --utility-sub-type=audio.mojom.AudioService",
                true,
                Audio,
            ),
            (
                "/usr/lib/chromium/chrome_crashpad_handler --monitor-self",
                true,
                Crashpad,
            ),
            ("/opt/Slack/slack --type=crashpad-handler", true, Crashpad),
            ("/opt/Slack/slack --type=broker", true, Utility),
            ("/opt/Slack/slack --enable-crashpad", true, Main),
            ("/usr/bin/gimp", false, Native),
            // A path that merely contains the word is not a switch.
            ("/opt/type=renderer/app", false, Native),
        ] {
            assert_eq!(role(&argv(cmd), chromium), want, "{cmd}");
        }
    }

    #[test]
    fn chromium_children_are_recognised() {
        assert!(is_chromium_child(&argv("/x --type=renderer")));
        assert!(!is_chromium_child(&argv("/x --typed")));
        assert!(!is_chromium_child(&[]));
    }

    #[test]
    fn bands_rise_from_a_floor_and_names_round_trip() {
        for r in ALL {
            assert_eq!(Role::from_name(r.name()), Some(r));
            if let Some((start, max)) = r.band() {
                assert!(start > 0 && max >= start * 2, "{r:?}");
            }
        }
        assert_eq!(Role::Native.band(), None);
        assert_eq!(native_ceiling(10 * MIB), 64 * MIB);
        assert_eq!(native_ceiling(1000 * MIB), 500 * MIB);
    }
}
