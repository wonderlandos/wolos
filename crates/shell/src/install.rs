//! Install WolOS (live desktop, ADR 0013): the disks it may erase, the progress lines of
//! `libexec/wolos/install-run` and what the progress page says. Pure; UI in `ui::installer`.

/// Erases the disk and installs (root; checks the disk against `install-disks` again).
pub const RUN: &str = "/usr/local/libexec/wolos/install-run";
/// Disks that are not the boot media, one per line.
pub const DISKS: &str = "/usr/local/libexec/wolos/install-disks";
/// Facts about one disk before anything is written (`size`, `os`, `secureboot`, `refuse` lines).
pub const CHECK: &str = "/usr/local/libexec/wolos/install-check";

/// What `install-check <disk>` found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Check {
    /// Other systems on the disk, as named for people: "Windows", "Linux", "macOS", "FreeBSD".
    pub others: Vec<String>,
    /// Secure Boot is on: the installed WolOS would not start (beta: no signed loader).
    pub secure_boot: bool,
    pub refused: Option<String>,
}

pub fn parse_check(out: &str) -> Check {
    let mut c = Check::default();
    for line in out.lines() {
        match line.split_once(' ') {
            Some(("os", name)) => c.others.push(name.to_string()),
            Some(("secureboot", "on")) => c.secure_boot = true,
            Some(("refuse", why)) => c.refused = Some(why.to_string()),
            _ => {}
        }
    }
    c
}

/// The erase warning names what will be lost: "This disk has Windows on it. Erasing removes it."
pub fn erase_warning(others: &[String]) -> String {
    match others {
        [] => "Everything on this disk will be erased.".to_string(),
        [one] => format!("This disk has {one} on it. Erasing removes it."),
        [rest @ .., last] => format!(
            "This disk has {} and {last} on it. Erasing removes them.",
            rest.join(", ")
        ),
    }
}

/// `install-check <disk>` as the app runs it: through sudo where the live image allows it (the
/// Secure Boot variable is root's to read), else directly (disks are readable by operator).
pub fn check_command(disk: &str, sudo: bool) -> Vec<String> {
    if sudo {
        ["sudo", "-n", CHECK, disk].map(String::from).to_vec()
    } else {
        [CHECK, disk].map(String::from).to_vec()
    }
}

/// Why the app won't erase `title` (`refuse` reason from `install-check`); None: it may.
/// `in-use` is not a refusal here: install-run lets go of the disk's mounts first.
pub fn refusal_text(why: &str, title: &str) -> Option<String> {
    match why {
        "in-use" => None,
        "too-small" => Some(format!(
            "“{title}” is too small. WolOS needs at least 16 GB."
        )),
        _ => Some(format!(
            "“{title}” can't be erased: it's the disk WolOS started from, or it's gone. Choose another disk."
        )),
    }
}

/// Sheet shown when Secure Boot is on: honest, no jargon beyond the setting's own name.
pub const SECURE_BOOT_TITLE: &str = "Turn Off Secure Boot to Start WolOS";
pub const SECURE_BOOT_TEXT: &str = "This beta of WolOS can’t start while Secure Boot is on. \
Restart, open your computer's startup settings (often F2, F10, F12 or Delete), turn off \
Secure Boot, then install again.";

/// Why the screen stays on and the machine awake (idle inhibitors show it).
pub const AWAKE_REASON: &str = "Installing WolOS";

/// Keeps the machine awake exactly while the install runs: `set` says when to take or give back
/// the inhibitors (Some(on)), None when nothing changes.
#[derive(Debug, Default)]
pub struct Awake {
    on: bool,
}

impl Awake {
    pub fn set(&mut self, installing: bool) -> Option<bool> {
        (installing != self.on).then(|| {
            self.on = installing;
            installing
        })
    }
}

/// Prefix of every progress line `install-run` and the installer script print.
pub const MARK: &str = "wolos-install: ";
/// Title word of a bsdinstall error box (an extract error; the installer script replaces the
/// final one with a progress line): the app answers it with Return so the run can end.
pub const BSDINSTALL_ERROR: &str = "Error";
/// Smallest disk WolOS installs on (system, apps, swap and room to work).
pub const MIN_BYTES: u64 = 16_000_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disk {
    pub name: String,
    pub model: String,
    pub bytes: u64,
}

impl Disk {
    /// "Samsung SSD 980 · 500 GB"-style title: the model, else the device name.
    pub fn title(&self) -> String {
        if self.model.is_empty() {
            self.name.clone()
        } else {
            self.model.clone()
        }
    }

    pub fn big_enough(&self) -> bool {
        self.bytes >= MIN_BYTES
    }
}

/// Decimal size as disks are sold: "500 GB", "1 TB", "1.5 TB".
pub fn size_text(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1000.0 {
        let tb = gb / 1000.0;
        let r = (tb * 10.0).round() / 10.0;
        if r.fract() == 0.0 {
            format!("{r:.0} TB")
        } else {
            format!("{r:.1} TB")
        }
    } else {
        format!("{} GB", gb.round() as u64)
    }
}

/// Disks from `geom disk list`, keeping only `allowed` (from `install-disks`), in that order.
pub fn parse_disks(geom: &str, allowed: &str) -> Vec<Disk> {
    let mut all: Vec<Disk> = Vec::new();
    for line in geom.lines() {
        let (k, v) = match line.trim().split_once(':') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };
        match k {
            "Geom name" => all.push(Disk {
                name: v.into(),
                model: String::new(),
                bytes: 0,
            }),
            "Mediasize" => {
                if let Some(d) = all.last_mut() {
                    d.bytes = v
                        .split_whitespace()
                        .next()
                        .and_then(|n| n.parse().ok())
                        .unwrap_or(0);
                }
            }
            "descr" => {
                if let Some(d) = all.last_mut()
                    && v != "(null)"
                {
                    d.model = v.into();
                }
            }
            _ => {}
        }
    }
    allowed
        .lines()
        .map(str::trim)
        .filter_map(|n| all.iter().find(|d| d.name == n).cloned())
        .collect()
}

/// Root for `install-run <disk>`: sudo, which the live image allows its user without a password
/// (`image/build.sh`); `su` to the live root (empty password) where sudo is missing.
pub fn install_command(disk: &str, sudo: bool) -> Vec<String> {
    if sudo {
        ["sudo", "-n", RUN, disk].map(String::from).to_vec()
    } else {
        vec![
            "su".into(),
            "-m".into(),
            "root".into(),
            "-c".into(),
            format!("{RUN} {disk}"),
        ]
    }
}

/// The live user is in `operator`, which may run shutdown.
pub fn restart_command() -> Vec<String> {
    ["/sbin/shutdown", "-r", "now"].map(String::from).to_vec()
}

/// Where an install is: the stage words of the progress lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Checking the disk.
    Prepare,
    /// GPT, EFI and BIOS boot partitions, the ZFS pool and boot environment.
    Disk,
    /// The system files are copied (no progress of their own).
    Files,
    /// Boot loader on the EFI partition.
    Boot,
    /// rc.conf, fstab, time zone of the new system.
    Settings,
    /// Apps from the local repository.
    Packages,
    /// WolOS settings, unmount.
    Finish,
    Done,
    Failed,
}

impl Stage {
    fn parse(word: &str) -> Option<Stage> {
        Some(match word {
            "prepare" => Stage::Prepare,
            "disk" => Stage::Disk,
            "files" => Stage::Files,
            "boot" => Stage::Boot,
            "settings" => Stage::Settings,
            "packages" => Stage::Packages,
            "finish" => Stage::Finish,
            "done" => Stage::Done,
            "failed" => Stage::Failed,
            _ => return None,
        })
    }
}

/// One progress line: `wolos-install: <stage> <percent> <message>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub stage: Stage,
    /// 0-100, never backwards within a run (except `failed`, which says 0).
    pub percent: u8,
    /// For the log; the page says `doing(stage)`.
    pub message: String,
}

impl Report {
    pub fn new(stage: Stage, percent: u8) -> Report {
        Report {
            stage,
            percent,
            message: String::new(),
        }
    }
}

/// One progress line (bsdinstall may draw around it on the same terminal line).
pub fn parse_line(line: &str) -> Option<Report> {
    let rest = &line[line.find(MARK)? + MARK.len()..];
    let rest = rest.trim_end_matches(['\r', '\n']);
    let (stage, rest) = rest.split_once(' ')?;
    let (percent, message) = rest.split_once(' ').unwrap_or((rest, ""));
    let percent: u8 = percent.parse().ok().filter(|p| *p <= 100)?;
    Some(Report {
        stage: Stage::parse(stage)?,
        percent,
        message: message.trim().to_string(),
    })
}

/// Where `files` starts and ends (the boot loader step's percent): the bar creeps between them
/// while the system files are copied, and never past the end.
const FILES_START: f64 = 0.05;
const FILES_END: f64 = 0.21;
/// Copying the system files: about this long.
const FILES_SECS: f64 = 120.0;

/// Bar position for `r`, `secs` after its stage began.
pub fn fraction(r: &Report, secs: f64) -> f64 {
    let at = f64::from(r.percent) / 100.0;
    match r.stage {
        Stage::Files => {
            let creep =
                FILES_START + (FILES_END - FILES_START) * 0.9 * (secs / FILES_SECS).min(1.0);
            at.max(creep)
        }
        Stage::Failed => 0.0,
        _ => at,
    }
}

/// Seconds left, from how fast the bar moved since `start` (None until it has moved enough).
pub fn seconds_left(start: (f64, f64), now: (f64, f64)) -> Option<f64> {
    let (moved, took) = (now.0 - start.0, now.1 - start.1);
    (moved >= 0.05 && took > 0.0).then(|| (1.0 - now.0) * took / moved)
}

/// The line under the bar.
pub fn remaining_text(secs: Option<f64>) -> String {
    match secs {
        None => "Estimating time remaining…".into(),
        Some(s) if s < 60.0 => "Less than a minute remaining".into(),
        Some(s) => {
            let m = (s / 60.0).round().max(1.0) as u64;
            if m == 1 {
                "About a minute remaining".into()
            } else {
                format!("About {m} minutes remaining")
            }
        }
    }
}

/// What the page says the installer is doing.
pub fn doing(stage: Stage) -> &'static str {
    match stage {
        Stage::Prepare | Stage::Disk => "Preparing the disk…",
        Stage::Files => "Copying system files…",
        Stage::Boot => "Installing the startup files…",
        Stage::Settings => "Saving settings…",
        Stage::Packages => "Installing apps…",
        Stage::Finish => "Finishing up…",
        Stage::Done => "Done",
        Stage::Failed => "",
    }
}

/// Failed page subtitle, by the stage the install stopped in.
pub fn stopped_text(at: Stage) -> &'static str {
    match at {
        Stage::Prepare | Stage::Disk => {
            "The disk couldn't be prepared. Check that it's connected, then try again or choose another disk."
        }
        _ => "The installation stopped before it finished. Try again, or choose another disk.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GEOM: &str = "\
Geom name: ada0
Providers:
1. Name: ada0
   Mediasize: 500107862016 (466G)
   Sectorsize: 512
   Mode: r0w0e0
   descr: Samsung SSD 870 EVO 500GB
   ident: S5Y1NX0R
Geom name: da0
Providers:
1. Name: da0
   Mediasize: 15518924800 (14G)
   descr: SanDisk Ultra
Geom name: nda0
Providers:
1. Name: nda0
   Mediasize: 1000204886016 (932G)
   descr: (null)
";

    #[test]
    fn disks_are_listed_by_model_and_size() {
        let d = parse_disks(GEOM, "nda0\nada0\n");
        assert_eq!(
            d.iter()
                .map(|d| (d.title(), size_text(d.bytes)))
                .collect::<Vec<_>>(),
            [
                ("nda0".to_string(), "1 TB".to_string()),
                (
                    "Samsung SSD 870 EVO 500GB".to_string(),
                    "500 GB".to_string()
                ),
            ],
            "only install-disks' disks, in its order; boot stick left out"
        );
        assert!(d.iter().all(Disk::big_enough));
        let stick = &parse_disks(GEOM, "da0")[0];
        assert!(!stick.big_enough());
        assert_eq!(size_text(1_500_000_000_000), "1.5 TB");
        assert_eq!(install_command("ada0", true), ["sudo", "-n", RUN, "ada0"]);
        assert_eq!(install_command("ada0", false)[4], format!("{RUN} ada0"));
    }

    #[test]
    fn progress_lines_from_the_installer() {
        assert_eq!(
            parse_line("wolos-install: prepare 0 Preparing to install on ada0"),
            Some(Report {
                stage: Stage::Prepare,
                percent: 0,
                message: "Preparing to install on ada0".into()
            })
        );
        let p = parse_line("\x1b[2J\x1b[Hwolos-install: packages 53 Installing apps (30 of 74)\r")
            .unwrap();
        assert_eq!((p.stage, p.percent), (Stage::Packages, 53));
        assert_eq!(p.message, "Installing apps (30 of 74)");
        assert_eq!(
            parse_line("wolos-install: done 100").map(|r| r.stage),
            Some(Stage::Done)
        );
        assert_eq!(
            parse_line("wolos-install: failed 0 disk: a step failed").map(|r| r.stage),
            Some(Stage::Failed)
        );
        for junk in [
            "wolos-install: packages x Installing",
            "wolos-install: packages 101 Too far",
            "wolos-install: shrug 5 What",
            "wolos-install: done",
            "Installing WolOS...",
        ] {
            assert_eq!(parse_line(junk), None, "{junk}");
        }
    }

    /// Every progress line the scripts print parses, in order, and the bar never goes back.
    #[test]
    fn the_scripts_speak_this_protocol() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../image/");
        let mut seen = Vec::new();
        for f in [
            "files/usr/local/libexec/wolos/install-run",
            "installerconfig",
        ] {
            let text = std::fs::read_to_string(format!("{root}{f}")).unwrap();
            // echo/printf lines ("%d" is a percent) and install-run's `say <stage> <percent> ...`.
            let printed = text.match_indices(MARK).map(|(i, _)| &text[i..]);
            let said = text
                .match_indices("say ")
                .map(|(i, _)| &text[i + "say ".len()..]);
            for (line, from_say) in printed.map(|l| (l, false)).chain(said.map(|l| (l, true))) {
                let line = line
                    .lines()
                    .next()
                    .unwrap()
                    .replace('"', "")
                    .replace("%d", "50");
                if line.contains('<') || line.contains("$*") {
                    continue; // the format in comments, say's own definition
                }
                let line = if from_say {
                    format!("{MARK}{line}")
                } else {
                    line
                };
                let r = parse_line(&line).unwrap_or_else(|| panic!("{f}: {line}"));
                seen.push(r.stage);
            }
        }
        for s in [
            Stage::Disk,
            Stage::Files,
            Stage::Boot,
            Stage::Settings,
            Stage::Packages,
            Stage::Finish,
            Stage::Failed,
        ] {
            assert!(seen.contains(&s), "no script reports {s:?}");
        }
        // (report, seconds since its stage began), as the page samples them.
        let order = [
            (Report::new(Stage::Prepare, 0), 0.0),
            (Report::new(Stage::Disk, 2), 9.0),
            (Report::new(Stage::Files, 5), 0.0),
            (Report::new(Stage::Files, 5), 30.0),
            (Report::new(Stage::Files, 6), 30.0),
            (Report::new(Stage::Files, 6), 600.0),
            (Report::new(Stage::Boot, 22), 0.0),
            (Report::new(Stage::Settings, 24), 0.0),
            (Report::new(Stage::Packages, 25), 0.0),
            (Report::new(Stage::Packages, 95), 300.0),
            (Report::new(Stage::Finish, 95), 0.0),
            (Report::new(Stage::Finish, 97), 5.0),
            (Report::new(Stage::Done, 100), 0.0),
        ];
        let bar: Vec<f64> = order.iter().map(|(r, t)| fraction(r, *t)).collect();
        assert!(bar.windows(2).all(|w| w[0] <= w[1]), "{bar:?}");
        assert_eq!(*bar.last().unwrap(), 1.0);
    }

    #[test]
    fn awake_only_while_installing() {
        let mut a = Awake::default();
        assert_eq!(a.set(false), None, "nothing held on the disk page");
        assert_eq!(a.set(true), Some(true), "Erase and Install takes it");
        assert_eq!(a.set(true), None, "held once");
        assert_eq!(a.set(false), Some(false), "done or stopped gives it back");
        assert_eq!(a.set(true), Some(true), "Try Again takes it again");
    }

    #[test]
    fn the_app_checks_a_disk_and_says_why_not() {
        assert_eq!(check_command("ada0", true), ["sudo", "-n", CHECK, "ada0"]);
        assert_eq!(check_command("ada0", false), [CHECK, "ada0"]);
        assert_eq!(refusal_text("in-use", "SanDisk"), None);
        for why in ["too-small", "not-listed"] {
            let t = refusal_text(why, "SanDisk").unwrap();
            assert!(
                crate::copy::lint(&t).is_empty(),
                "{t}: {:?}",
                crate::copy::lint(&t)
            );
        }
    }

    #[test]
    fn check_names_other_systems_in_the_warning() {
        let c = parse_check("size 500000000000\nos Windows\nos Linux\nsecureboot on\n");
        assert_eq!(c.others, ["Windows", "Linux"]);
        assert!(c.secure_boot && c.refused.is_none());
        assert_eq!(
            erase_warning(&c.others[..1]),
            "This disk has Windows on it. Erasing removes it."
        );
        assert_eq!(
            erase_warning(&c.others),
            "This disk has Windows and Linux on it. Erasing removes them."
        );
        assert_eq!(
            erase_warning(&[]),
            "Everything on this disk will be erased."
        );
        assert_eq!(
            parse_check("refuse too-small\n").refused.as_deref(),
            Some("too-small")
        );
        let script = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../image/files/usr/local/libexec/wolos/install-check"
        );
        let text = std::fs::read_to_string(script).unwrap();
        for w in [
            "\"os \" names[i]",
            "secureboot on",
            "refuse too-small",
            "Windows Linux macOS FreeBSD",
        ] {
            assert!(text.contains(w), "install-check speaks {w}");
        }
    }

    #[test]
    fn time_left_reads_well_and_copy_is_clean() {
        assert_eq!(remaining_text(None), "Estimating time remaining…");
        assert_eq!(
            seconds_left((0.25, 100.0), (0.27, 110.0)),
            None,
            "too little to tell"
        );
        let left = seconds_left((0.25, 100.0), (0.6, 240.0)).unwrap();
        assert!((left - 160.0).abs() < 1e-6, "{left}");
        assert_eq!(remaining_text(Some(left)), "About 3 minutes remaining");
        assert_eq!(remaining_text(Some(70.0)), "About a minute remaining");
        assert_eq!(remaining_text(Some(20.0)), "Less than a minute remaining");
        for s in [
            Stage::Disk,
            Stage::Files,
            Stage::Boot,
            Stage::Settings,
            Stage::Packages,
            Stage::Finish,
        ] {
            assert!(!doing(s).is_empty());
            let l = crate::copy::lint(stopped_text(s));
            assert!(l.is_empty(), "{s:?}: {l:?}");
        }
    }
}
