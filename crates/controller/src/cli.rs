// SPDX-License-Identifier: GPL-2.0-or-later
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
    "modules",
    "maintenance",
];
pub const GLOBAL_FLAGS: &[&str] = &["json", "yes", "help"];
/// `wol toggle` words: `wol_core::toggles::words()` (test-checked).
pub const TOGGLES: &[&str] = &[
    "nightlight",
    "night-light",
    "nightshift",
    "night-shift",
    "stay-awake",
    "idle",
    "awake",
    "caffeine",
    "silence",
    "dnd",
    "focus",
];
const ONE_TOGGLE: Arg = Arg {
    name: "toggle",
    choices: TOGGLES,
    optional: true,
    rest: false,
};
/// `wol audio profile`: the word is optional (none reads the profile, like `status`).
const AUDIO_PROFILE: Arg = Arg {
    name: "profile",
    choices: &["default", "low-latency", "status"],
    optional: true,
    rest: false,
};
const ON_OFF: Arg = Arg {
    name: "state",
    choices: &["on", "off"],
    optional: true,
    rest: false,
};

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
        &["script"],
        "script.kinds",
        &[],
        "Script kinds wol checks and runs: sh, bash, ruby (ADR 0041)",
    ),
    cmd(
        &["script", "check"],
        "script.check",
        &[arg("file")],
        "Check a script without running it: it parses, and its linter is clean when installed",
    ),
    cmd(
        &["script", "run"],
        "script.run",
        &[arg("file"), rest("args")],
        "Run a sh, bash, ruby or rsh script as you, clean environment, its wol changes through the helper gate",
    ),
    cmd(
        &["script", "on"],
        "script.on",
        &[arg("host"), arg("file"), rest("args")],
        "Run a local script on another computer over ssh, the same way (the file goes on stdin)",
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
        &["theme", "apply"],
        "theme.apply",
        &[],
        "Restyle apps for the theme in use, saving nothing",
    ),
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
    // Plugin IPC (docs/modules/plugins.md "[[ipc]]"): one wire line, one JSON reply line.
    Cmd {
        path: &["shell", "call"],
        action: "shell.call",
        args: &[arg("plugin"), arg("function"), rest("json")],
        flags: &[],
        help: "Run a plugin's [[ipc]] function with one JSON value and print its JSON answer",
        hidden: false,
    },
    cmd(
        &["shell", "status"],
        "shell.status",
        &[],
        "Shell surfaces and panels, and which are open",
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
        &["update", "channel"],
        "update.channel",
        &[],
        "The update channel this computer follows (stable)",
    ),
    cmd(
        &["update", "status"],
        "update.status",
        &[],
        "What the daily check found: package fixes, security fixes, a new release",
    ),
    Cmd {
        path: &["update", "chroot"],
        action: "update.chroot",
        args: &[arg("root"), rest("args")],
        flags: &[],
        help: "Slot build: run a command in the copy at ROOT, its mounts private to it (root)",
        hidden: true,
    },
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
        &["update", "install"],
        "update.install",
        &[],
        "Install the waiting package updates, like Update Now (administrator password)",
    ),
    cmd(
        &["update", "rollback"],
        "update.rollback",
        &[opt("be")],
        "Boot the given (or the previous) restore point next (administrator)",
    ),
    cmd(
        &["encrypt", "rebind"],
        "encrypt.rebind",
        &[],
        "First boot: bind the disk key to this computer's firmware state (root)",
    ),
    cmd(
        &["encrypt", "reseal"],
        "encrypt.reseal",
        &[],
        "After a start the chip refused: bind the disk key to the firmware state again (root)",
    ),
    cmd(
        &["encrypt", "password"],
        "encrypt.password",
        &[opt("a")],
        "wol-admind: USER's new password (stdin) also opens the disk (root)",
    ),
    cmd(
        &["boot", "enroll"],
        "boot.enroll",
        &[],
        "Queue this computer's key for the firmware's key screen (root)",
    ),
    cmd(
        &["boot", "password"],
        "boot.password",
        &[],
        "Installer: the word typed on the firmware's key screen",
    ),
    cmd(
        &["boot", "detect"],
        "boot.detect",
        &[opt("want")],
        "Installer: the protection tier this firmware allows (yes|no: protection wanted)",
    ),
    cmd(
        &["boot", "key-screen"],
        "boot.key_screen",
        &[opt("tier")],
        "Installer: does this tier need the firmware's key screen (yes|no)",
    ),
    cmd(
        &["boot", "words"],
        "boot.words",
        &[opt("tier")],
        "Installer: kernel command line words of a tier",
    ),
    cmd(
        &["boot", "note"],
        "boot.note",
        &[opt("tier"), opt("screen")],
        "Installer: what to tell the person before the restart",
    ),
    cmd(
        &["boot", "status"],
        "boot.status",
        &[],
        "Secure Boot, TPM, lockdown and seal state as JSON",
    ),
    cmd(
        &["boot", "check"],
        "boot.check",
        &[],
        "Boot check: refuse a kernel older than the slots kept",
    ),
    cmd(
        &["boot", "bless"],
        "boot.bless",
        &[],
        "After a good boot: raise the rollback floor and name the previous version",
    ),
    cmd(
        &["seal", "apply"],
        "seal.apply",
        &[opt("root"), opt("digestfile")],
        "Make a system slot read-only and record fs-verity digests (root)",
    ),
    cmd(
        &["seal", "check"],
        "seal.check",
        &[],
        "Boot check: the running slot matches what was sealed, else the other slot starts",
    ),
    cmd(
        &["seal", "off"],
        "seal.off",
        &[],
        "Developer mode: make the system writable again and stop checking it (root)",
    ),
    cmd(
        &["layers", "list"],
        "layers.list",
        &[],
        "Live layers: the packages installed since the last update, their size and date",
    ),
    cmd(
        &["layers", "fold"],
        "layers.fold",
        &[opt("a"), opt("b"), opt("c")],
        "Update: copy live-installed packages into the new slot (root)",
    ),
    cmd(
        &["layers", "clear"],
        "layers.clear",
        &[],
        "Boot: remove the live layers this slot already holds (root)",
    ),
    cmd(
        &["pkg"],
        "pkg",
        &[rest("args")],
        "pacman for the sealed system: wol pkg -- -S name (installs are live at once)",
    ),
    cmd(
        &["encrypt", "decide"],
        "encrypt.decide",
        &[opt("a")],
        "Disk encryption for the installer: off, tpm2 or passphrase",
    ),
    cmd(
        &["encrypt", "enroll"],
        "encrypt.enroll",
        &[opt("a"), opt("b"), opt("c"), opt("d")],
        "Installer: enroll TPM2 or passphrase and a recovery key (root)",
    ),
    cmd(
        &["encrypt", "words"],
        "encrypt.words",
        &[opt("a"), opt("b")],
        "Installer: kernel command line words that unlock the disk",
    ),
    cmd(
        &["encrypt", "hooks"],
        "encrypt.hooks",
        &[opt("a")],
        "Installer: add the unlock hook to an mkinitcpio config",
    ),
    cmd(
        &["install", "info"],
        "install.info",
        &[opt("plan")],
        "Installer: a plan's disk and encryption as shell assignments (root)",
    ),
    cmd(
        &["install", "probe"],
        "install.probe",
        &[opt("disk")],
        "Installer: what is on a disk, as JSON; reads only (root)",
    ),
    cmd(
        &["install", "check"],
        "install.check",
        &[opt("plan")],
        "Installer: check a plan against the disk as it is now; writes nothing (root)",
    ),
    cmd(
        &["install", "layout"],
        "install.layout",
        &[opt("plan"), opt("key")],
        "Installer: partition and format as the plan says; prints where things went (root)",
    ),
    cmd(
        &["install", "settings"],
        "install.settings",
        &[opt("plan"), opt("root")],
        "Installer: language, keyboard and time zone of the new system (root)",
    ),
    cmd(
        &["install", "account"],
        "install.account",
        &[opt("plan"), opt("root"), opt("passfile")],
        "Installer: create the account in the new system (root)",
    ),
    cmd(
        &["install", "facts"],
        "install.facts",
        &[opt("disk")],
        "Installer: a disk's facts as lines: size <bytes>, os <family>, bitlocker yes|no; reads only",
    ),
    cmd(
        &["install", "mounts"],
        "install.mounts",
        &[opt("disk")],
        "Installer: what of a disk is in use, as lines: mount <path> (deepest first), swap <device>",
    ),
    cmd(
        &["install", "hostname"],
        "install.hostname",
        &[opt("name")],
        "Installer: the computer name if valid (settings::hostname), else wolos",
    ),
    cmd(
        &["mirror", "status"],
        "mirror.status",
        &[],
        "Package mirrors, CPU level and whether the [cachyos-*-v3] sections are on",
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
        "Turn the [cachyos-*-v3] sections on only where the CPU runs x86-64-v3 builds (root, --yes)",
    ),
    cmd(
        &["mirror", "kernel"],
        "mirror.kernel",
        &[],
        "System kernel package for this CPU",
    ),
    // The keyring (ADR 0054): the person's own secrets, so the verbs stay out of the helper
    // catalogue. Everything after the verb goes to wol-keyring as typed; a secret or a password is
    // read from the terminal or stdin there, never from an argument.
    Cmd {
        path: &["keyring"],
        action: "keyring",
        args: &[opt("verb"), rest("args")],
        flags: &["passphrase"],
        help: "Keyring: status init id list set get delete export import move audit acl enrol compact (no verb: usage)",
        hidden: false,
    },
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
        "compositor render config fragment",
    ),
    // Hardware: drivers, firmware, quirks (wol_core::hardware, docs/modules/hardware.md).
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
        &["hw", "compute"],
        "hw.compute",
        &[],
        "GPU compute for local AI: backend (cuda, rocm, vulkan or cpu), VRAM and the GPU",
    ),
    cmd(
        &["hw", "report"],
        "hw.report",
        &[],
        "The models of this computer's devices for the public hardware list (JSON, no personal data)",
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
        help: "Boot step: load drivers, write tunables (wolos-hw.service, root); --online installs missing packages first",
        hidden: false,
    },
    Cmd {
        path: &["hw", "online"],
        action: "hw.online",
        args: &[],
        flags: &[],
        help: "Network came up: install a driver first boot could not (NetworkManager hook, root)",
        hidden: true,
    },
    // Optional modules (wol_core::modules, docs/modules/modules.md, ADR 0032).
    cmd(
        &["modules"],
        "modules.list",
        &[],
        "Optional modules: AI, developer, creative and console stacks, and their state",
    ),
    cmd(
        &["modules", "list"],
        "modules.list",
        &[],
        "Every optional module: installed, enabled, pending or available",
    ),
    cmd(
        &["modules", "status"],
        "modules.status",
        &[opt("id")],
        "State of one module (or all): installed, enabled, pending, what it needs",
    ),
    cmd(
        &["modules", "install"],
        "modules.install",
        &[arg("id")],
        "Install a module's packages (administrator); then wol modules enable <id>",
    ),
    cmd(
        &["modules", "remove"],
        "modules.remove",
        &[arg("id")],
        "Remove a module's packages (administrator) and its user-side parts",
    ),
    cmd(
        &["modules", "enable"],
        "modules.enable",
        &[arg("id")],
        "Finish a module's setup for this user (recipes, mise tools, post step); repeatable",
    ),
    // Sessions (wol_core::sessions, ADR 0032): terminal and agent windows reopened at login.
    cmd(
        &["sessions"],
        "sessions.list",
        &[],
        "Terminal and agent windows saved for the next login",
    ),
    cmd(
        &["sessions", "list"],
        "sessions.list",
        &[],
        "Terminal and agent windows saved for the next login",
    ),
    cmd(
        &["sessions", "save"],
        "sessions.save",
        &[],
        "Save this session's terminal and agent windows (workspace, folder, zellij, claude --continue)",
    ),
    cmd(
        &["sessions", "restore"],
        "sessions.restore",
        &[],
        "Reopen the saved windows in their workspaces",
    ),
    // Doctor: every area in plain words, each with its fix (docs/modules/doctor.md).
    cmd(
        &["doctor"],
        "doctor.check",
        &[Arg {
            name: "area",
            choices: crate::doctor::AREAS,
            optional: true,
            rest: false,
        }],
        "Check sound, games, updates, security, storage, network, power and displays, with the fix",
    ),
    cmd(
        &["doctor", "fix"],
        "doctor.fix",
        &[arg("id")],
        "Run one check's fix (wol-admind when it needs the administrator), then check again",
    ),
    // Games (wol_core::games, docs/modules/games.md).
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
    // Power (wol_core::power::policy, docs/modules/power.md).
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
        "Apply the power policy now (root; runs by itself when the charger is plugged or unplugged, at boot and after sleep)",
    ),
    cmd(&["power", "sleep"], "power.sleep", &[], "Sleep now"),
    cmd(&["power", "restart"], "power.restart", &[], "Restart now"),
    cmd(
        &["power", "shutdown"],
        "power.shutdown",
        &[],
        "Shut down now",
    ),
    // Firewall and Privacy & Security (wol_core::security, docs/modules/security.md).
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
        "Firewall, remote login, kernel hardening and the malware check",
    ),
    cmd(
        &["security", "scan"],
        "security.scan",
        &[arg("path")],
        "Check a file, and what is packed inside it, against known malware",
    ),
    cmd(
        &["security", "allow"],
        "security.allow",
        &[arg("path")],
        "Open Anyway: let a file the malware check stopped run on this computer",
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
    // Desktop widgets (docs/modules/widgets.md): the layout file and widget packages.
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
    // Shell plugins (ADR 0031, docs/modules/plugins.md): folders with a plugin.toml.
    cmd(
        &["plugin"],
        "plugin.list",
        &[],
        "Shell plugins: installed, on or off, problems",
    ),
    cmd(
        &["plugin", "list"],
        "plugin.list",
        &[],
        "Shell plugins: installed, on or off, problems",
    ),
    cmd(
        &["plugin", "new"],
        "plugin.new",
        &[arg("id"), opt("kinds")],
        "Scaffold a plugin that passes check (kinds: menubar,widget,panel,search,dock,tile,toggle,hotkey,webapp,theme)",
    ),
    cmd(
        &["plugin", "check"],
        "plugin.check",
        &[arg("plugin")],
        "Check a plugin (id or folder): manifest, files, Lua loads and renders, theme, compositor lines",
    ),
    cmd(
        &["plugin", "run"],
        "plugin.run",
        &[
            arg("plugin"),
            choice("kind", &["menubar", "widget", "panel", "search"]),
            opt("entry"),
            opt("input"),
        ],
        "Render one entry headless as JSON (input: a button number to click first, or the search text)",
    ),
    cmd(
        &["plugin", "enable"],
        "plugin.enable",
        &[arg("id")],
        "Turn a plugin on and grant the permissions it declares",
    ),
    cmd(
        &["plugin", "allow-host"],
        "plugin.allow-host",
        &[arg("id"), opt("host")],
        "Let a turned-on plugin read data from one web site (a host name; none: list the sites it may read)",
    ),
    cmd(
        &["plugin", "disable"],
        "plugin.disable",
        &[arg("id")],
        "Turn a plugin off",
    ),
    cmd(
        &["plugin", "remove"],
        "plugin.remove",
        &[arg("id")],
        "Delete a user plugin folder and its state",
    ),
    cmd(
        &["plugin", "add"],
        "plugin.add",
        &[arg("plugin")],
        "Install a .wolplug file or a plugin folder for this account (unsigned: --yes); turn it on with enable",
    ),
    cmd(
        &["plugin", "pack"],
        "plugin.pack",
        &[arg("plugin"), opt("out")],
        "Make <id>-<version>.wolplug (zstd tar of the folder with its SHA256 list) next to it or at out",
    ),
    // One interface for people, scripts and helpers (docs/modules/automation.md).
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
    Cmd {
        path: &["webapp-call"],
        action: "webapp.call",
        args: &[arg("app"), arg("method"), opt("params")],
        flags: &[],
        help: "wold: one bridge call of a system web app (ADR 0048)",
        hidden: true,
    },
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
    // ADR 0038 names `wol volume`: the volume as the keys and the OSD see it (`wol volume`), or set it
    // (`wol volume 40`). The same action as `wol audio volume`; without a percent it reads.
    alias(&["volume"], "audio.volume", &[opt("percent")]),
    cmd(&["audio", "mute"], "audio.mute", &[], "Mute the sound"),
    cmd(
        &["audio", "unmute"],
        "audio.unmute",
        &[],
        "Unmute the sound",
    ),
    // Sound stack (crates/audio, docs/modules/audio.md, docs/creators-audio.md).
    cmd(
        &["audio", "doctor"],
        "audio.doctor",
        &[],
        "Sound check: services, inputs above 0 dB, buffers, dropouts, OBS pieces, with the fix",
    ),
    cmd(
        &["audio", "reset"],
        "audio.reset",
        &[],
        "Forget stored sound defaults and volumes, drop Pro Audio overrides, restart PipeWire (dry run; --yes does it)",
    ),
    cmd(
        &["audio", "profile"],
        "audio.profile",
        &[AUDIO_PROFILE],
        "Buffer profile: low-latency (128 frames, 2.7 ms), default (automatic), status",
    ),
    cmd(
        &["audio", "appmix"],
        "audio.appmix",
        &[choice("state", &["on", "off", "status"])],
        "OBS: the \"WolOS App Mix\" output and \"Mic + App Mix\" source (pipewire-pulse modules, on demand)",
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
        &[
            arg("ssid"),
            Arg {
                name: "hidden",
                choices: &["hidden"],
                optional: true,
                rest: false,
            },
        ],
        "Join a network (add `hidden` for one that hides its name); Wi-Fi password on stdin",
    ),
    cmd(
        &["wifi", "disconnect"],
        "wifi.disconnect",
        &[],
        "Leave the current Wi-Fi network (it stays saved)",
    ),
    cmd(
        &["wifi", "hotspot"],
        "wifi.hotspot",
        &[],
        "Is the Wi-Fi hotspot on",
    ),
    cmd(
        &["wifi", "hotspot", "on"],
        "wifi.hotspot.on",
        &[opt("name")],
        "Share this computer's internet over Wi-Fi; hotspot password (8+ characters) on stdin",
    ),
    cmd(
        &["wifi", "hotspot", "off"],
        "wifi.hotspot.off",
        &[],
        "Stop the Wi-Fi hotspot",
    ),
    cmd(
        &["network", "ip"],
        "network.ip",
        &[
            arg("interface"),
            Arg {
                name: "mode",
                choices: &["auto", "manual"],
                optional: true,
                rest: false,
            },
            opt("address"),
            opt("router"),
            rest("dns"),
        ],
        "A link's IPv4 address: show it, `auto` (DHCP) or `manual <address/prefix> [router] [dns…]` (undone after 15 s if the link loses the network)",
    ),
    cmd(&["vpn"], "vpn.list", &[], "VPN connections"),
    cmd(
        &["vpn", "import"],
        "vpn.import",
        &[arg("file")],
        "Add a VPN from a WireGuard .conf or OpenVPN .ovpn file (its key goes to the login keyring)",
    ),
    cmd(
        &["vpn", "connect"],
        "vpn.connect",
        &[arg("name")],
        "Connect a VPN; a password it asks for on stdin",
    ),
    cmd(
        &["vpn", "disconnect"],
        "vpn.disconnect",
        &[arg("name")],
        "Disconnect a VPN (it stays saved)",
    ),
    cmd(
        &["vpn", "forget"],
        "vpn.forget",
        &[arg("name")],
        "Delete a VPN and its saved secrets",
    ),
    cmd(&["proxy"], "proxy.get", &[], "The system proxy"),
    cmd(
        &["proxy", "off"],
        "proxy.off",
        &[],
        "No proxy (administrator)",
    ),
    cmd(
        &["proxy", "auto"],
        "proxy.auto",
        &[arg("url"), rest("options")],
        "Proxy from a PAC file: <url> [no-proxy=a,b] (administrator)",
    ),
    cmd(
        &["proxy", "manual"],
        "proxy.manual",
        &[rest("options")],
        "Fixed proxy servers: http=host:port https=host:port socks=host:port no-proxy=a,b (administrator)",
    ),
    cmd(
        &["dns"],
        "dns.get",
        &[],
        "DNS over TLS and the computer's own DNS servers",
    ),
    cmd(
        &["dns", "tls"],
        "dns.tls",
        &[
            choice("mode", &["off", "opportunistic", "strict"]),
            rest("servers"),
        ],
        "DNS over TLS: off, opportunistic or strict, with servers like 9.9.9.9#dns.quad9.net (administrator)",
    ),
    cmd(
        &["airplane"],
        "airplane.get",
        &[],
        "Airplane mode and the radios",
    ),
    cmd(
        &["airplane", "on"],
        "airplane.on",
        &[],
        "Airplane mode: Wi-Fi, Bluetooth and mobile radios off",
    ),
    cmd(
        &["airplane", "off"],
        "airplane.off",
        &[],
        "Airplane mode off: the radios that were on come back",
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
    // Web apps (ADR 0031 W5, wol_core::webapps): a web address as an app of its own.
    cmd(&["webapp"], "webapp.list", &[], "Your web apps"),
    cmd(&["webapp", "list"], "webapp.list", &[], "Your web apps"),
    Cmd {
        flags: &["own-profile"],
        ..cmd(
            &["webapp", "add"],
            "webapp.add",
            &[arg("name"), arg("url"), opt("icon")],
            "Make a web address an app with its own window and icon",
        )
    },
    cmd(
        &["webapp", "remove"],
        "webapp.remove",
        &[arg("name")],
        "Remove a web app (name or id)",
    ),
    // Desktop toggles (wol_core::toggles): Night Light, Stay Awake, Do Not Disturb.
    cmd(
        &["toggle"],
        "toggle",
        &[ONE_TOGGLE, ON_OFF],
        "Turn a toggle on or off; without on|off it flips; alone it lists them",
    ),
    cmd(
        &["toggle", "list"],
        "toggle",
        &[],
        "Desktop toggles and their state",
    ),
    cmd(
        &["remind"],
        "remind",
        &[arg("minutes"), rest("text")],
        "Show a reminder in <minutes> (1..1440)",
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
        "Undo the last change (asks first at a terminal; --yes skips), or change <id>",
    ),
    alias(&["restore"], "restore.list", &[]),
    cmd(
        &["restore", "list"],
        "restore.list",
        &[],
        "Restore points (the system copies you can start from); JSON: name, active, next_boot, release, tries_left, rollback",
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
    // Apps: .wol packages, the App Store catalogue, pkg (docs/modules/packages.md).
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
    // Crystal shards from the signed WolOS shard list (docs/modules/shards.md, RCR-040 to RCR-044).
    cmd(
        &["shard"],
        "shard.list",
        &[rest("term")],
        "Crystal shards the WolOS shard list vouches for",
    ),
    cmd(
        &["shard", "list"],
        "shard.list",
        &[rest("term")],
        "Crystal shards the WolOS shard list vouches for (filter by words)",
    ),
    Cmd {
        path: &["shard", "add"],
        action: "shard.add",
        args: &[arg("shard"), opt("dir")],
        flags: &["local"],
        help: "Add a curated shard to shard.yml pinned by commit, write shard.lock, fill the shards cache (--local: no fetch)",
        hidden: false,
    },
    Cmd {
        path: &["shard", "update"],
        action: "shard.update",
        args: &[opt("shard"), opt("dir")],
        flags: &["local"],
        help: "Move shard pins to the commits the list vouches for now and rewrite shard.lock",
        hidden: false,
    },
    Cmd {
        path: &["shard", "check"],
        action: "shard.check",
        args: &[opt("dir")],
        flags: &["local"],
        help: "Check shard.yml and shard.lock: commit pins, the lock, the signed list (--local: files only)",
        hidden: false,
    },
    // Crystal apps (wolos-crystal/skill/SKILL.md, RCR-050 to RCR-053): templates, run, check.
    Cmd {
        path: &["app", "new"],
        action: "app.new",
        args: &[arg("id"), opt("template")],
        flags: &["template"],
        help: "Make a Crystal project from a template: wol app new <name or app id> --template app|cli|tool|ai",
        hidden: false,
    },
    Cmd {
        path: &["app", "run"],
        action: "app.run",
        args: &[opt("dir")],
        flags: &["watch"],
        help: "Build and run the project (make run); --watch follows your edits",
        hidden: false,
    },
    Cmd {
        path: &["app", "check"],
        action: "app.check",
        args: &[opt("dir")],
        flags: &["rules"],
        help: "Check a Crystal project against the WolOS rules, then type-check it (--rules: the files only)",
        hidden: false,
    },
    // Every WolOS shell after a missing or failed command it names (console/hints.tsv, ADR 0032).
    Cmd {
        path: &["hint"],
        action: "hint",
        args: &[arg("command"), opt("status")],
        flags: &["not-found"],
        help: "What a missing or failed command means on WolOS, and what to run instead",
        hidden: false,
    },
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
    /// A group word alone (`wol layers`): its commands.
    Group {
        word: String,
        subs: String,
    },
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
            Self::Group { word, subs } => write!(f, "usage: wol {word} {subs}"),
        }
    }
}

/// QA-PLUG-47: where a parse error points: `wol help <the longest command the words start>`
/// (`wol plugin check` with no plugin says `Try 'wol help plugin check'.`), else `wol help`.
pub fn help_hint<S: AsRef<str>>(argv: &[S]) -> String {
    let words: Vec<&str> = argv
        .iter()
        .map(AsRef::as_ref)
        .filter(|a| !a.starts_with("--"))
        .collect();
    let known = (1..=words.len()).rev().find(|&n| {
        TREE.iter()
            .any(|c| c.path.len() >= n && c.path[..n] == words[..n])
    });
    match known {
        Some(n) => format!("Try 'wol help {}'.", words[..n].join(" ")),
        None => "Try 'wol help'.".to_owned(),
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
        // `wol --version` is the version, not the whole help (QA-STORE-76).
        let action = if flags.iter().any(|f| f == "version") {
            "version"
        } else {
            "help"
        };
        return Ok(Invocation {
            path: vec![],
            action,
            args: vec![],
            flags,
        });
    }
    let best = TREE
        .iter()
        .filter(|c| c.path.len() <= words.len() && c.path.iter().zip(&words).all(|(p, w)| p == w))
        .max_by_key(|c| c.path.len());
    // A group alone (`wol layers`, `wol boot`): its commands, as `wol help layers` lists them.
    // It said "unknown command 'layers'; did you mean 'layers'?" (QA-PWR-6).
    if best.is_none()
        && TREE.iter().any(|c| {
            !c.hidden
                && c.path.len() > words.len()
                && c.path.iter().zip(&words).all(|(p, w)| p == w)
        })
    {
        return Ok(Invocation {
            path: vec!["help".into()],
            action: "help",
            args: words.iter().map(|w| ("command", w.to_string())).collect(),
            flags,
        });
    }
    let Some(c) = best else {
        // A group with no command of its own (`wol layers`): name its commands, never "did you
        // mean 'layers'?" (QA-STORE-76).
        let subs: Vec<&str> = TREE
            .iter()
            .filter(|c| !c.hidden && c.path.len() > 1 && c.path[0] == words[0])
            .map(|c| c.path[1])
            .collect();
        if !subs.is_empty() && words.len() == 1 {
            return Err(ParseError::Group {
                word: words[0].into(),
                subs: subs.join("|"),
            });
        }
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
    // `--help` never runs the command: `run::execute` prints this path's usage, so the
    // command's own arguments are not required (`wol jobs start --help`).
    if flags.iter().any(|f| f == "help") {
        return Ok(Invocation {
            path: c.path.iter().map(|s| s.to_string()).collect(),
            action: c.action,
            args: vec![],
            flags,
        });
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
/// Longest choice list a usage line spells out (`on|off`); a longer one is named (`<panel>`)
/// and `wol help <command>` lists it. The panel list made lines of 270 characters that broke
/// the table in an 80 or 120 column terminal (QA-PWR-15).
pub const USAGE_CHOICES_MAX: usize = 24;

/// `c`'s usage (`theme set <id>`); `full` spells out every choice list.
pub fn usage_line(c: &Cmd, full: bool) -> String {
    let mut usage = c.path.join(" ");
    for a in c.args {
        let all = a.choices.join("|");
        let n = if a.choices.is_empty() || (!full && all.len() > USAGE_CHOICES_MAX) {
            a.name.to_string()
        } else {
            all
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
    usage
}

pub fn help(prefix: &[String]) -> String {
    let mut out = String::from("usage: wol <command> [args] [--json] [--yes]\n\n");
    let shown: Vec<&Cmd> = TREE
        .iter()
        .filter(|c| !c.hidden && c.path.iter().zip(prefix).all(|(p, w)| p == w))
        .collect();
    for c in &shown {
        // One command asked about: its choices in full, one list per line under it.
        let full = shown.len() == 1;
        out.push_str(&format!("  {:<44} {}\n", usage_line(c, false), c.help));
        for a in c
            .args
            .iter()
            .filter(|a| full && a.choices.join("|").len() > USAGE_CHOICES_MAX)
        {
            out.push_str(&format!("    <{}>: {}\n", a.name, a.choices.join(", ")));
        }
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
            ("update install", "update.install"),
            (
                "update apply wolos-b 2026/11/02 --yes",
                "update.apply be=wolos-b release=2026/11/02 --yes",
            ),
            ("update plan", "update.plan"),
            ("admin revoke", "admin action=revoke"),
            ("admin idle 300", "admin action=idle args=300"),
            ("gpu env", "gpu.env"),
            ("keyring", "keyring"),
            (
                "keyring get site=a.example",
                "keyring verb=get args=site=a.example",
            ),
            (
                "keyring export f.wkb --passphrase",
                "keyring verb=export args=f.wkb --passphrase",
            ),
            ("update rollback", "update.rollback"),
            ("update rollback wol-1", "update.rollback be=wol-1"),
            ("restore", "restore.list"),
            (
                "appearance set dark --json",
                "appearance.set mode=dark --json",
            ),
            ("notify -- Hi --there", "notify title=Hi body=--there"),
            ("gpu hyprconf", "gpu.hyprconf"),
            // wolos-hw.service, Settings, scripts
            ("hw boot", "hw.boot"),
            ("hw", "hw.list"),
            ("hw list --json", "hw.list --json"),
            ("hw status --json", "hw.status --json"),
            ("hw fix --yes", "hw.fix --yes"),
            // Settings > Modules, the installer, firstboot (ADR 0032)
            ("hw compute --json", "hw.compute --json"),
            ("hw report", "hw.report"),
            ("modules", "modules.list"),
            ("modules list --json", "modules.list --json"),
            ("modules status dev --json", "modules.status id=dev --json"),
            ("modules install dev", "modules.install id=dev"),
            ("modules remove dev", "modules.remove id=dev"),
            ("modules enable dev", "modules.enable id=dev"),
            // steam.desktop, the App Store Steam entry, scripts
            ("games steam", "games.steam"),
            (
                "games steam steam://install/3658110",
                "games.steam url=steam://install/3658110",
            ),
            ("games doctor --json", "games.doctor --json"),
            ("games", "games.doctor"),
            // hyprland.conf toggles and web app launchers (ADR 0031 W5)
            ("toggle", "toggle"),
            ("toggle list --json", "toggle --json"),
            ("toggle dnd", "toggle toggle=dnd"),
            ("toggle nightlight on", "toggle toggle=nightlight state=on"),
            ("toggle idle off", "toggle toggle=idle state=off"),
            ("webapp", "webapp.list"),
            (
                "webapp add Mail https://mail.example.com --own-profile",
                "webapp.add name=Mail url=https://mail.example.com --own-profile",
            ),
            ("webapp remove Mail", "webapp.remove name=Mail"),
            (
                "remind 10 tea is ready",
                "remind minutes=10 text=tea text=is text=ready",
            ),
            // the shells' hint hooks (zshrc, bashrc, config.fish, wolos.nu, profile.ps1)
            ("hint su", "hint command=su"),
            ("hint apt --not-found", "hint command=apt --not-found"),
            ("hint cp 1", "hint command=cp status=1"),
            ("", "help"),
            // QA-PWR-6: a group alone lists its commands.
            ("layers", "help command=layers"),
            ("boot", "help command=boot"),
            ("encrypt", "help command=encrypt"),
        ];
        for (line, want) in table {
            assert_eq!(d(line), want, "wol {line}");
        }
    }

    /// QA-PWR-15: `wol help` lines reached 270 characters (the panel list); the usage column
    /// stays narrow and `wol help shell toggle` lists the panels.
    #[test]
    fn help_usage_column_stays_narrow() {
        for c in TREE.iter().filter(|c| !c.hidden) {
            let u = usage_line(c, false);
            assert!(u.len() <= 80, "{} chars: {u}", u.len());
        }
        let all = help(&[]);
        assert!(
            !all.contains("switcher|notifications"),
            "no panel list in the table"
        );
        let one = help(&["shell".into(), "toggle".into()]);
        assert!(one.contains("shell toggle <panel>"), "{one}");
        assert!(
            one.contains(&format!("    <panel>: {}", PANELS.join(", "))),
            "{one}"
        );
        assert!(help(&[]).contains("theme toggle"), "short lists stay");
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
        assert_eq!(TOGGLES, wol_core::toggles::words().as_slice());
    }

    #[test]
    fn help_lists_visible_commands() {
        let h = help(&[]);
        // QA-PWR-15: the panel list is in `wol help shell toggle`, not in the table.
        assert!(h.contains("shell toggle <panel>"));
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
