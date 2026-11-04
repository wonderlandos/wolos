// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol install ...`: the installer's steps that touch a disk (ADR 0023). The decisions live in
//! `wol_core::install`; this runs them, one tool call at a time, and after every change to the
//! partition table it reads the table back and stops if anything outside the person's choice moved.
//! Reading a disk (`probe`) only ever mounts read-only and writes nothing.
use crate::cli::Invocation;
use crate::login;
use crate::run::{Env, fail};
use shared::errors::ErrorCode;
use shared::login::NewAccount;
use std::collections::BTreeSet;
use std::path::Path;
use wol_core::install::disk::{self, Blocked, Disk, Part};
use wol_core::install::layout::{Compiled, Fs, Layout, Op, Slots, Target, unchanged};
use wol_core::install::plan::{
    Plan, console_keymap, keeps_bitlocker, keeps_windows, unlock_problem,
};
use wol_core::{encrypt, region};

/// Where a probe looks inside a file system, and where btrfs is opened to shrink it.
const LOOK: &str = "/run/wolos/install-look";

pub struct Out {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// The one way out of this module: run a program.
pub trait Tools {
    fn run(&mut self, argv: &[&str], stdin: Option<&str>) -> Out;
}

pub struct Real;

impl Tools for Real {
    fn run(&mut self, argv: &[&str], stdin: Option<&str>) -> Out {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let spawned = Command::new(argv[0])
            .args(&argv[1..])
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let Ok(mut c) = spawned else {
            return Out {
                ok: false,
                stdout: String::new(),
                stderr: format!("{} did not start", argv[0]),
            };
        };
        if let (Some(mut i), Some(text)) = (c.stdin.take(), stdin) {
            let _ = i.write_all(text.as_bytes());
        }
        match c.wait_with_output() {
            Ok(o) => Out {
                ok: o.status.success(),
                stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            },
            Err(e) => Out {
                ok: false,
                stdout: String::new(),
                stderr: e.to_string(),
            },
        }
    }
}

/// The partition table and file systems as they are now, without opening anything.
pub fn read_table(t: &mut dyn Tools, path: &str) -> Option<Disk> {
    let ls = t.run(
        &[
            "lsblk",
            "--json",
            "-b",
            "-o",
            // NAME: without it lsblk prints a flat list, no partitions under their disk.
            "NAME,PATH,SIZE,MODEL,LOG-SEC,FSTYPE,LABEL,UUID,MOUNTPOINTS",
            path,
        ],
        None,
    );
    if !ls.ok {
        return None;
    }
    let sf = t.run(&["sfdisk", "--json", path], None);
    disk::parse(sf.ok.then_some(sf.stdout.as_str()), &ls.stdout)
}

/// Read-only mount options that replay no journal either.
fn look_options(fstype: &str) -> &'static str {
    match fstype {
        "ext3" | "ext4" => "ro,noexec,nosuid,nodev,noload",
        "btrfs" => "ro,noexec,nosuid,nodev,rescue=nologreplay",
        "xfs" => "ro,noexec,nosuid,nodev,norecovery",
        _ => "ro,noexec,nosuid,nodev",
    }
}

fn os_of(t: &mut dyn Tools, top: &str) -> Option<String> {
    for sub in ["", "@", "@rootfs", "root", "@wolos-a"] {
        for f in ["etc/os-release", "usr/lib/os-release"] {
            let p = Path::new(top).join(sub).join(f);
            let o = t.run(&["cat", &p.to_string_lossy()], None);
            if o.ok {
                return disk::parse_os_release(&o.stdout);
            }
        }
    }
    None
}

/// What only opening a file system can tell.
fn look_inside(t: &mut dyn Tools, p: &mut Part) {
    if !t.run(&["mkdir", "-p", LOOK], None).ok {
        return;
    }
    let opts = look_options(&p.fstype);
    if !t.run(&["mount", "-o", opts, &p.path, LOOK], None).ok {
        return;
    }
    if p.fstype == "btrfs" {
        let o = t.run(&["btrfs", "inspect-internal", "min-dev-size", LOOK], None);
        p.min_bytes = disk::parse_btrfs_min(&o.stdout);
    }
    if p.is_esp() {
        let o = t.run(&["df", "-B1", "--output=used,avail", LOOK], None);
        if let Some((used, free)) = disk::parse_df(&o.stdout) {
            p.used_bytes = Some(used);
            p.free_bytes = Some(free);
        }
        // Another WolOS's startup files: the layout refuses to reuse this one (Refusal::EspHasWolos).
        p.wolos = t
            .run(&["test", "-e", &format!("{LOOK}/EFI/wolos")], None)
            .ok;
    } else {
        p.os = os_of(t, LOOK);
    }
    t.run(&["umount", LOOK], None);
}

/// Everything the pages need to know about `path`. Writes nothing to it.
pub fn probe(t: &mut dyn Tools, path: &str) -> Option<Disk> {
    let mut d = read_table(t, path)?;
    for p in d.parts.iter_mut().filter(|p| !p.mounted) {
        match p.fstype.as_str() {
            "ntfs" => {
                let o = t.run(
                    &["ntfsresize", "--info", "--no-progress-bar", &p.path],
                    None,
                );
                let text = format!("{}\n{}", o.stdout, o.stderr);
                match disk::parse_ntfsresize(&text).filter(|_| o.ok) {
                    Some(m) => p.min_bytes = Some(m),
                    None => p.blocked = Some(disk::ntfs_blocked(&text)),
                }
            }
            "BitLocker" => p.blocked = Some(Blocked::BitLocker),
            "apfs" | "hfsplus" => p.blocked = Some(Blocked::MacOs),
            "ext2" | "ext3" | "ext4" => {
                let m = t.run(&["resize2fs", "-P", &p.path], None);
                let h = t.run(&["dumpe2fs", "-h", &p.path], None);
                if let (Some(min), Some((bs, n, free))) = (
                    disk::parse_resize2fs(&m.stdout),
                    disk::parse_dumpe2fs(&h.stdout),
                ) {
                    p.min_bytes = Some(min * bs);
                    p.used_bytes = Some((n - free) * bs);
                }
                look_inside(t, p);
            }
            "btrfs" | "xfs" | "f2fs" => look_inside(t, p),
            "vfat" if p.is_esp() => look_inside(t, p),
            _ => {}
        }
    }
    Some(d)
}

/// Why a run stopped: for the log. The app shows `install::stopped_text`.
#[derive(Debug, PartialEq, Eq)]
pub enum Stop {
    /// A tool said no.
    Step(String),
    /// A partition nobody chose moved: everything stops.
    Moved(String),
}

/// Where the new system ended up.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub esp: String,
    pub root: String,
    /// What the root file system is on (the mapper device when encrypted).
    pub fsdev: String,
    pub slots_two: bool,
    /// Separate /home and /var, mounted by install-run at the right moment.
    pub home: String,
    pub var: String,
    /// fstab lines beyond what install-run writes.
    pub fstab: Vec<String>,
    /// crypttab lines for the encrypted volumes beyond the system (`<name> UUID=<luks> <key file> luks`):
    /// install-run writes them and puts the install key at each key file, inside the encrypted system.
    pub crypttab: Vec<String>,
    /// Another system stays on this disk ([`Disk::others_after`]): the menu waits a moment.
    pub others: bool,
    /// The startup partition is an existing one, kept as it is (`Change::ReuseEsp`): boot-setup
    /// keeps its fallback loader and menu settings when another system stays too (audit3 R2B-4).
    pub esp_reused: bool,
    /// That reused startup partition holds the startup files of a WolOS this layout replaces
    /// (`compile` refuses it while one stays): install-run clears them first (audit3 R2B-5).
    pub esp_wolos: bool,
    /// Windows stays on this disk: the clock stays in local time, BitLocker may ask for its key.
    pub windows: bool,
    /// That Windows is protected by BitLocker.
    pub bitlocker: bool,
}

/// Where the install key of an extra encrypted volume lives in the new system (systemd-cryptsetup
/// reads crypttab's third field; the system volume unlocks first, so the file is never in the open).
pub const KEY_DIR: &str = "/etc/cryptsetup-keys.d";

/// Device-mapper name of an encrypted volume: the system is `wolosroot` (`encrypt::MAPPER`), the
/// others `wolos` + their folder (`woloshome`, `wolosvar`, `wolosswap`, `wolosgames`).
pub fn mapper_for(mount: &str) -> String {
    match mount {
        "/" => encrypt::MAPPER.to_string(),
        "swap" => "wolosswap".to_string(),
        m => format!("wolos{}", m.trim_start_matches('/').replace('/', "-")),
    }
}

/// One `KEY=VALUE` line for install-run, which reads them against a list of keys and never runs
/// them as shell (audit3 R2B-14): the value is taken literally to the end of the line, so a line
/// end inside it would start a new key and is turned into a space.
fn kv(k: &str, v: &str) -> String {
    format!("{k}={}\n", v.replace(['\n', '\r'], " "))
}

impl Applied {
    /// `KEY=VALUE` lines install-run reads: one value per line, `extra_fstab` and
    /// `extra_crypttab` once per line of theirs.
    pub fn env(&self) -> String {
        let mut s = String::new();
        for (k, v) in [
            ("esp", &self.esp),
            ("root", &self.root),
            ("fsdev", &self.fsdev),
            ("home_dev", &self.home),
            ("var_dev", &self.var),
        ] {
            s.push_str(&kv(k, v));
        }
        s.push_str(&kv("slots", if self.slots_two { "two" } else { "one" }));
        for (k, b) in [
            ("others", self.others),
            ("esp_reused", self.esp_reused),
            ("esp_wolos", self.esp_wolos),
            ("windows", self.windows),
            ("bitlocker", self.bitlocker),
        ] {
            s.push_str(&kv(k, if b { "1" } else { "0" }));
        }
        for l in &self.fstab {
            s.push_str(&kv("extra_fstab", l));
        }
        for l in &self.crypttab {
            s.push_str(&kv("extra_crypttab", l));
        }
        s
    }
}

struct Run<'a> {
    t: &'a mut dyn Tools,
    disk: &'a str,
    /// New partitions by index in `Layout::create`.
    made: Vec<Option<String>>,
    /// LUKS volumes this run opened (mapper names), in order: closed again when the run stops,
    /// so "Try Again" finds the partitions free for `luksFormat` (audit core-install #2).
    opened: Vec<String>,
    log: Vec<String>,
}

impl Run<'_> {
    fn must(&mut self, argv: &[&str], stdin: Option<&str>) -> Result<String, Stop> {
        let o = self.t.run(argv, stdin);
        if o.ok {
            Ok(o.stdout)
        } else {
            Err(Stop::Step(format!(
                "{}: {}",
                argv.join(" "),
                o.stderr.trim()
            )))
        }
    }

    fn settle(&mut self) {
        self.t.run(&["udevadm", "settle"], None);
    }

    /// btrfs resizes only while mounted, and only from inside.
    fn with_btrfs(&mut self, path: &str, size: &str) -> Result<(), Stop> {
        self.must(&["mkdir", "-p", LOOK], None)?;
        self.must(&["mount", path, LOOK], None)?;
        let r = self.must(&["btrfs", "filesystem", "resize", size, LOOK], None);
        self.t.run(&["umount", LOOK], None);
        r.map(|_| ())
    }

    fn shrink_fs(&mut self, path: &str, fstype: &str, bytes: u64) -> Result<(), Stop> {
        let size = bytes.to_string();
        match fstype {
            "ntfs" => {
                // The dry run proves the shrink first; the real one asks to be sure.
                let dry = ["ntfsresize", "--no-action", "--size", &size, path];
                self.must(&dry, Some("y\n"))?;
                self.must(
                    &["ntfsresize", "--force", "--size", &size, path],
                    Some("y\n"),
                )?;
            }
            "ext2" | "ext3" | "ext4" => {
                self.must(&["e2fsck", "-f", "-y", path], None)?;
                let k = format!("{}K", bytes / 1024);
                self.must(&["resize2fs", path, &k], None)?;
            }
            "btrfs" => self.with_btrfs(path, &size)?,
            other => return Err(Stop::Step(format!("can't shrink {other}"))),
        }
        Ok(())
    }

    fn grow_fs(&mut self, path: &str, fstype: &str) -> Result<(), Stop> {
        match fstype {
            "ntfs" => {
                self.must(&["ntfsresize", "--force", path], Some("y\n"))?;
            }
            "ext2" | "ext3" | "ext4" => {
                self.must(&["e2fsck", "-f", "-y", path], None)?;
                self.must(&["resize2fs", path], None)?;
            }
            "btrfs" => self.with_btrfs(path, "max")?,
            "xfs" => {
                self.must(&["mkdir", "-p", LOOK], None)?;
                self.must(&["mount", path, LOOK], None)?;
                let r = self.must(&["xfs_growfs", LOOK], None);
                self.t.run(&["umount", LOOK], None);
                r?;
            }
            other => return Err(Stop::Step(format!("can't grow {other}"))),
        }
        Ok(())
    }

    fn sized(&mut self, number: u32, sectors: u64) -> Result<(), Stop> {
        let n = number.to_string();
        let disk = self.disk.to_string();
        self.must(
            &["sfdisk", "--no-tell-kernel", "-N", &n, &disk],
            Some(&format!(",{sectors}\n")),
        )?;
        self.must(&["partx", "-u", "--nr", &n, &disk], None)?;
        Ok(())
    }

    fn mkfs(&mut self, dev: &str, fs: Fs, label: &str) -> Result<(), Stop> {
        match fs {
            Fs::Btrfs => self.must(&["mkfs.btrfs", "-f", "-L", label, dev], None),
            Fs::Ext4 => self.must(&["mkfs.ext4", "-F", "-L", label, dev], None),
            Fs::Xfs => self.must(&["mkfs.xfs", "-f", "-L", label, dev], None),
            Fs::Vfat => self.must(&["mkfs.vfat", "-F", "32", "-n", label, dev], None),
            Fs::Swap => self.must(&["mkswap", "-L", label, dev], None),
        }
        .map(|_| ())
    }
}

fn label_for(mount: &str) -> &'static str {
    match mount {
        "/" => "WolOS",
        "/efi" => "WOLOS",
        "/home" => "Home",
        "/var" => "Var",
        "swap" => "Swap",
        _ => "Data",
    }
}

fn step(r: &mut Run, op: &Op, l: &Layout, key: &str) -> Result<(), Stop> {
    let disk = r.disk.to_string();
    match op {
        Op::Repart => return Err(Stop::Step("Erase runs through systemd-repart".into())),
        Op::NewTable => {
            r.must(&["sfdisk", &disk], Some("label: gpt\n"))?;
        }
        Op::Delete { number, .. } => {
            r.must(
                &[
                    "sfdisk",
                    "--no-tell-kernel",
                    "--delete",
                    &disk,
                    &number.to_string(),
                ],
                None,
            )?;
            r.must(&["partx", "-d", "--nr", &number.to_string(), &disk], None)?;
        }
        Op::ShrinkFs {
            path,
            fstype,
            bytes,
            ..
        } => r.shrink_fs(path, fstype, *bytes)?,
        Op::GrowFs { path, fstype, .. } => r.grow_fs(path, fstype)?,
        Op::ShrinkPart {
            number, sectors, ..
        }
        | Op::GrowPart {
            number, sectors, ..
        } => r.sized(*number, *sectors)?,
        Op::Create {
            index,
            start,
            sectors,
            kind,
            name,
        } => {
            let line = format!("start={start}, size={sectors}, type={kind}, name=\"{name}\"\n");
            r.must(&["sfdisk", "--append", &disk], Some(&line))?;
            r.settle();
            let now = read_table(r.t, &disk)
                .ok_or_else(|| Stop::Step("the disk can't be read".into()))?;
            let p = now.parts.iter().find(|p| p.start == *start);
            let p = p.ok_or_else(|| Stop::Step("new partition is missing".into()))?;
            if r.made.len() <= *index {
                r.made.resize(*index + 1, None);
            }
            r.made[*index] = Some(p.path.clone());
        }
        Op::Format {
            target,
            fs,
            encrypt: lock,
        } => {
            let (path, mount) = match target {
                Target::New(i) => (
                    r.made
                        .get(*i)
                        .cloned()
                        .flatten()
                        .ok_or_else(|| Stop::Step("new partition is missing".into()))?,
                    l.create[*i].mount.clone(),
                ),
                Target::Existing(id) => {
                    let now = read_table(r.t, &disk)
                        .ok_or_else(|| Stop::Step("the disk can't be read".into()))?;
                    let p = now
                        .part(id)
                        .ok_or_else(|| Stop::Step("partition is missing".into()))?;
                    let m = l.assign.iter().find(|a| &a.part == id);
                    (p.path.clone(), m.map_or(String::new(), |a| a.mount.clone()))
                }
            };
            let dev = if *lock {
                let k = format!("--key-file={key}");
                r.must(
                    &[
                        "cryptsetup",
                        "luksFormat",
                        "--batch-mode",
                        "--type",
                        "luks2",
                        "--pbkdf",
                        "argon2id",
                        &k,
                        &path,
                    ],
                    None,
                )?;
                let name = mapper_for(&mount);
                let open = if mount == "/" {
                    encrypt::open_argv(&path, key)
                } else {
                    vec!["cryptsetup".into(), "open".into(), k, path, name.clone()]
                };
                let a: Vec<&str> = open.iter().map(String::as_str).collect();
                r.must(&a, None)?;
                r.opened.push(name.clone());
                format!("/dev/mapper/{name}")
            } else {
                path
            };
            r.mkfs(&dev, *fs, label_for(&mount))?;
        }
    }
    Ok(())
}

/// Runs `c`. `before` is the disk the plan was checked against; after every step the table is
/// read back and compared. Only the partitions in `Layout::chosen` may differ.
pub fn apply(
    t: &mut dyn Tools,
    l: &Layout,
    c: &Compiled,
    before: &Disk,
    key: &str,
) -> Result<Applied, Stop> {
    let mut r = Run {
        t,
        disk: &before.path,
        made: vec![None; l.create.len()],
        opened: Vec::new(),
        log: Vec::new(),
    };
    let done = run_ops(&mut r, l, c, before, key);
    if done.is_err() {
        // A stop leaves no volume of this run open: the look-in folder is let go first (a stop
        // may come while it still holds the system volume), then each volume, the last first.
        r.t.run(&["umount", LOOK], None);
        for name in r.opened.iter().rev() {
            r.t.run(&["cryptsetup", "close", name], None);
        }
    }
    done
}

fn run_ops(
    r: &mut Run,
    l: &Layout,
    c: &Compiled,
    before: &Disk,
    key: &str,
) -> Result<Applied, Stop> {
    let chosen: BTreeSet<&str> = l.chosen();
    for op in &c.ops {
        r.log.push(format!("{op:?}"));
        step(r, op, l, key)?;
        r.settle();
        let after = read_table(r.t, &before.path)
            .ok_or_else(|| Stop::Step("the disk can't be read".into()))?;
        // An existing partition being reformatted has a new file system identity: allowed.
        unchanged(before, &after, &chosen).map_err(Stop::Moved)?;
    }
    let now =
        read_table(r.t, &before.path).ok_or_else(|| Stop::Step("the disk can't be read".into()))?;
    let mut a = Applied {
        slots_two: c.slots == Slots::Two,
        others: !before.others_after(l).is_empty(),
        windows: keeps_windows(l, before),
        bitlocker: keeps_bitlocker(l, before),
        ..Applied::default()
    };
    for m in &c.mounts {
        let path = match &m.target {
            Target::New(i) => r.made[*i].clone().unwrap_or_default(),
            Target::Existing(id) => now.part(id).map(|p| p.path.clone()).unwrap_or_default(),
        };
        if path.is_empty() {
            return Err(Stop::Step(format!("nothing to mount at {}", m.path)));
        }
        let locked = c
            .ops
            .iter()
            .any(|o| matches!(o, Op::Format { target, encrypt: true, .. } if *target == m.target));
        match m.path.as_str() {
            "/" => {
                a.fsdev = if locked {
                    format!("/dev/mapper/{}", encrypt::MAPPER)
                } else {
                    path.clone()
                };
                a.root = path;
            }
            "/efi" => {
                if let Target::Existing(id) = &m.target {
                    a.esp_reused = !m.format;
                    a.esp_wolos = !m.format && before.part(id).is_some_and(|p| p.wolos);
                }
                a.esp = path;
            }
            other => {
                // Encrypted: the file system lives on the mapper device; crypttab opens it at boot
                // with the key install-run leaves in the (encrypted) system.
                let dev = if locked {
                    let name = mapper_for(other);
                    let luks = r.must(&["blkid", "-s", "UUID", "-o", "value", &path], None)?;
                    a.crypttab.push(format!(
                        "{name} UUID={} {KEY_DIR}/{name}.key luks",
                        luks.trim()
                    ));
                    format!("/dev/mapper/{name}")
                } else {
                    path
                };
                let uuid = r.must(&["blkid", "-s", "UUID", "-o", "value", &dev], None)?;
                let uuid = uuid.trim().to_string();
                a.fstab.push(match other {
                    "swap" => format!("UUID={uuid} none swap defaults 0 0"),
                    _ => format!("UUID={uuid} {other} {} defaults 0 2", m.fs.word()),
                });
                match other {
                    "/home" => a.home = dev,
                    "/var" => a.var = dev,
                    _ => {}
                }
            }
        }
    }
    if a.slots_two {
        let fsdev = a.fsdev.clone();
        r.must(&["mkdir", "-p", LOOK], None)?;
        r.must(&["mount", &fsdev, LOOK], None)?;
        let mut subs = vec!["@wolos-a"];
        if a.home.is_empty() {
            subs.push("@home");
        }
        if a.var.is_empty() {
            subs.push("@var");
        }
        for s in subs {
            let p = format!("{LOOK}/{s}");
            if let Err(e) = r.must(&["btrfs", "subvolume", "create", &p], None) {
                r.t.run(&["umount", LOOK], None);
                return Err(e);
            }
        }
        r.must(&["umount", LOOK], None)?;
    }
    Ok(a)
}

/// Why `wol install check` refused, as one word for the log; the app words it itself.
pub fn refusal_word(r: &wol_core::install::layout::Refusal) -> String {
    format!("{r:?}")
}

/// Reads a plan file.
pub fn read_plan(text: &str) -> Option<Plan> {
    serde_json::from_str(text).ok()
}

/// An xkb layout as the Install app offers them: `us`, `us(intl)`, `de(nodeadkeys)`, `latam`.
/// Same rule as the account's keymap (`settings::users::validate_account`).
pub fn is_valid_keyboard(k: &str) -> bool {
    (2..=16).contains(&k.len())
        && k.bytes()
            .all(|b| b.is_ascii_lowercase() || b == b'(' || b == b')')
        && k.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && k.matches('(').count() == k.matches(')').count()
        && k.matches('(').count() <= 1
        && !k.ends_with("()")
}

/// Files of the new system's language, keyboard and time zone. None: the plan's values are not
/// ones WolOS knows (the plan file is input: nothing from it reaches a path unchecked).
pub fn settings_files(p: &Plan) -> Option<Vec<(String, String)>> {
    if !region::is_valid_locale(&p.locale)
        || !wol_core::settings::timezones::is_valid(&p.zone)
        || !is_valid_keyboard(&p.keyboard)
    {
        return None;
    }
    Some(vec![
        ("etc/locale.conf".into(), format!("LANG={}\n", p.locale)),
        (
            "etc/vconsole.conf".into(),
            format!("KEYMAP={}\n", console_keymap(&p.keyboard)),
        ),
        // Every language Settings offers, not only the installer's (QA-SET-27).
        ("etc/locale.gen".into(), region::locale_gen(&p.locale)),
    ])
}

/// The account's request for `login::setup_plan`.
pub fn new_account(p: &Plan, password: &str) -> Option<NewAccount> {
    let a = p.account.as_ref()?;
    Some(NewAccount {
        name: a.name.clone(),
        full_name: a.full_name.clone(),
        hint: String::new(),
        locale: p.locale.clone(),
        keymap: p.keyboard.clone(),
        appearance: "auto".into(),
        avatar: a.tone.clone(),
        accent: shared::login::DEFAULT_ACCENT.into(),
        password: password.into(),
    })
}

/// Creates the account in the new system through the same plan as the Setup Assistant.
pub fn create_account(t: &mut dyn Tools, root: &Path, a: &NewAccount) -> bool {
    let plan = login::setup_plan(a);
    let rooted = root.to_string_lossy().into_owned();
    let passwd = root.join("etc/passwd");
    let name = a.name.clone();
    // A kept /home may already hold this person's folder, with another number on it.
    let existed = root.join("home").join(&name).exists();
    let made = login::apply_setup(
        &plan,
        root,
        |argv, stdin| {
            let mut v: Vec<&str> = vec!["arch-chroot", &rooted];
            v.extend(argv.iter().map(String::as_str));
            t.run(&v, Some(stdin)).ok
        },
        || login::find(&std::fs::read_to_string(&passwd).ok()?, &name),
    );
    let Some(u) = made else { return false };
    if existed {
        let owner = format!("{}:{}", u.uid, u.gid);
        let home = format!("/home/{name}");
        return t
            .run(
                &["arch-chroot", &rooted, "chown", "-R", &owner, &home],
                None,
            )
            .ok;
    }
    true
}

/// The computer name rule: [`wol_core::settings::hostname::is_valid`], the one Settings uses.
pub fn is_valid_hostname(h: &str) -> bool {
    wol_core::settings::hostname::is_valid(h)
}

/// `WOLOS_SWAP`: `none`, `zram` or `swapfile:<GiB>` (1-64).
pub fn is_valid_swap(s: &str) -> bool {
    match s {
        "none" | "zram" => true,
        _ => s
            .strip_prefix("swapfile:")
            .and_then(|n| n.parse::<u32>().ok())
            .is_some_and(|n| (1..=64).contains(&n)),
    }
}

/// The plan's choices beyond the disk, read from its JSON by key so a plan written before they
/// existed still runs: `hostname`, `autologin` (bool), `swap`. Anything unknown becomes the default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extras {
    pub hostname: String,
    pub autologin: bool,
    pub swap: String,
}

impl Default for Extras {
    fn default() -> Extras {
        Extras {
            hostname: "wolos".into(),
            autologin: false,
            swap: "zram".into(),
        }
    }
}

impl Extras {
    pub fn from_json(text: &str) -> Extras {
        let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
        let mut e = Extras::default();
        if let Some(h) = v.get("hostname").and_then(|h| h.as_str())
            && is_valid_hostname(h)
        {
            e.hostname = h.into();
        }
        if let Some(a) = v.get("autologin").and_then(|a| a.as_bool()) {
            e.autologin = a;
        }
        if let Some(s) = v.get("swap").and_then(|s| s.as_str())
            && is_valid_swap(s)
        {
            e.swap = s.into();
        }
        e
    }
}

/// `KEY=VALUE` lines for install-run (read like [`Applied::env`], never run). `WOLOS_SECURE`:
/// the person's Secure Boot answer, yes for a plan that never asked. `modules`: checked module
/// ids, `extra_packages`: the Store kits' checked package names, both space-separated (ADR 0032 B).
pub fn info_text(p: &Plan, x: &Extras) -> String {
    [
        kv("disk", &p.disk),
        kv("wipe", if p.layout.wipe { "1" } else { "0" }),
        kv("WOLOS_ENCRYPT", p.protect.word()),
        kv(
            "account",
            p.account.as_ref().map_or("", |a| a.name.as_str()),
        ),
        kv("WOLOS_HOSTNAME", &x.hostname),
        kv("WOLOS_AUTOLOGIN", if x.autologin { "yes" } else { "no" }),
        kv("WOLOS_SWAP", &x.swap),
        kv(
            "WOLOS_SECURE",
            if p.secure_boot.unwrap_or(true) {
                "yes"
            } else {
                "no"
            },
        ),
        kv("modules", &p.module_ids().join(" ")),
        kv("extra_packages", &p.extra_packages().join(" ")),
    ]
    .concat()
}

/// Everything about a plan that would otherwise fail after the disk is erased (install-run runs
/// `wol install check` before its first write): settings WolOS can write, an account name no
/// system user of the live image (and so of the new system) already has, a password the Setup
/// Assistant would accept. `passwd`: the live `/etc/passwd`; `password`: the side file's content.
pub fn plan_problem(p: &Plan, passwd: &str, password: Option<&str>) -> Option<&'static str> {
    if settings_files(p).is_none() {
        return Some("BadSettings");
    }
    if let Some(a) = &p.account {
        if !wol_core::settings::users::is_valid_name(&a.name) {
            return Some("BadName");
        }
        if login::find(passwd, &a.name).is_some() {
            return Some("ReservedName");
        }
        let typed = password.and_then(|w| encrypt::typed_secret(w.as_bytes()).ok());
        if !typed
            .as_deref()
            .is_some_and(wol_core::settings::users::is_valid_password)
        {
            return Some("WeakPassword");
        }
        if let Some(why) = unlock_problem(p.protect, password) {
            return Some(why);
        }
    }
    None
}

/// A secret's file read through [`encrypt::typed_secret`] (one line end off, text only). None:
/// no such file.
pub fn secret_file(env: &dyn Env, p: &Path) -> Option<Result<String, encrypt::BadSecret>> {
    match env.read_file(p) {
        Some(t) => Some(encrypt::typed_secret(t.as_bytes())),
        None => std::fs::read(p).ok().map(|b| encrypt::typed_secret(&b)),
    }
}

fn plan_of(env: &mut dyn Env, inv: &Invocation) -> Option<(Plan, String)> {
    let f = inv.get("plan")?;
    let text = env.read_file(Path::new(f))?;
    Some((read_plan(&text)?, text))
}

/// `lsblk` for [`disk::facts`]: the disk and everything on it, never opened.
pub fn read_facts(t: &mut dyn Tools, path: &str) -> Option<disk::Facts> {
    let ls = t.run(
        &["lsblk", "--json", "-b", "-o", disk::FACTS_LSBLK, path],
        None,
    );
    ls.ok.then(|| disk::facts(&ls.stdout)).flatten()
}

/// The read-only verbs scripts take disk facts from (audit3 R2B-16/19), and the computer name
/// rule (R2B-18). Any user may run them: they open nothing and write nothing.
fn read_only(env: &mut dyn Env, inv: &Invocation, t: &mut dyn Tools) -> Option<i32> {
    match inv.action {
        "install.facts" | "install.mounts" => {
            let Some(f) = inv.get("disk").and_then(|d| read_facts(t, d)) else {
                return Some(fail(env, ErrorCode::NotFound, "no such disk"));
            };
            env.out(&if inv.action == "install.facts" {
                f.lines()
            } else {
                f.mount_lines()
            });
            Some(0)
        }
        "install.hostname" => {
            let name = inv.get("name").unwrap_or_default();
            let ok = wol_core::settings::hostname::is_valid(name);
            env.out(&format!("{}\n", if ok { name } else { "wolos" }));
            Some(0)
        }
        _ => None,
    }
}

/// `wol install ...`.
pub fn verb(env: &mut dyn Env, inv: &Invocation) -> i32 {
    if let Some(rc) = read_only(env, inv, &mut Real) {
        return rc;
    }
    if !env.is_root() {
        return fail(env, ErrorCode::Denied, "the installer needs root");
    }
    let mut t = Real;
    if inv.action == "install.probe" {
        let Some(d) = inv.get("disk").and_then(|d| probe(&mut t, d)) else {
            return fail(env, ErrorCode::NotFound, "no such disk");
        };
        env.out(&serde_json::to_string(&d).unwrap_or_default());
        return 0;
    }
    let Some((plan, text)) = plan_of(env, inv) else {
        return fail(env, ErrorCode::Usage, "no readable plan file");
    };
    match inv.action {
        "install.info" => {
            env.out(&info_text(&plan, &Extras::from_json(&text)));
            0
        }
        "install.check" | "install.layout" => {
            let Some(now) = probe(&mut t, &plan.disk) else {
                return fail(env, ErrorCode::NotFound, "no such disk");
            };
            let c = match plan.check(&now) {
                Ok(c) => c,
                Err(r) => {
                    env.out(&format!("refuse {}\n", refusal_word(&r)));
                    return 2;
                }
            };
            if inv.action == "install.check" {
                let plandir = Path::new(inv.get("plan").unwrap_or_default())
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_default();
                let passwd = env.read_file(Path::new("/etc/passwd")).unwrap_or_default();
                let password = match secret_file(env, &plandir.join("account-password")) {
                    Some(Ok(p)) => Some(p),
                    // Not typed text: no password the Setup Assistant would take.
                    Some(Err(_)) => Some(String::new()),
                    None => None,
                };
                if let Some(why) = plan_problem(&plan, &passwd, password.as_deref()) {
                    env.out(&format!("refuse {why}\n"));
                    return 2;
                }
                return 0;
            }
            let key = inv.get("key").unwrap_or_default();
            match apply(&mut t, &plan.layout, &c, &now, key) {
                Ok(a) => {
                    env.out(&a.env());
                    0
                }
                Err(s) => {
                    env.err(&format!("wol install: {s:?}\n"));
                    1
                }
            }
        }
        "install.settings" => {
            let root = Path::new(inv.get("root").unwrap_or_default());
            let Some(files) = settings_files(&plan) else {
                return fail(
                    env,
                    ErrorCode::Usage,
                    "the plan has values WolOS does not know",
                );
            };
            for (f, text) in files {
                if std::fs::write(root.join(f), text).is_err() {
                    return 1;
                }
            }
            let link = root.join("etc/localtime");
            let _ = std::fs::remove_file(&link);
            let target = format!("/usr/share/zoneinfo/{}", plan.zone);
            if std::os::unix::fs::symlink(target, link).is_err() {
                return 1;
            }
            let rooted = root.to_string_lossy();
            i32::from(!t.run(&["arch-chroot", &rooted, "locale-gen"], None).ok)
        }
        "install.account" => {
            let root = Path::new(inv.get("root").unwrap_or_default());
            let pass = match inv
                .get("passfile")
                .and_then(|f| secret_file(env, Path::new(f)))
            {
                Some(Ok(p)) => p,
                Some(Err(e)) => {
                    return fail(
                        env,
                        ErrorCode::Usage,
                        &format!("the account password file is not typed text ({e:?})"),
                    );
                }
                None => String::new(),
            };
            let Some(a) = new_account(&plan, &pass) else {
                return 0;
            };
            i32::from(!create_account(&mut t, root, &a))
        }
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wol_core::install::disk::{DATA_TYPE, ESP_TYPE, Table};
    use wol_core::install::layout::{Assign, New, Resize, compile, verify};
    use wol_core::install::plan::{Account, Plan, Protect};

    const GB: u64 = 1_000_000_000;

    /// Changes the disk behind the executor's back.
    type Nasty = Box<dyn FnMut(&mut Disk)>;

    /// A disk that behaves: sfdisk really changes the fake table, every call is logged, and a
    /// hook can do something nasty at a chosen call.
    struct Fake {
        d: Disk,
        log: Vec<String>,
        /// Called after each sfdisk write.
        nasty: Option<Nasty>,
        refuse: Option<&'static str>,
        cat: Vec<(&'static str, &'static str)>,
    }

    fn table(d: &Disk) -> String {
        let parts: Vec<serde_json::Value> = d
            .parts
            .iter()
            .map(|p| {
                serde_json::json!({"node":p.path,"start":p.start,"size":p.sectors,"type":p.kind,"uuid":p.uuid,"name":p.name})
            })
            .collect();
        serde_json::json!({"partitiontable":{"label":"gpt","id":d.id,"device":d.path,"unit":"sectors",
            "firstlba":d.first,"lastlba":d.last,"sectorsize":d.sector,"partitions":parts}})
        .to_string()
    }

    const NTFSRESIZE_OPTIONS: &[&str] = &[
        "--info",
        "--no-action",
        "--force",
        "--size",
        "--no-progress-bar",
    ];

    /// As the real one: partitions come as children only when NAME is a column, else as a flat list.
    fn lsblk(d: &Disk, argv: &[&str]) -> String {
        let kids: Vec<serde_json::Value> = d
            .parts
            .iter()
            .map(|p| {
                serde_json::json!({"path":p.path,"size":p.sectors*d.sector,"fstype":p.fstype,"label":p.label,"uuid":p.fsuuid,"mountpoints":[null]})
            })
            .collect();
        let tree = argv.iter().any(|a| a.split(',').any(|c| c == "NAME"));
        let top = serde_json::json!({"path":d.path,"size":d.bytes,"model":"QEMU"});
        let all = if tree {
            let mut top = top;
            top["children"] = kids.into();
            vec![top]
        } else {
            std::iter::once(top).chain(kids).collect()
        };
        serde_json::json!({ "blockdevices": all }).to_string()
    }

    impl Fake {
        fn new(d: Disk) -> Fake {
            Fake {
                d,
                log: Vec::new(),
                nasty: None,
                refuse: None,
                cat: Vec::new(),
            }
        }
        fn writes(&self) -> Vec<&str> {
            self.log
                .iter()
                .map(String::as_str)
                .filter(|l| {
                    !["lsblk", "sfdisk --json", "udevadm", "blkid"]
                        .iter()
                        .any(|r| l.starts_with(r))
                })
                .collect()
        }
    }

    impl Tools for Fake {
        fn run(&mut self, argv: &[&str], stdin: Option<&str>) -> Out {
            let line = argv.join(" ");
            self.log.push(line.clone());
            let ok = |stdout: String| Out {
                ok: true,
                stdout,
                stderr: String::new(),
            };
            if self.refuse.is_some_and(|r| line.starts_with(r)) {
                return Out {
                    ok: false,
                    stdout: String::new(),
                    stderr: "no".into(),
                };
            }
            // The real ntfsresize stops at an option it doesn't know.
            if argv[0] == "ntfsresize"
                && argv
                    .iter()
                    .any(|a| a.starts_with("--") && !NTFSRESIZE_OPTIONS.contains(a))
            {
                return Out {
                    ok: false,
                    stdout: String::new(),
                    stderr: format!("{line}: unrecognized option"),
                };
            }
            match argv {
                ["lsblk", ..] => ok(lsblk(&self.d, argv)),
                ["sfdisk", "--json", ..] => ok(table(&self.d)),
                ["sfdisk", "--no-tell-kernel", "-N", n, _] => {
                    let n: u32 = n.parse().unwrap();
                    let s: u64 = stdin
                        .unwrap()
                        .trim()
                        .trim_start_matches(',')
                        .parse()
                        .unwrap();
                    self.d
                        .parts
                        .iter_mut()
                        .find(|p| p.number == n)
                        .unwrap()
                        .sectors = s;
                    if let Some(f) = &mut self.nasty {
                        f(&mut self.d);
                    }
                    ok(String::new())
                }
                ["sfdisk", "--no-tell-kernel", "--delete", _, n] => {
                    let n: u32 = n.parse().unwrap();
                    self.d.parts.retain(|p| p.number != n);
                    ok(String::new())
                }
                ["sfdisk", "--append", _] => {
                    let f = |k: &str| -> String {
                        stdin
                            .unwrap()
                            .split(", ")
                            .find_map(|w| w.strip_prefix(k))
                            .unwrap()
                            .trim()
                            .to_string()
                    };
                    let number = self.d.parts.iter().map(|p| p.number).max().unwrap_or(0) + 1;
                    self.d.parts.push(Part {
                        number,
                        path: format!("/dev/vda{number}"),
                        start: f("start=").parse().unwrap(),
                        sectors: f("size=").parse().unwrap(),
                        kind: f("type="),
                        uuid: format!("new{number}"),
                        ..Part::default()
                    });
                    if let Some(f) = &mut self.nasty {
                        f(&mut self.d);
                    }
                    ok(String::new())
                }
                ["blkid", ..] => ok("FS-UUID\n".into()),
                // A path is there when `cat` knows it (same suffix table).
                ["test", "-e", p] => Out {
                    ok: self.cat.iter().any(|(k, _)| p.ends_with(k)),
                    stdout: String::new(),
                    stderr: String::new(),
                },
                ["cat", p] => {
                    let hit = self.cat.iter().find(|(k, _)| p.ends_with(k));
                    match hit {
                        Some((_, v)) => ok((*v).to_string()),
                        None => Out {
                            ok: false,
                            stdout: String::new(),
                            stderr: String::new(),
                        },
                    }
                }
                ["df", ..] => ok("Used Avail\n40000000 700000000\n".into()),
                ["ntfsresize", "--info", ..] => ok(
                    "You might resize at 50000000000 bytes or 50000 MB (freeing 2 MB).\n".into(),
                ),
                ["resize2fs", "-P", _] => {
                    ok("Estimated minimum size of the filesystem: 1000000\n".into())
                }
                ["dumpe2fs", ..] => ok(
                    "Block count:   2000000\nFree blocks:   900000\nBlock size:   4096\n".into(),
                ),
                ["btrfs", "inspect-internal", ..] => ok("9000000000 bytes (8GiB)\n".into()),
                _ => ok(String::new()),
            }
        }
    }

    fn windows() -> Disk {
        let p = |n: u32, start: u64, sectors: u64, kind: &str, fs: &str| Part {
            number: n,
            path: format!("/dev/vda{n}"),
            start,
            sectors,
            kind: kind.into(),
            uuid: format!("u{n}"),
            fstype: fs.into(),
            fsuuid: format!("f{n}"),
            ..Part::default()
        };
        let mut c = p(
            2,
            206848,
            200 * GB / 512 / 2048 * 2048,
            "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7",
            "ntfs",
        );
        c.min_bytes = Some(50 * GB);
        Disk {
            path: "/dev/vda".into(),
            bytes: 300 * GB,
            sector: 512,
            table: Table::Gpt,
            id: "t".into(),
            first: 2048,
            last: 300 * GB / 512 - 34,
            parts: vec![
                p(1, 2048, 204800, ESP_TYPE, "vfat"),
                c,
                p(
                    3,
                    391000000,
                    2097152,
                    "de94bba4-06d1-4d40-a16a-bfd50179d6ac",
                    "ntfs",
                ),
            ],
            ..Disk::default()
        }
    }

    fn alongside() -> Layout {
        Layout {
            resize: vec![Resize {
                part: "u2".into(),
                bytes: 100 * GB,
            }],
            create: vec![
                New {
                    bytes: Some(GB),
                    mount: "/efi".into(),
                    fs: Fs::Vfat,
                },
                New {
                    bytes: None,
                    mount: "/".into(),
                    fs: Fs::Btrfs,
                },
            ],
            ..Layout::default()
        }
    }

    #[test]
    fn a_probe_only_reads() {
        let mut d = windows();
        d.parts.push(Part {
            number: 4,
            path: "/dev/vda4".into(),
            start: 400000000,
            sectors: 4096,
            kind: DATA_TYPE.into(),
            uuid: "u4".into(),
            fstype: "ext4".into(),
            ..Part::default()
        });
        let mut f = Fake::new(d);
        f.cat = vec![("etc/os-release", "PRETTY_NAME=\"Ubuntu 24.04\"\n")];
        let got = probe(&mut f, "/dev/vda").unwrap();
        assert_eq!(got.parts[1].min_bytes, Some(50 * GB));
        assert_eq!(got.parts[3].os.as_deref(), Some("Ubuntu 24.04"));
        assert_eq!(got.parts[3].min_bytes, Some(4_096_000_000));
        assert_eq!(got.parts[3].used_bytes, Some(1_100_000 * 4096));
        assert_eq!(got.parts[0].free_bytes, Some(700_000_000));
        assert!(!got.parts[0].wolos, "an empty startup partition");
        for l in &f.log {
            let w = l.split(' ').next().unwrap();
            assert!(
                [
                    "lsblk",
                    "sfdisk",
                    "ntfsresize",
                    "resize2fs",
                    "dumpe2fs",
                    "mkdir",
                    "mount",
                    "umount",
                    "cat",
                    "df",
                    "test"
                ]
                .contains(&w),
                "{l}"
            );
            if w == "mount" {
                assert!(l.contains("-o ro,"), "{l}");
            }
            if w == "sfdisk" {
                assert!(l.starts_with("sfdisk --json"), "{l}");
            }
            if w == "resize2fs" {
                assert!(l.starts_with("resize2fs -P"), "{l}");
            }
            if w == "ntfsresize" {
                assert!(l.contains("--info"), "{l}");
            }
        }
    }

    #[test]
    fn hibernated_and_encrypted_windows_are_named() {
        struct No;
        impl Tools for No {
            fn run(&mut self, a: &[&str], _: Option<&str>) -> Out {
                let d = {
                    let mut d = windows();
                    d.parts[1].fstype = "ntfs".into();
                    d.parts[2].fstype = "BitLocker".into();
                    d
                };
                match a[0] {
                    "lsblk" => Out {
                        ok: true,
                        stdout: lsblk(&d, a),
                        stderr: String::new(),
                    },
                    "sfdisk" => Out {
                        ok: true,
                        stdout: table(&d),
                        stderr: String::new(),
                    },
                    _ => Out {
                        ok: false,
                        stdout: String::new(),
                        stderr: "Windows is hibernated".into(),
                    },
                }
            }
        }
        let got = probe(&mut No, "/dev/vda").unwrap();
        assert_eq!(got.parts[1].blocked, Some(Blocked::NotShutDown));
        assert_eq!(got.parts[2].blocked, Some(Blocked::BitLocker));
    }

    /// audit core-install #1: a BitLocker Windows, probed as the installer probes it, with room
    /// free next to it: the free-space layout is refused with its own word (nothing runs), Erase
    /// Disk is not.
    #[test]
    fn a_bitlocker_windows_next_to_free_space_is_refused_before_any_write() {
        let mut d = windows();
        d.parts[1].fstype = "BitLocker".into();
        let mut f = Fake::new(d);
        let got = probe(&mut f, "/dev/vda").unwrap();
        assert_eq!(got.parts[1].blocked, Some(Blocked::BitLocker));
        assert!(!got.free().is_empty(), "there is room next to Windows");
        let free = Layout {
            create: alongside().create,
            ..Layout::default()
        };
        for encrypt in [false, true] {
            let r = compile(&free, &got, encrypt).unwrap_err();
            assert_eq!(refusal_word(&r), "WindowsUnchecked(\"u2\")");
        }
        assert!(compile(&Layout::erase(), &got, true).is_ok());
        assert!(
            f.writes()
                .iter()
                .all(|w| !w.starts_with("sfdisk") && !w.starts_with("cryptsetup"))
        );
    }

    #[test]
    fn alongside_runs_in_order_on_the_named_partition_only() {
        let d = windows();
        let l = alongside();
        let c = compile(&l, &d, false).unwrap();
        verify(&c, &l, &d).unwrap();
        let mut f = Fake::new(d.clone());
        let a = apply(&mut f, &l, &c, &d, "/run/k").unwrap();
        let w = f.writes();
        let at = |needle: &str| {
            w.iter()
                .position(|l| l.contains(needle))
                .unwrap_or_else(|| panic!("{needle} in {w:#?}"))
        };
        assert!(at("ntfsresize --no-action") < at("ntfsresize --force"));
        assert!(
            at("ntfsresize --force") < at("sfdisk --no-tell-kernel -N 2"),
            "file system before the partition"
        );
        assert!(at("-N 2") < at("sfdisk --append"));
        assert!(at("mkfs.vfat") < at("mkfs.btrfs"));
        assert!(at("mkfs.btrfs") < at("btrfs subvolume create /run/wolos/install-look/@wolos-a"));
        assert!(at("@home") < at("umount"));
        // The new partitions are 4 and 5; nothing else was named on any line.
        for l in &w {
            for n in ["/dev/vda1", "/dev/vda3"] {
                assert!(!l.contains(n), "{l}");
            }
        }
        assert_eq!(a.esp, "/dev/vda4");
        assert_eq!(
            (a.root.as_str(), a.fsdev.as_str()),
            ("/dev/vda5", "/dev/vda5")
        );
        assert!(a.slots_two && a.fstab.is_empty());
    }

    /// The disk does something nobody asked for mid-run: the run stops at once.
    #[test]
    fn a_partition_nobody_chose_moving_stops_the_run() {
        for (name, hurt) in [
            (
                "resized",
                (|d: &mut Disk| d.parts[2].sectors -= 2048) as fn(&mut Disk),
            ),
            ("removed", |d| {
                d.parts.remove(0);
            }),
            ("new file system", |d| d.parts[0].fsuuid = "other".into()),
            ("overlapped", |d| {
                let (s, n) = (d.parts[0].start, d.parts[0].sectors);
                d.parts.last_mut().unwrap().start = s + n / 2;
            }),
        ] {
            let d = windows();
            let l = alongside();
            let c = compile(&l, &d, false).unwrap();
            let mut f = Fake::new(d.clone());
            f.nasty = Some(Box::new(hurt));
            let r = apply(&mut f, &l, &c, &d, "/run/k");
            assert!(matches!(r, Err(Stop::Moved(_))), "{name}: {r:?}");
            assert!(
                !f.log.iter().any(|l| l.starts_with("mkfs")),
                "{name}: nothing is formatted after that"
            );
        }
    }

    #[test]
    fn a_tool_that_says_no_stops_before_the_next_step() {
        let d = windows();
        let l = alongside();
        let c = compile(&l, &d, false).unwrap();
        let mut f = Fake::new(d.clone());
        f.refuse = Some("ntfsresize --no-action");
        assert!(matches!(apply(&mut f, &l, &c, &d, "k"), Err(Stop::Step(_))));
        assert!(
            !f.log
                .iter()
                .any(|l| l.contains("--force") || l.contains("-N 2"))
        );
    }

    #[test]
    fn encrypted_root_is_luks_with_a_key_file_never_an_argument() {
        let d = windows();
        let l = alongside();
        let c = compile(&l, &d, true).unwrap();
        let mut f = Fake::new(d.clone());
        let a = apply(&mut f, &l, &c, &d, "/run/k").unwrap();
        let w = f.writes().join("\n");
        assert!(w.contains("cryptsetup luksFormat --batch-mode --type luks2 --pbkdf argon2id --key-file=/run/k /dev/vda5"), "{w}");
        assert!(w.contains("mkfs.btrfs -f -L WolOS /dev/mapper/wolosroot"));
        assert_eq!(a.fsdev, "/dev/mapper/wolosroot");
        assert_eq!(a.root, "/dev/vda5");
        assert!(
            a.crypttab.is_empty(),
            "only the system: nothing for crypttab"
        );
    }

    /// Encrypted install with a new /home and swap: each gets its own LUKS volume under its own
    /// mapper name, the file system goes on the mapper device, and install-run gets a crypttab line
    /// per volume (the key file lives in the encrypted system). The startup partition stays plain.
    #[test]
    fn encrypted_extra_volumes_get_their_own_lock_and_a_crypttab_line() {
        let d = windows();
        let mut l = alongside();
        l.create[1].bytes = Some(40 * GB);
        l.create.push(New {
            bytes: Some(20 * GB),
            mount: "/home".into(),
            fs: Fs::Ext4,
        });
        l.create.push(New {
            bytes: Some(2 * GB),
            mount: "swap".into(),
            fs: Fs::Swap,
        });
        let c = compile(&l, &d, true).unwrap();
        let mut f = Fake::new(d.clone());
        let a = apply(&mut f, &l, &c, &d, "/run/k").unwrap();
        let w = f.writes().join("\n");
        for (dev, name) in [("/dev/vda6", "woloshome"), ("/dev/vda7", "wolosswap")] {
            assert!(w.contains(&format!("--key-file=/run/k {dev}")), "{w}");
            assert!(
                w.contains(&format!("cryptsetup open --key-file=/run/k {dev} {name}")),
                "{w}"
            );
        }
        assert!(
            w.contains("mkfs.ext4 -F -L Home /dev/mapper/woloshome"),
            "{w}"
        );
        assert!(w.contains("mkswap -L Swap /dev/mapper/wolosswap"), "{w}");
        assert!(
            w.contains("mkfs.vfat -F 32 -n WOLOS /dev/vda4"),
            "plain ESP: {w}"
        );
        assert!(
            !w.contains("/dev/vda4 wolos"),
            "the startup partition is never locked: {w}"
        );
        assert_eq!(a.home, "/dev/mapper/woloshome");
        assert_eq!(
            a.fstab,
            [
                "UUID=FS-UUID /home ext4 defaults 0 2",
                "UUID=FS-UUID none swap defaults 0 0"
            ]
        );
        assert_eq!(
            a.crypttab,
            [
                "woloshome UUID=FS-UUID /etc/cryptsetup-keys.d/woloshome.key luks",
                "wolosswap UUID=FS-UUID /etc/cryptsetup-keys.d/wolosswap.key luks"
            ]
        );
        let env = a.env();
        assert!(env.contains("home_dev=/dev/mapper/woloshome\n"), "{env}");
        assert!(
            env.contains("extra_crypttab=woloshome UUID=FS-UUID /etc/cryptsetup-keys.d/woloshome.key luks\nextra_crypttab=wolosswap"),
            "{env}"
        );
        // Not encrypted: plain partitions, no crypttab.
        let c = compile(&l, &d, false).unwrap();
        let mut f = Fake::new(d.clone());
        let a = apply(&mut f, &l, &c, &d, "/run/k").unwrap();
        assert!(!f.writes().join("\n").contains("cryptsetup"));
        assert_eq!(a.home, "/dev/vda6");
        assert!(a.crypttab.is_empty());
        assert!(!a.env().contains("extra_crypttab"), "{}", a.env());
        assert_eq!(mapper_for("/games"), "wolosgames");
    }

    /// audit core-install #2: an encrypted layout that stops after it opened volumes closes every
    /// one of them (the last first), so "Try Again" can format the same partitions again; a
    /// volume that never opened is not closed.
    #[test]
    fn a_later_step_that_stops_leaves_no_volume_open() {
        let d = windows();
        let mut l = alongside();
        l.create[1].bytes = Some(40 * GB);
        l.create.push(New {
            bytes: Some(20 * GB),
            mount: "/home".into(),
            fs: Fs::Ext4,
        });
        l.create.push(New {
            bytes: Some(2 * GB),
            mount: "swap".into(),
            fs: Fs::Swap,
        });
        let c = compile(&l, &d, true).unwrap();
        for (stop, open) in [
            ("mkswap", &["wolosroot", "woloshome", "wolosswap"][..]),
            ("mkfs.ext4", &["wolosroot", "woloshome"][..]),
            (
                "cryptsetup open --key-file=/run/k /dev/vda7",
                &["wolosroot", "woloshome"][..],
            ),
            (
                "btrfs subvolume create",
                &["wolosroot", "woloshome", "wolosswap"][..],
            ),
        ] {
            let mut f = Fake::new(d.clone());
            f.refuse = Some(stop);
            assert!(
                matches!(apply(&mut f, &l, &c, &d, "/run/k"), Err(Stop::Step(_))),
                "{stop}"
            );
            let closed: Vec<&str> = f
                .log
                .iter()
                .filter_map(|x| x.strip_prefix("cryptsetup close "))
                .collect();
            let mut want = open.to_vec();
            want.reverse();
            assert_eq!(closed, want, "{stop}: {:#?}", f.log);
            let last_open = f.log.iter().rposition(|x| x.starts_with("cryptsetup open"));
            let first_close = f.log.iter().position(|x| x.starts_with("cryptsetup close"));
            assert!(
                last_open < first_close,
                "{stop}: closed after the last open"
            );
        }
        // A run that finishes leaves its volumes open for install-run.
        let mut f = Fake::new(d.clone());
        apply(&mut f, &l, &c, &d, "/run/k").unwrap();
        assert!(!f.log.iter().any(|x| x.starts_with("cryptsetup close")));
    }

    #[test]
    fn manual_keeps_home_as_it_is_and_says_where_it_goes() {
        let mut d = windows();
        d.parts[1].fstype = "ext4".into();
        d.parts[1].kind = DATA_TYPE.into();
        let l = Layout {
            assign: vec![Assign {
                part: "u2".into(),
                mount: "/home".into(),
                format: None,
            }],
            create: vec![
                New {
                    bytes: Some(GB),
                    mount: "/efi".into(),
                    fs: Fs::Vfat,
                },
                New {
                    bytes: Some(30 * GB),
                    mount: "/".into(),
                    fs: Fs::Ext4,
                },
                New {
                    bytes: Some(2 * GB),
                    mount: "swap".into(),
                    fs: Fs::Swap,
                },
            ],
            ..Layout::default()
        };
        let c = compile(&l, &d, false).unwrap();
        let mut f = Fake::new(d.clone());
        let a = apply(&mut f, &l, &c, &d, "k").unwrap();
        let w = f.writes().join("\n");
        assert!(
            !w.contains("/dev/vda2"),
            "the kept partition is never named: {w}"
        );
        assert!(!w.contains("subvolume"), "one copy: no slots");
        assert_eq!(a.home, "/dev/vda2");
        assert!(!a.slots_two);
        assert_eq!(
            a.fstab,
            [
                "UUID=FS-UUID /home ext4 defaults 0 2",
                "UUID=FS-UUID none swap defaults 0 0"
            ]
        );
        let env = a.env();
        assert!(
            env.contains("slots=one\n") && env.contains("home_dev=/dev/vda2\n"),
            "{env}"
        );
    }

    #[test]
    fn a_reused_startup_partition_is_named_but_not_written() {
        let mut d = windows();
        d.parts[0].free_bytes = Some(800_000_000);
        let l = Layout {
            assign: vec![Assign {
                part: "u1".into(),
                mount: "/efi".into(),
                format: None,
            }],
            create: vec![New {
                bytes: Some(40 * GB),
                mount: "/".into(),
                fs: Fs::Btrfs,
            }],
            ..Layout::default()
        };
        let c = compile(&l, &d, false).unwrap();
        let mut f = Fake::new(d.clone());
        let a = apply(&mut f, &l, &c, &d, "k").unwrap();
        assert_eq!(a.esp, "/dev/vda1");
        assert!(!f.writes().join("\n").contains("mkfs.vfat"));
        // The plan decides "shared" for boot-setup, not a file on the partition (audit3 R2B-4).
        assert!(a.esp_reused && a.others && a.windows && !a.esp_wolos && !a.bitlocker);
        let env = a.env();
        for w in [
            "esp_reused=1\n",
            "others=1\n",
            "windows=1\n",
            "esp_wolos=0\n",
        ] {
            assert!(env.contains(w), "{w}: {env}");
        }
    }

    /// audit3 R2B-2: replacing the only other system leaves no menu timeout behind; Windows'
    /// recovery partition alone is no system. R2B-5: an old WolOS startup partition is reused and
    /// install-run is told to clear it.
    #[test]
    fn replacing_the_only_other_system_leaves_nothing_to_choose() {
        let mut d = windows();
        d.parts[0].free_bytes = Some(800_000_000);
        d.parts[0].wolos = true;
        d.parts[1].fstype = "btrfs".into();
        d.parts[1].kind = DATA_TYPE.into();
        d.parts[1].name = "WolOS".into();
        let l = wol_core::install::plan::replace(&d, "u2");
        let c = compile(&l, &d, false).unwrap_or_else(|r| panic!("{r:?}"));
        let mut f = Fake::new(d.clone());
        let a = apply(&mut f, &l, &c, &d, "k").unwrap();
        assert!(!a.others, "{a:?}");
        assert!(!a.windows);
        assert!(a.esp_reused && a.esp_wolos, "{a:?}");
        assert!(a.env().contains("others=0\n"), "{}", a.env());
    }

    #[test]
    fn settings_come_from_checked_values_only() {
        let mut p = Plan {
            locale: "pt_BR.UTF-8".into(),
            keyboard: "br".into(),
            zone: "America/Sao_Paulo".into(),
            ..Plan::default()
        };
        let f = settings_files(&p).unwrap();
        assert_eq!(
            f[0],
            (
                "etc/locale.conf".to_string(),
                "LANG=pt_BR.UTF-8\n".to_string()
            )
        );
        assert_eq!(f[1].1, "KEYMAP=br-abnt2\n");
        assert!(f[2].1.contains("pt_BR.UTF-8 UTF-8\n"));
        p.zone = "../../etc/shadow".into();
        assert!(settings_files(&p).is_none());
        p.zone = "UTC".into();
        p.locale = "x; rm".into();
        assert!(settings_files(&p).is_none());
    }

    /// QA-SET-27: Settings > Language & Region offers every `LANGUAGES` entry, so the installed
    /// system has them all, whatever language the installer was run in (en_US included).
    #[test]
    fn every_language_in_locale_gen() {
        for chosen in ["en_US.UTF-8", "pt_BR.UTF-8", "ja_JP.UTF-8"] {
            let p = Plan {
                locale: chosen.into(),
                keyboard: "us".into(),
                zone: "UTC".into(),
                ..Plan::default()
            };
            let gen_file = &settings_files(&p).unwrap()[2];
            assert_eq!(gen_file.0, "etc/locale.gen");
            for (l, _) in region::LANGUAGES {
                assert!(
                    gen_file.1.contains(&format!("{l} UTF-8\n")),
                    "{chosen}: {l}"
                );
            }
            assert!(gen_file.1.contains(&format!("{chosen} UTF-8\n")));
        }
    }

    /// The default US layout the app ships is `us(intl)`: a plan with it must install.
    #[test]
    fn xkb_layouts_with_a_variant_install() {
        for ok in ["us", "us(intl)", "de(nodeadkeys)", "latam", "ch(fr)"] {
            assert!(is_valid_keyboard(ok), "{ok}");
        }
        for bad in [
            "", "u", "US", "us(intl", "us)", "us()", "a(b)(c)", "../x", "us intl",
        ] {
            assert!(!is_valid_keyboard(bad), "{bad}");
        }
        let p = Plan {
            locale: "en_US.UTF-8".into(),
            keyboard: "us(intl)".into(),
            zone: "UTC".into(),
            ..Plan::default()
        };
        let f = settings_files(&p).unwrap();
        assert_eq!(f[1].1, "KEYMAP=us\n", "the console has no dead keys");
    }

    #[test]
    fn plan_problems_are_found_before_the_first_write() {
        let passwd = "root:x:0:0::/root:/bin/sh\nlive:x:1000:1000::/home/live:/bin/zsh\n\
                      _xguard:x:970:970::/:/usr/bin/nologin\n";
        let mut p = Plan {
            locale: "en_US.UTF-8".into(),
            keyboard: "us".into(),
            zone: "UTC".into(),
            account: Some(Account {
                name: "alice".into(),
                full_name: "Alice".into(),
                tone: "green".into(),
            }),
            ..Plan::default()
        };
        assert_eq!(plan_problem(&p, passwd, Some("curiouser\n")), None);
        assert_eq!(
            plan_problem(&p, passwd, Some("short\n")),
            Some("WeakPassword")
        );
        assert_eq!(plan_problem(&p, passwd, None), Some("WeakPassword"));
        for taken in ["live", "_xguard", "root"] {
            p.account.as_mut().unwrap().name = taken.into();
            assert_eq!(
                plan_problem(&p, passwd, Some("curiouser")),
                Some("ReservedName"),
                "{taken} exists on the live image, so on the new system"
            );
        }
        // With the chip the account password is the disk's fallback, typed at the start prompt.
        p.account.as_mut().unwrap().name = "alice".into();
        p.protect = Protect::Tpm;
        assert_eq!(
            plan_problem(&p, passwd, Some("caf\u{e9}12345\n")),
            Some("UntypablePassword")
        );
        assert_eq!(plan_problem(&p, passwd, Some("curiouser\n")), None);
        p.protect = Protect::Password;
        assert_eq!(plan_problem(&p, passwd, Some("caf\u{e9}12345\n")), None);
        p.account.as_mut().unwrap().name = "Alice".into();
        assert_eq!(plan_problem(&p, passwd, Some("curiouser")), Some("BadName"));
        p.account = None;
        assert_eq!(
            plan_problem(&p, passwd, None),
            None,
            "no account: nothing to check"
        );
        p.keyboard = "US".into();
        assert_eq!(plan_problem(&p, passwd, None), Some("BadSettings"));
    }

    #[test]
    fn the_plans_extras_reach_install_run_checked_with_defaults() {
        let d = Extras::default();
        assert_eq!(
            (d.hostname.as_str(), d.autologin, d.swap.as_str()),
            ("wolos", false, "zram")
        );
        assert_eq!(Extras::from_json("{}"), d);
        assert_eq!(Extras::from_json("not json"), d);
        let x =
            Extras::from_json(r#"{"hostname":"alices-pc","autologin":true,"swap":"swapfile:8"}"#);
        assert_eq!(
            (x.hostname.as_str(), x.autologin, x.swap.as_str()),
            ("alices-pc", true, "swapfile:8")
        );
        // A bad value falls back rather than reaching a shell line.
        let bad = Extras::from_json(
            r#"{"hostname":"bad host;rm","autologin":"yes","swap":"swapfile:0"}"#,
        );
        assert_eq!(bad, d);
        for ok in ["pc", "alice-pc", "x1"] {
            assert!(is_valid_hostname(ok), "{ok}");
        }
        // One rule with Settings > Sharing (audit3 R2B-18): one lowercase label.
        let long = "x".repeat(64);
        for bad in [
            "",
            "-pc",
            "pc-",
            "a..b",
            "a.b.c",
            "Alice-PC",
            "a_b",
            "é",
            long.as_str(),
        ] {
            assert!(!is_valid_hostname(bad), "{bad}");
        }
        assert!(is_valid_swap("none") && is_valid_swap("zram") && is_valid_swap("swapfile:1"));
        assert!(
            !is_valid_swap("swapfile:65") && !is_valid_swap("swapfile") && !is_valid_swap("file")
        );
        let p = Plan {
            disk: "/dev/vda".into(),
            account: Some(Account {
                name: "alice".into(),
                ..Account::default()
            }),
            ..Plan::default()
        };
        assert_eq!(
            info_text(&p, &x),
            "disk=/dev/vda\nwipe=0\nWOLOS_ENCRYPT=no\naccount=alice\nWOLOS_HOSTNAME=alices-pc\nWOLOS_AUTOLOGIN=yes\nWOLOS_SWAP=swapfile:8\nWOLOS_SECURE=yes\nmodules=\nextra_packages=\n"
        );
    }

    #[test]
    fn the_installers_new_pages_reach_install_run_as_checked_lines() {
        let p = Plan {
            disk: "/dev/vda".into(),
            secure_boot: Some(false),
            modules: vec!["dev".into(), "x;reboot".into(), "creative".into()],
            extras: vec!["codecs-kit".into()],
            ..Plan::default()
        };
        let t = info_text(&p, &Extras::default());
        let line = |k: &str| {
            t.lines()
                .find_map(|l| l.strip_prefix(k)?.strip_prefix('='))
                .unwrap_or_else(|| panic!("no {k} in {t}"))
                .to_string()
        };
        assert_eq!(line("WOLOS_SECURE"), "no");
        assert_eq!(
            line("modules"),
            "dev creative",
            "a bad id never reaches the shell"
        );
        let pk = line("extra_packages");
        assert!(pk.starts_with("gst-plugins-good lame "), "MP3 first: {pk}");
        assert!(
            pk.split(' ').all(wol_core::store::is_pkg_name),
            "only package names: {pk}"
        );
        let asked = Plan {
            secure_boot: Some(true),
            ..Plan::default()
        };
        assert!(info_text(&asked, &Extras::default()).contains("WOLOS_SECURE=yes\n"));
    }

    #[test]
    fn the_account_is_the_setup_assistants_account() {
        let p = Plan {
            locale: "en_US.UTF-8".into(),
            keyboard: "us".into(),
            account: Some(Account {
                name: "alice".into(),
                full_name: "Alice".into(),
                tone: "green".into(),
            }),
            protect: Protect::Off,
            ..Plan::default()
        };
        let a = new_account(&p, "pw").unwrap();
        assert_eq!(
            (a.name.as_str(), a.avatar.as_str(), a.password.as_str()),
            ("alice", "green", "pw")
        );
        assert_eq!(a.appearance, "auto");
        assert!(login::setup_plan(&a).len() > 3);
    }
}
