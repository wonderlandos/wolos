//! `wol` command tree as data + parser (IaraOS Iara.Cli.Core). Compatibility: every invocation in the
//! desktop configs is a row in `compat_table`; never rename, add aliases.

/// One accepted positional argument.
#[derive(Clone, Copy, Debug)]
pub struct Arg {
    pub name: &'static str,
    /// Allowed values; empty = any.
    pub choices: &'static [&'static str],
    pub optional: bool,
    /// Takes every remaining positional.
    pub rest: bool,
}

const fn arg(name: &'static str) -> Arg {
    Arg {
        name,
        choices: &[],
        optional: false,
        rest: false,
    }
}
const fn opt(name: &'static str) -> Arg {
    Arg {
        name,
        choices: &[],
        optional: true,
        rest: false,
    }
}
const fn choice(name: &'static str, choices: &'static [&'static str]) -> Arg {
    Arg {
        name,
        choices,
        optional: false,
        rest: false,
    }
}
const fn rest(name: &'static str) -> Arg {
    Arg {
        name,
        choices: &[],
        optional: true,
        rest: true,
    }
}

/// One command: words, action id, args, flags (`--json`, `--yes` are global).
#[derive(Clone, Copy, Debug)]
pub struct Cmd {
    pub path: &'static [&'static str],
    pub action: &'static str,
    pub args: &'static [Arg],
    pub flags: &'static [&'static str],
    pub help: &'static str,
    pub hidden: bool,
}

const fn cmd(
    path: &'static [&'static str],
    action: &'static str,
    args: &'static [Arg],
    help: &'static str,
) -> Cmd {
    Cmd {
        path,
        action,
        args,
        flags: &[],
        help,
        hidden: false,
    }
}
const fn alias(path: &'static [&'static str], action: &'static str, args: &'static [Arg]) -> Cmd {
    Cmd {
        path,
        action,
        args,
        flags: &[],
        help: "",
        hidden: true,
    }
}

pub use shared::shell::PANELS;
/// `wol appearance set` modes.
pub const MODES: &[&str] = &["light", "dark", "auto"];
/// `wol power mode` (`wol_core::power::policy::Mode` names, test-checked).
pub const POWER_MODES: &[&str] = &["auto", "low", "high"];
/// `wol_core::theme::accent::ACCENTS` ids (test-checked).
pub const ACCENTS: &[&str] = &[
    "violet", "blue", "pink", "red", "orange", "yellow", "green", "graphite",
];
/// `wol_core::automation::CAPABILITIES` ids (test-checked).
pub const CAPABILITIES: &[&str] = &[
    "read",
    "appearance",
    "sound",
    "display",
    "network",
    "focus",
    "apps",
    "notify",
    "screen-capture",
    "window-list",
    "files",
    "restore",
];
pub const GLOBAL_FLAGS: &[&str] = &["json", "yes", "help"];

/// The tree. Longest matching path wins.
pub const TREE: &[Cmd] = &[
    cmd(&["about"], "about", &[], "OS, kernel, processor, memory"),
    cmd(&["jobs"], "jobs.list", &[], "Background jobs (wold)"),
    alias(&["jobs", "list"], "jobs.list", &[]),
    alias(&["jobs", "start"], "jobs.start", &[arg("label")]),
    alias(&["jobs", "stop"], "jobs.stop", &[arg("label")]),
    cmd(
        &["start"],
        "jobs.start",
        &[arg("label")],
        "Start a job (root)",
    ),
    cmd(&["stop"], "jobs.stop", &[arg("label")], "Stop a job (root)"),
    cmd(
        &["sys"],
        "sys",
        &[opt("topic")],
        "System info topic: about stats disks temperatures gpu hw updates",
    ),
    cmd(
        &["admin"],
        "admin",
        &[arg("action"), rest("args")],
        "One allow-listed administrator action (wol-admind; password on stdin)",
    ),
    cmd(
        &["run"],
        "bundle.run",
        &[arg("bundle"), rest("args")],
        "Run a .wol bundle (wol-run)",
    ),
    cmd(
        &["port"],
        "bundle.plan",
        &[arg("bundle")],
        "Show which target of a bundle runs on this host",
    ),
    alias(&["plan"], "bundle.plan", &[arg("bundle")]),
    cmd(&["theme"], "theme.current", &[], "Current theme id"),
    cmd(
        &["theme", "current"],
        "theme.current",
        &[],
        "Current theme id",
    ),
    cmd(&["theme", "list"], "theme.list", &[], "Installed themes"),
    cmd(&["theme", "set"], "theme.set", &[arg("id")], "Use a theme"),
    cmd(
        &["theme", "light"],
        "theme.switch",
        &[],
        "Use the light default theme",
    ),
    cmd(
        &["theme", "dark"],
        "theme.switch",
        &[],
        "Use the dark default theme",
    ),
    cmd(
        &["theme", "toggle"],
        "theme.switch",
        &[],
        "Switch light <-> dark",
    ),
    cmd(
        &["theme", "css"],
        "theme.css",
        &[opt("id")],
        "Theme as CSS variables",
    ),
    cmd(
        &["shell", "toggle"],
        "shell.toggle",
        &[choice("panel", PANELS)],
        "Toggle a shell panel",
    ),
    cmd(
        &["shell", "show"],
        "shell.show",
        &[choice("panel", PANELS)],
        "Show a shell panel",
    ),
    cmd(
        &["shell", "hide"],
        "shell.hide",
        &[choice("panel", PANELS)],
        "Hide a shell panel",
    ),
    cmd(
        &["shell", "reload"],
        "shell.reload",
        &[],
        "Reload the shell",
    ),
    cmd(
        &["update"],
        "update.check",
        &[],
        "List pending package upgrades",
    ),
    cmd(
        &["update", "check"],
        "update.check",
        &[],
        "List pending package upgrades",
    ),
    cmd(
        &["base", "verify"],
        "base.verify",
        &[],
        "Base files that changed or went missing (wolos-base manifest)",
    ),
    cmd(
        &["update", "status"],
        "update.status",
        &[],
        "What the daily check found: package fixes, security fixes, a new release",
    ),
    cmd(
        &["update", "plan"],
        "update.plan",
        &[opt("be"), opt("release")],
        "Commands an update would run (the other root slot)",
    ),
    Cmd {
        path: &["update", "apply"],
        action: "update.apply",
        args: &[opt("be"), opt("release")],
        flags: &[],
        help: "Update into the other root slot (root, --yes)",
        hidden: false,
    },
    cmd(
        &["update", "rollback"],
        "update.rollback",
        &[opt("be")],
        "Boot the given (or the previous) restore point next (administrator)",
    ),
    cmd(
        &["mirror", "status"],
        "mirror.status",
        &[],
        "Package mirrors, CPU level and whether [wolos-cachy] is on",
    ),
    cmd(
        &["mirror", "rank"],
        "mirror.rank",
        &[],
        "Probe the mirrors in parallel, fastest first (--yes writes the order, root)",
    ),
    cmd(
        &["mirror", "gate"],
        "mirror.gate",
        &[],
        "Turn [wolos-cachy] on only where the CPU runs x86-64-v3 builds (root, --yes)",
    ),
    cmd(
        &["mirror", "kernel"],
        "mirror.kernel",
        &[],
        "System kernel package for this CPU",
    ),
    cmd(
        &["gpu", "env"],
        "gpu.env",
        &[],
        "Session GPU environment (export lines)",
    ),
    cmd(
        &["gpu", "hyprconf"],
        "gpu.hyprconf",
        &[],
        "Hyprland render config fragment",
    ),
    // Hardware: drivers, firmware, quirks (wol_core::hardware, archicture/modules/hardware.md).
    cmd(
        &["hw"],
        "hw.list",
        &[],
        "Hardware: every device, its driver and status",
    ),
    cmd(
        &["hw", "list"],
        "hw.list",
        &[],
        "Every device, its driver and status",
    ),
    cmd(
        &["hw", "status"],
        "hw.status",
        &[],
        "Devices that need a driver or firmware, and the fix",
    ),
    cmd(
        &["hw", "fix"],
        "hw.fix",
        &[],
        "Load missing drivers (administrator); --yes also installs packages",
    ),
    Cmd {
        path: &["hw", "boot"],
        action: "hw.boot",
        args: &[],
        flags: &["online"],
        help: "Boot step: load drivers, write tunables (rc.d wolos_hw, root); --online installs missing packages first",
        hidden: false,
    },
    Cmd {
        path: &["hw", "online"],
        action: "hw.online",
        args: &[],
        flags: &[],
        help: "Network came up: install a driver first boot could not (dhclient hook, root)",
        hidden: true,
    },
    // Games (wol_core::games, archicture/modules/games.md).
    cmd(
        &["games"],
        "games.doctor",
        &[],
        "Games: what Steam and Windows games need, with the fix",
    ),
    cmd(
        &["games", "doctor"],
        "games.doctor",
        &[],
        "What Steam and Windows games need on this machine, with the fix",
    ),
    cmd(
        &["games", "steam"],
        "games.steam",
        &[opt("url")],
        "Start Steam (downloads it the first time); optional steam:// link",
    ),
    // Power (wol_core::power::policy, archicture/modules/power.md).
    cmd(
        &["power"],
        "power.status",
        &[],
        "Power: source, mode and what the policy sets",
    ),
    cmd(
        &["power", "status"],
        "power.status",
        &[],
        "Power source, mode, battery and what the policy sets",
    ),
    cmd(
        &["power", "mode"],
        "power.mode",
        &[choice("mode", POWER_MODES)],
        "auto follows the power source; low saves energy; high is fastest",
    ),
    cmd(
        &["power", "apply"],
        "power.apply",
        &[],
        "Apply the power policy now (root; devd on AC changes, rc.d at boot and resume)",
    ),
    cmd(&["power", "sleep"], "power.sleep", &[], "Sleep now"),
    cmd(&["power", "restart"], "power.restart", &[], "Restart now"),
    cmd(
        &["power", "shutdown"],
        "power.shutdown",
        &[],
        "Shut down now",
    ),
    // Firewall and Privacy & Security (wol_core::security, archicture/modules/security.md).
    cmd(
        &["firewall"],
        "firewall.status",
        &[],
        "Firewall: on or off (nothing comes in unasked)",
    ),
    cmd(
        &["firewall", "status"],
        "firewall.status",
        &[],
        "Firewall on at boot and filtering now",
    ),
    cmd(
        &["firewall", "on"],
        "firewall.on",
        &[],
        "Turn the firewall on",
    ),
    cmd(
        &["firewall", "off"],
        "firewall.off",
        &[],
        "Turn the firewall off",
    ),
    cmd(
        &["security"],
        "security.status",
        &[],
        "Firewall, remote login and kernel hardening",
    ),
    Cmd {
        path: &["power", "idle"],
        action: "power.idle",
        args: &[choice("step", &["display-off", "sleep"])],
        flags: &[],
        help: "hypridle: on battery, turn the display off or sleep",
        hidden: true,
    },
    Cmd {
        path: &["power", "suspend"],
        action: "power.suspend",
        args: &[],
        flags: &[],
        help: "rc.suspend: lock every session before the machine sleeps (root)",
        hidden: true,
    },
    // Desktop widgets (archicture/modules/widgets.md): the layout file and widget packages.
    cmd(
        &["widgets"],
        "widgets.list",
        &[],
        "Widgets on the desktop and the ones you can add",
    ),
    cmd(
        &["widgets", "list"],
        "widgets.list",
        &[],
        "Widgets on the desktop and the ones you can add",
    ),
    cmd(
        &["widgets", "add"],
        "widgets.add",
        &[arg("kind"), opt("size")],
        "Put a widget on the desktop (clock, weather, a package id; small|medium|large)",
    ),
    cmd(
        &["widgets", "remove"],
        "widgets.remove",
        &[arg("id")],
        "Take a widget off the desktop by its id",
    ),
    cmd(
        &["widgets", "reload"],
        "widgets.reload",
        &[],
        "Reread widgets.json and the widget packages",
    ),
    cmd(
        &["widgets", "validate"],
        "widgets.validate",
        &[arg("package")],
        "Check a widget package (id or folder): schema, source, every binding",
    ),
    // One interface for people, scripts and helpers (archicture/modules/automation.md).
    cmd(
        &["api"],
        "api",
        &[],
        "Every command: arguments, access, approval",
    ),
    cmd(
        &["mcp"],
        "mcp",
        &[],
        "Model Context Protocol server on stdin/stdout",
    ),
    cmd(
        &["theme", "get"],
        "theme.get",
        &[],
        "Current theme, appearance and accent",
    ),
    cmd(
        &["appearance"],
        "appearance.get",
        &[],
        "Appearance, accent, theme and wallpaper",
    ),
    cmd(
        &["appearance", "set"],
        "appearance.set",
        &[choice("mode", MODES)],
        "Light, Dark or Auto (keeps the accent)",
    ),
    cmd(
        &["accent", "set"],
        "accent.set",
        &[choice("accent", ACCENTS)],
        "Accent colour (keeps the appearance)",
    ),
    cmd(&["wallpaper"], "wallpaper.get", &[], "Current wallpaper"),
    cmd(
        &["wallpaper", "set"],
        "wallpaper.set",
        &[arg("path")],
        "Use an image file as wallpaper",
    ),
    cmd(&["audio"], "audio.get", &[], "Volume, mute and outputs"),
    cmd(
        &["audio", "volume"],
        "audio.volume",
        &[arg("percent")],
        "Set the volume, 0-100",
    ),
    cmd(&["audio", "mute"], "audio.mute", &[], "Mute the sound"),
    cmd(
        &["audio", "unmute"],
        "audio.unmute",
        &[],
        "Unmute the sound",
    ),
    cmd(&["display"], "display.get", &[], "Brightness and displays"),
    cmd(
        &["display", "brightness"],
        "display.brightness",
        &[arg("percent")],
        "Set the brightness, 1-100",
    ),
    cmd(&["battery"], "battery", &[], "Battery charge and health"),
    cmd(
        &["network"],
        "network",
        &[],
        "Online, addresses, Wi-Fi network, Bluetooth",
    ),
    cmd(
        &["wifi"],
        "wifi.get",
        &[],
        "Wi-Fi state and nearby networks",
    ),
    cmd(
        &["wifi", "on"],
        "wifi.on",
        &[],
        "Turn Wi-Fi on (administrator)",
    ),
    cmd(
        &["wifi", "off"],
        "wifi.off",
        &[],
        "Turn Wi-Fi off (administrator)",
    ),
    cmd(
        &["wifi", "join"],
        "wifi.join",
        &[arg("ssid")],
        "Join a network; Wi-Fi password on stdin (administrator)",
    ),
    cmd(&["bluetooth"], "bluetooth.get", &[], "Bluetooth state"),
    cmd(
        &["bluetooth", "on"],
        "bluetooth.on",
        &[],
        "Turn Bluetooth on (administrator)",
    ),
    cmd(
        &["bluetooth", "off"],
        "bluetooth.off",
        &[],
        "Turn Bluetooth off (administrator)",
    ),
    cmd(&["focus"], "focus.get", &[], "Do Not Disturb state"),
    cmd(&["focus", "on"], "focus.on", &[], "Turn Do Not Disturb on"),
    cmd(
        &["focus", "off"],
        "focus.off",
        &[],
        "Turn Do Not Disturb off",
    ),
    cmd(&["settings"], "settings", &[], "Your desktop settings"),
    cmd(
        &["open"],
        "open",
        &[arg("target")],
        "Open an app (id or name), a file or an http/https/mailto link",
    ),
    cmd(
        &["notify"],
        "notify",
        &[arg("title"), rest("body")],
        "Show a notification",
    ),
    cmd(
        &["screenshot"],
        "screenshot",
        &[],
        "Screenshot of the whole screen; prints the file",
    ),
    cmd(&["windows"], "windows.list", &[], "Open windows"),
    cmd(
        &["windows", "focus"],
        "windows.focus",
        &[arg("window")],
        "Bring a window forward (address or app id)",
    ),
    cmd(
        &["files", "tidy"],
        "files.tidy",
        &[opt("plan")],
        "Sort Downloads into folders: dry run, then apply its plan id",
    ),
    cmd(&["history"], "history", &[], "Recent settings changes"),
    cmd(
        &["undo"],
        "undo",
        &[opt("id")],
        "Undo the last change, or change <id>",
    ),
    alias(&["restore"], "restore.list", &[]),
    cmd(
        &["restore", "list"],
        "restore.list",
        &[],
        "Restore points (ZFS boot environments)",
    ),
    cmd(
        &["restore", "activate"],
        "restore.activate",
        &[arg("be")],
        "Start from a restore point next boot (administrator)",
    ),
    cmd(
        &["helpers"],
        "helpers.status",
        &[],
        "Helpers: on or off, capabilities, who connected",
    ),
    cmd(
        &["helpers", "on"],
        "helpers.on",
        &[],
        "Let helpers use WolOS",
    ),
    cmd(
        &["helpers", "off"],
        "helpers.off",
        &[],
        "Turn every helper away",
    ),
    cmd(
        &["helpers", "allow"],
        "helpers.allow",
        &[choice("capability", CAPABILITIES)],
        "Allow helpers one capability",
    ),
    cmd(
        &["helpers", "deny"],
        "helpers.deny",
        &[choice("capability", CAPABILITIES)],
        "Deny helpers one capability",
    ),
    cmd(
        &["helpers", "forget"],
        "helpers.forget",
        &[arg("helper")],
        "Take back every Always Allow of one helper",
    ),
    cmd(
        &["helpers", "log"],
        "helpers.log",
        &[],
        "What helpers did, newest first",
    ),
    // Apps: .wol packages, the App Store catalogue, pkg (archicture/modules/packages.md).
    cmd(
        &["install"],
        "package.install",
        &[arg("app")],
        "Install a .wol file, an App Store app or a package (administrator)",
    ),
    cmd(
        &["remove"],
        "package.remove",
        &[arg("app")],
        "Remove an app installed from a .wol (administrator)",
    ),
    cmd(
        &["list"],
        "package.list",
        &[],
        "Apps installed from .wol packages",
    ),
    cmd(
        &["search"],
        "package.search",
        &[rest("term")],
        "Search the App Store and the .wol index",
    ),
    cmd(
        &["update", "apps"],
        "package.updates",
        &[],
        "Newer versions of .wol apps; --yes installs them (administrator)",
    ),
    cmd(&["help"], "help", &[rest("command")], "Show help"),
];

/// Parsed invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    pub path: Vec<String>,
    pub action: &'static str,
    pub args: Vec<(&'static str, String)>,
    pub flags: Vec<String>,
}

impl Invocation {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.args.iter().find(|a| a.0 == name).map(|a| a.1.as_str())
    }
    /// Values bound to a `rest` arg (and any repeated name).
    pub fn all(&self, name: &str) -> Vec<&str> {
        self.args
            .iter()
            .filter(|a| a.0 == name)
            .map(|a| a.1.as_str())
            .collect()
    }
    pub fn flag(&self, f: &str) -> bool {
        self.flags.iter().any(|x| x == f)
    }
    /// Stable text form used by compat tests: `action name=value ... --flag`.
    pub fn describe(&self) -> String {
        let mut s = self.action.to_string();
        for (k, v) in &self.args {
            s.push_str(&format!(" {k}={v}"));
        }
        for f in &self.flags {
            s.push_str(&format!(" --{f}"));
        }
        s
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    Unknown {
        word: String,
        suggestion: Option<String>,
    },
    Missing(&'static str),
    BadChoice {
        arg: &'static str,
        value: String,
        choices: &'static [&'static str],
    },
    Extra(String),
    BadFlag(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown {
                word,
                suggestion: Some(s),
            } => write!(f, "unknown command '{word}'; did you mean '{s}'?"),
            Self::Unknown {
                word,
                suggestion: None,
            } => write!(f, "unknown command '{word}'"),
            Self::Missing(a) => write!(f, "missing <{a}>"),
            Self::BadChoice {
                arg,
                value,
                choices,
            } => write!(
                f,
                "<{arg}> must be one of {}, got '{value}'",
                choices.join(", ")
            ),
            Self::Extra(w) => write!(f, "unexpected argument '{w}'"),
            Self::BadFlag(x) => write!(f, "unknown flag --{x}"),
        }
    }
}

pub fn parse<S: AsRef<str>>(argv: &[S]) -> Result<Invocation, ParseError> {
    let argv: Vec<&str> = argv.iter().map(AsRef::as_ref).collect();
    let mut words = Vec::new();
    let mut flags = Vec::new();
    let mut passthrough = false;
    for a in &argv {
        if passthrough {
            words.push(*a);
        } else if *a == "--" {
            passthrough = true;
        } else if let Some(f) = a.strip_prefix("--") {
            flags.push(f.to_string());
        } else {
            words.push(*a);
        }
    }
    if words.is_empty() {
        return Ok(Invocation {
            path: vec![],
            action: "help",
            args: vec![],
            flags,
        });
    }
    let best = TREE
        .iter()
        .filter(|c| c.path.len() <= words.len() && c.path.iter().zip(&words).all(|(p, w)| p == w))
        .max_by_key(|c| c.path.len());
    let Some(c) = best else {
        let roots: Vec<&str> = TREE
            .iter()
            .filter(|c| !c.hidden)
            .map(|c| c.path[0])
            .collect();
        return Err(ParseError::Unknown {
            word: words[0].into(),
            suggestion: suggest(words[0], &roots),
        });
    };
    for f in &flags {
        if !GLOBAL_FLAGS.contains(&f.as_str()) && !c.flags.contains(&f.as_str()) {
            return Err(ParseError::BadFlag(f.clone()));
        }
    }
    let mut rest_words = words[c.path.len()..].iter();
    let mut args = Vec::new();
    for a in c.args {
        if a.rest {
            args.extend(rest_words.by_ref().map(|w| (a.name, w.to_string())));
            break;
        }
        match rest_words.next() {
            Some(w) => {
                if !a.choices.is_empty() && !a.choices.contains(w) {
                    return Err(ParseError::BadChoice {
                        arg: a.name,
                        value: w.to_string(),
                        choices: a.choices,
                    });
                }
                args.push((a.name, w.to_string()));
            }
            None if a.optional => break,
            None => return Err(ParseError::Missing(a.name)),
        }
    }
    if let Some(w) = rest_words.next() {
        // a word right after a group (e.g. `wol theme bogus`) is an unknown subcommand
        if c.args.is_empty() {
            let subs: Vec<&str> = TREE
                .iter()
                .filter(|x| x.path.len() == c.path.len() + 1 && x.path.starts_with(c.path))
                .map(|x| x.path[c.path.len()])
                .collect();
            if !subs.is_empty() {
                return Err(ParseError::Unknown {
                    word: w.to_string(),
                    suggestion: suggest(w, &subs),
                });
            }
        }
        return Err(ParseError::Extra(w.to_string()));
    }
    Ok(Invocation {
        path: c.path.iter().map(|s| s.to_string()).collect(),
        action: c.action,
        args,
        flags,
    })
}

/// Closest candidate within edit distance 2 (did-you-mean).
pub fn suggest(word: &str, candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .map(|c| (distance(word, c), *c))
        .filter(|(d, _)| *d <= 2)
        .min()
        .map(|(_, c)| c.to_string())
}

fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push(
                (prev[j] + usize::from(ca != *cb))
                    .min(prev[j + 1] + 1)
                    .min(cur[j] + 1),
            );
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Help text for the whole tree or one command path.
pub fn help(prefix: &[String]) -> String {
    let mut out = String::from("usage: wol <command> [args] [--json] [--yes]\n\n");
    for c in TREE
        .iter()
        .filter(|c| !c.hidden && c.path.iter().zip(prefix).all(|(p, w)| p == w))
    {
        let mut usage = c.path.join(" ");
        for a in c.args {
            let n = if a.choices.is_empty() {
                a.name.to_string()
            } else {
                a.choices.join("|")
            };
            usage.push_str(&match (a.optional, a.rest) {
                (_, true) => format!(" [{n}...]"),
                (true, _) => format!(" [{n}]"),
                _ => format!(" <{n}>"),
            });
        }
        for f in c.flags {
            usage.push_str(&format!(" [--{f}]"));
        }
        out.push_str(&format!("  {usage:<44} {}\n", c.help));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(line: &str) -> String {
        let argv: Vec<&str> = line.split_whitespace().collect();
        parse(&argv)
            .map(|i| i.describe())
            .unwrap_or_else(|e| format!("error: {e}"))
    }

    /// Every `wol ...` invocation in desktop/ configs, keybinds and docs keeps its action.
    #[test]
    fn compat_table() {
        let table = [
            // desktop/ (hyprland binds, waybar, menus)
            ("theme light", "theme.switch"),
            ("theme dark", "theme.switch"),
            ("theme toggle", "theme.switch"),
            ("shell toggle switcher", "shell.toggle panel=switcher"),
            (
                "shell toggle notifications",
                "shell.toggle panel=notifications",
            ),
            ("shell toggle session", "shell.toggle panel=session"),
            ("shell show session", "shell.show panel=session"),
            ("shell toggle clipboard", "shell.toggle panel=clipboard"),
            ("shell hide osd", "shell.hide panel=osd"),
            ("shell reload", "shell.reload"),
            // IaraOS-compatible spellings
            ("theme", "theme.current"),
            ("theme list", "theme.list"),
            ("theme list --json", "theme.list --json"),
            ("theme set wol-dark", "theme.set id=wol-dark"),
            ("jobs", "jobs.list"),
            ("jobs --json", "jobs.list --json"),
            ("jobs list", "jobs.list"),
            ("jobs start x", "jobs.start label=x"),
            ("start org.wol.shell", "jobs.start label=org.wol.shell"),
            ("stop org.wol.shell", "jobs.stop label=org.wol.shell"),
            ("sys stats --json", "sys topic=stats --json"),
            ("sys about", "sys topic=about"),
            ("sys", "sys"),
            ("about", "about"),
            ("run Game.wol", "bundle.run bundle=Game.wol"),
            (
                "run Game.wol -- -x y",
                "bundle.run bundle=Game.wol args=-x args=y",
            ),
            ("plan Game.wol --json", "bundle.plan bundle=Game.wol --json"),
            ("port Game.wol", "bundle.plan bundle=Game.wol"),
            ("update check", "update.check"),
            ("update apply", "update.apply"),
            (
                "update apply wolos-b 2026/11/02 --yes",
                "update.apply be=wolos-b release=2026/11/02 --yes",
            ),
            ("update plan", "update.plan"),
            ("admin revoke", "admin action=revoke"),
            ("admin idle 300", "admin action=idle args=300"),
            ("gpu env", "gpu.env"),
            ("update rollback", "update.rollback"),
            ("update rollback wol-1", "update.rollback be=wol-1"),
            ("restore", "restore.list"),
            (
                "appearance set dark --json",
                "appearance.set mode=dark --json",
            ),
            ("notify -- Hi --there", "notify title=Hi body=--there"),
            ("gpu hyprconf", "gpu.hyprconf"),
            // rc.d wolos_hw, Settings, scripts
            ("hw boot", "hw.boot"),
            ("hw", "hw.list"),
            ("hw list --json", "hw.list --json"),
            ("hw status --json", "hw.status --json"),
            ("hw fix --yes", "hw.fix --yes"),
            // steam.desktop, the App Store Steam entry, scripts
            ("games steam", "games.steam"),
            (
                "games steam steam://install/3658110",
                "games.steam url=steam://install/3658110",
            ),
            ("games doctor --json", "games.doctor --json"),
            ("games", "games.doctor"),
            ("", "help"),
        ];
        for (line, want) in table {
            assert_eq!(d(line), want, "wol {line}");
        }
    }

    #[test]
    fn errors_table() {
        // The panel list grows with the shell; the message always names all of them.
        let panels = format!(
            "error: <panel> must be one of {}, got 'nope'",
            PANELS.join(", ")
        );
        for (line, want) in [
            (
                "thme light",
                "error: unknown command 'thme'; did you mean 'theme'?",
            ),
            ("zzzzzz", "error: unknown command 'zzzzzz'"),
            (
                "theme toggel",
                "error: unknown command 'toggel'; did you mean 'toggle'?",
            ),
            ("start", "error: missing <label>"),
            ("shell toggle nope", panels.as_str()),
            ("about extra", "error: unexpected argument 'extra'"),
            ("about --bogus", "error: unknown flag --bogus"),
            ("theme light --base", "error: unknown flag --base"),
        ] {
            assert_eq!(d(line), want, "wol {line}");
        }
    }

    #[test]
    fn choice_lists_match_core() {
        let accents: Vec<&str> = wol_core::theme::accent::ACCENTS
            .iter()
            .map(|a| a.0)
            .collect();
        assert_eq!(ACCENTS, accents.as_slice());
        let caps: Vec<&str> = wol_core::automation::CAPABILITIES
            .iter()
            .map(|c| c.0)
            .collect();
        assert_eq!(CAPABILITIES, caps.as_slice());
    }

    #[test]
    fn help_lists_visible_commands() {
        let h = help(&[]);
        assert!(h.contains("shell toggle <switcher|notifications"));
        assert!(!h.contains("jobs list"), "aliases are hidden");
        let h = help(&["update".into()]);
        assert!(h.contains("update apply [be] [release]") && !h.contains("theme"));
    }

    /// `.wol` and App Store verbs (ADR 0011).
    #[test]
    fn package_commands() {
        for (line, want) in [
            ("install Raider.wol", "package.install app=Raider.wol"),
            ("install gimp", "package.install app=gimp"),
            ("remove raider", "package.remove app=raider"),
            ("list --json", "package.list --json"),
            (
                "search photo editor",
                "package.search term=photo term=editor",
            ),
            ("update apps --yes", "package.updates --yes"),
            ("update", "update.check"),
        ] {
            assert_eq!(d(line), want, "wol {line}");
        }
        assert_eq!(d("install"), "error: missing <app>");
    }
}
