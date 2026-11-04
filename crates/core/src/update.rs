// SPDX-License-Identifier: BSD-2-Clause
//! System update with A/B root slots (ADR 0016): snapshot the running root into the other slot,
//! pin it to the new release's Arch Linux Archive date, `pacman -Syu` inside it, rebuild its
//! initramfs, signed kernel image and boot entry (boot counting), boot it next. A failed slot falls
//! back on its own: no default is pinned, the newest entry boots and a bad one sorts last.
//! Packages the person installed with
//! pacman come along: the slot is a copy of the running root, never an image.
//! Pure: produces argv lists; the caller runs them in order and stops on the first failure,
//! then runs [`rollback`] (the new slot is deleted; the running slot stays the restore point).

pub mod auto;
pub mod check;
pub mod layers;
pub mod pkg;

use std::path::Path;

/// The two root slots (btrfs subvolumes `@wolos-a`, `@wolos-b`).
pub const SLOTS: [&str; 2] = ["wolos-a", "wolos-b"];
/// The btrfs top level (subvolid=5), mounted by the image.
pub const TOP: &str = "/run/wolos/top";
/// The EFI system partition.
pub const ESP: &str = "/efi";
/// Kernel every slot boots (`linux-wolos`).
pub const KERNEL: &str = "vmlinuz-linux-wolos";
/// Boot counting: tries before systemd-boot falls back to the other slot.
pub const TRIES: u32 = 3;
/// pacman.conf: the Arch repos at one Arch Linux Archive date (`/repos/YYYY/MM/DD/`).
pub const PACMAN_CONF: &str = crate::mirror::PACMAN_CONF;
/// `[wolos]` servers, at the release date (`YYYY-MM-DD`, ADR 0019). The `[cachyos-*-v3]` sections
/// have their own mirrorlist (`/etc/pacman.d/wolos-cachy-mirrorlist`, ADR 0030) and are not dated.
pub const MIRRORLIST: &str = crate::mirror::MIRRORLIST;

/// Pending package names from `pacman -Qu` output (`name old -> new`, the update check's `pkg`
/// file); None when output unknown.
/// Inside one release only `[wolos]` moves: everything else is pinned to the snapshot date.
pub fn parse_pending(output: Option<&str>) -> Option<Vec<String>> {
    Some(
        output?
            .lines()
            .filter(|l| l.contains(" -> "))
            .filter_map(|l| l.split_whitespace().next().map(str::to_owned))
            .collect(),
    )
}

/// "2 package updates: a, b" / "+N more".
pub fn summary(names: &[String], max: usize) -> String {
    let shown: Vec<&str> = names.iter().take(max).map(String::as_str).collect();
    let mut s = format!("{} package updates: {}", names.len(), shown.join(", "));
    if names.len() > max {
        s.push_str(&format!(", +{} more", names.len() - max));
    }
    s
}

/// A root slot name ([`SLOTS`]).
pub fn is_valid_be(name: &str) -> bool {
    SLOTS.contains(&name)
}

/// Arch Linux Archive date `YYYY/MM/DD`.
pub fn is_snapshot_date(d: &str) -> bool {
    let b = d.as_bytes();
    b.len() == 10
        && b[4] == b'/'
        && b[7] == b'/'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// Running slot from `/proc/cmdline` (`rootflags=subvol=@wolos-a`).
pub fn running_slot(cmdline: &str) -> Option<&'static str> {
    let sub = cmdline.split_whitespace().find_map(|w| {
        w.strip_prefix("rootflags=")?
            .split(',')
            .find_map(|o| o.strip_prefix("subvol="))
    })?;
    let sub = sub.trim_start_matches('/').trim_start_matches('@');
    SLOTS.into_iter().find(|s| *s == sub)
}

/// The slot that is not `running`.
pub fn other_slot(running: &str) -> Option<&'static str> {
    is_valid_be(running).then(|| SLOTS.into_iter().find(|s| *s != running))?
}

/// Entry file name with the boot counter.
pub fn entry_file(slot: &str) -> String {
    format!("{ESP}/loader/entries/{slot}+{TRIES}.conf")
}

/// One update step: a fixed argv (never a shell) or a whole-file write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Run(Vec<String>),
    Write { path: String, text: String },
}

impl std::fmt::Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Step::Run(a) => f.write_str(&a.join(" ")),
            Step::Write { path, .. } => write!(f, "write {path}"),
        }
    }
}

/// Steps to update into `slot` (the inactive one). `replace`: the slot holds the previous
/// restore point, deleted first (slot count stays two). `date`: the new release's snapshot
/// (None = same release, `[wolos]` fixes only). The default entry always boots linux-wolos;
/// kernels the person added with pacman stay installed.
pub fn plan(slot: &str, replace: bool, date: Option<&str>) -> Option<Vec<Step>> {
    plan_for(slot, replace, date, None)
}

/// [`plan`] on a running system whose `/proc/cmdline` is `cmdline`: the new slot's entry keeps its
/// root words (an encrypted disk stays unlockable). The finished slot is sealed (read-only,
/// fs-verity digests recorded on the ESP) before its entry exists.
pub fn plan_for(
    slot: &str,
    replace: bool,
    date: Option<&str>,
    cmdline: Option<&str>,
) -> Option<Vec<Step>> {
    plan_from(slot, Base::Running { replace }, date, cmdline, &[])
}

/// [`plan_for`] from a chosen base: [`Base::Pending`] stacks the update onto a slot that waits
/// for its first boot (a kernel installed with pacman), so the update keeps that change instead
/// of replacing the slot with a fresh copy of the running root. `holds`: package names that stay
/// at their version (QA-STORE-49; validated names, as wol-admind's `pkg-upgrade` arguments). None
/// on a bad slot or date.
pub fn plan_from(
    slot: &str,
    from: Base,
    date: Option<&str>,
    cmdline: Option<&str>,
    holds: &[String],
) -> Option<Vec<Step>> {
    if date.is_some_and(|d| !is_snapshot_date(d)) {
        return None;
    }
    let release = date.unwrap_or("current").to_owned();
    let date = date.map(str::to_owned);
    let prep = move |root: &str| repo_date_steps(date.as_deref(), root);
    let args = pkg::update_now_args(holds);
    let pacman: Vec<&str> = args.iter().map(String::as_str).collect();
    build(&Build {
        slot,
        from,
        prep: &prep,
        pacman: &pacman,
        files: &[],
        release: &release,
        cmdline,
    })
}

/// Moves the repos in `root` to release date `d`.
fn repo_date_steps(d: Option<&str>, root: &str) -> Vec<Step> {
    let Some(d) = d else {
        return Vec::new();
    };
    let sed = |expr: String, file: &str| {
        Step::Run(
            ["sed", "-i", "-E", &expr, &format!("{root}{file}")]
                .map(String::from)
                .to_vec(),
        )
    };
    vec![
        sed(
            format!("s#/repos/[0-9]{{4}}/[0-9]{{2}}/[0-9]{{2}}/#/repos/{d}/#"),
            PACMAN_CONF,
        ),
        // [wolos] moves with Arch: both at one release date (the [cachyos-*-v3] mirrorlist has no date).
        sed(
            format!(
                "s#[0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}}#{}#g",
                d.replace('/', "-")
            ),
            MIRRORLIST,
        ),
    ]
}

/// Marks a system installed with one copy (a layout without btrfs, ADR 0023).
pub const SINGLE_COPY: &str = "/etc/wolos/single-copy";

/// Steps to update a one-copy system: in place, with nothing to go back to and no seal. The
/// pacman hooks rebuild the initramfs and sync the startup partition.
pub fn plan_in_place(date: Option<&str>) -> Option<Vec<Step>> {
    if date.is_some_and(|d| !is_snapshot_date(d)) {
        return None;
    }
    let mut steps = repo_date_steps(date, "");
    steps.push(Step::Run(
        [PACMAN, "-Syu", "--noconfirm"].map(String::from).to_vec(),
    ));
    Some(steps)
}

/// Scratch subvolume a stacked transaction is built in (the pending slot stays until it is done).
pub const SCRATCH: &str = "wolos-tx";
/// The real pacman (the `pacman` on PATH is the transaction wrapper, `/usr/local/bin`).
pub const PACMAN: &str = "/usr/bin/pacman";

/// What a new slot starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    /// A copy of the running root; `replace`: the slot holds an old restore point, deleted first.
    Running { replace: bool },
    /// A copy of the slot itself (it waits for its first boot): the change stacks onto it.
    Pending,
}

/// The `/etc` overlay of a two-slot system (fstab line). The lower is the slot's own `/etc`, a
/// different directory after every slot switch, while the upper in the shared `@var` stays.
/// overlayfs defines changes to a lower layer made while it is not mounted only when the index,
/// metacopy, xino and redirect features were not used (Documentation/filesystems/overlayfs.rst,
/// "Changes to underlying filesystems"). With the kernel default `index=on` the mount refused the
/// upper after a switch: its root origin named the other slot's `/etc` (ESTALE, emergency mode).
/// `redirect_dir=follow` makes no redirects and still reads those an older upper has;
/// [`ETC_SETTLE`] turns metacopy-only copy-ups into full ones first. Nothing is stripped from the
/// upper: a slot from before this line (`index=on`) still finds its own origin there.
pub const ETC_FSTAB: &str = "overlay /etc overlay lowerdir=/etc,upperdir=/var/lib/wolos/etc/upper,\
workdir=/var/lib/wolos/etc/work,index=off,metacopy=off,redirect_dir=follow,xino=off,nfs_export=off,\
x-systemd.requires-mounts-for=/var 0 0";
/// Copies the data of metacopy-only copy-ups in the `/etc` upper up through the live overlay, so a
/// mount with `metacopy=off` (the new slot's [`ETC_FSTAB`]) reads them.
pub const ETC_SETTLE: &str = "/usr/lib/wolos/etc-settle";
/// The slot's own letter (`a` or `b`), in its sealed `/etc`: written by install-run for the first
/// slot and by every slot build for the new one (the copy still names the slot it came from).
/// esp-sync reads it when it is given a root but no slot.
pub const SLOT_FILE: &str = "/etc/wolos/slot";

/// [`SLOT_FILE`]'s text for a slot of [`SLOTS`].
pub fn slot_file_text(slot: &str) -> String {
    format!("{}\n", slot.strip_prefix("wolos-").unwrap_or(slot))
}

/// Where [`Build::files`] are inside the copy, one folder per file (a signature next to it keeps
/// its name).
pub const STAGE: &str = "/var/tmp/wolos-pkg";

/// The path pacman sees for file `i` of [`Build::files`], `src` on the running system.
pub fn staged(i: usize, src: &str) -> String {
    let name = Path::new(src)
        .file_name()
        .map_or_else(|| src.to_owned(), |n| n.to_string_lossy().into_owned());
    format!("{STAGE}/{i}/{name}")
}

/// `src`'s folder and a `find -name` pattern for its detached signature (`NAME.sig`).
fn sig_pattern(src: &str) -> (String, String) {
    let p = Path::new(src);
    let dir = match p.parent().map(|d| d.to_string_lossy().into_owned()) {
        Some(d) if d.starts_with(['/', '.']) => d,
        Some(d) if !d.is_empty() => format!("./{d}"),
        _ => ".".to_owned(),
    };
    let name = p
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let mut pat = String::new();
    for c in name.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '\\') {
            pat.push('\\');
        }
        pat.push(c);
    }
    (dir, pat + ".sig")
}

/// One change to a slot: the one implementation `wol update` and the pacman wrapper share.
pub struct Build<'a> {
    pub slot: &'a str,
    pub from: Base,
    /// Steps between the snapshot and pacman, given the working root (repo dates).
    pub prep: &'a dyn Fn(&str) -> Vec<Step>,
    /// Arguments of the real pacman, run in the slot.
    pub pacman: &'a [&'a str],
    /// Package files on the running system that `pacman` names by [`staged`] path: copied into
    /// the copy first (the chroot has no `/var`, `/home` or `/tmp` of the running system, and
    /// starts in `/`), removed again before the seal.
    pub files: &'a [String],
    pub release: &'a str,
    pub cmdline: Option<&'a str>,
}

/// Held (`flock`) for the whole of one change to a slot or a layer (`wol update apply`, the pacman
/// wrapper): two at once would delete or snapshot over what the other is building (audit8
/// stability2 #2). Under `/run`: a crash or restart never leaves it held.
pub const LOCK: &str = "/run/wolos/update.lock";
/// Said when [`LOCK`] is taken.
pub const BUSY: &str =
    "Another system update or package change is running. Try again when it has finished.";

/// Builds the slot's signed kernel image and its counted boot entry.
pub const ESP_SYNC: &str = "/usr/lib/wolos/esp-sync";

/// Snapshot, change, rebuild the initramfs, seal, build the boot image, boot it next.
pub fn build(b: &Build) -> Option<Vec<Step>> {
    let slot = b.slot;
    if !is_valid_be(slot) {
        return None;
    }
    let v = |a: &[&str]| Step::Run(a.iter().map(|s| (*s).to_owned()).collect());
    let root = format!("{TOP}/@{slot}");
    // QA-STORE-3: every change is built aside, in the scratch copy; the slot (the earlier restore
    // point, or a change that waits for its restart) is touched only once the new one is complete
    // and sealed ([`swap_steps`]). A cut or a failed step before that leaves the slot as it was.
    let work = format!("{TOP}/@{SCRATCH}");
    let mut steps = vec![Step::Write {
        path: BUILDING.into(),
        text: format!("{slot} {}\n", b.release),
    }];
    // A scratch copy a cut left behind (QA-STORE-8, QA-LIVE4-8: the next try failed with "File
    // exists"): unmounted and deleted first; nothing to do when there is none.
    steps.extend(clear_scratch());
    let from = match b.from {
        Base::Running { .. } => "/",
        Base::Pending => root.as_str(),
    };
    steps.push(v(&["btrfs", "subvolume", "snapshot", from, &work]));
    // Packages installed live since: their files and database entries come along, then the
    // layers are cleared from the copy.
    steps.push(v(&["wol", "layers", "fold", &work]));
    // The new slot mounts /etc so that its own, different lower is never refused (ETC_FSTAB), and
    // the upper holds nothing that mount would refuse.
    steps.push(v(&[
        "sed",
        "-i",
        "-E",
        &format!("s#^overlay[[:space:]]+/etc[[:space:]]+overlay[[:space:]].*#{ETC_FSTAB}#"),
        &format!("{work}/etc/fstab"),
    ]));
    // The copy is the running slot: it names itself until told otherwise (UPG-0121).
    steps.push(Step::Write {
        path: format!("{work}{SLOT_FILE}"),
        text: slot_file_text(slot),
    });
    steps.push(v(&[ETC_SETTLE]));
    // QA-LIVE4-13: the repos the copy updates from are the ones Settings and the daily check read
    // (the live /etc, with the person's own repos), not the sealed copy under the overlay.
    // A stacked change keeps the pending slot's own (its repos dated by the first change).
    if b.from != Base::Pending {
        steps.push(v(&[
            "cp",
            "--remove-destination",
            PACMAN_CONF,
            &format!("{work}{PACMAN_CONF}"),
        ]));
    }
    // QA-UPG-11: the copy's kernel command line names the slot it becomes (esp-sync reads the file
    // when a pacman hook runs it in that slot later).
    steps.push(v(&[
        "find",
        &format!("{work}/etc"),
        "-maxdepth",
        "2",
        "-path",
        &format!("{work}/etc/kernel/cmdline"),
        "-exec",
        "sed",
        "-i",
        "-E",
        &format!("s#subvol=/?@wolos-[ab]([ ,]|$)#subvol=@{slot}\\1#"),
        "{}",
        "+",
    ]));
    steps.extend((b.prep)(&work));
    let stage = format!("{work}{STAGE}");
    if !b.files.is_empty() {
        let dirs: Vec<String> = (0..b.files.len()).map(|i| format!("{stage}/{i}")).collect();
        let mut mkdir = vec!["mkdir", "-p"];
        mkdir.extend(dirs.iter().map(String::as_str));
        steps.push(v(&mkdir));
        for (src, dir) in b.files.iter().zip(&dirs) {
            steps.push(v(&["cp", "--reflink=auto", "-t", dir, "--", src]));
            // LocalFileSigLevel: a signature next to the file is checked, so it comes along.
            let (from, pat) = sig_pattern(src);
            steps.push(v(&[
                "find",
                "-L",
                &from,
                "-maxdepth",
                "1",
                "-type",
                "f",
                "-name",
                &pat,
                "-exec",
                "cp",
                "--reflink=auto",
                "-t",
                dir,
                "--",
                "{}",
                "+",
            ]));
        }
    }
    // pacman and mkinitcpio run in the copy through `wol update chroot`, each in a mount
    // namespace of its own (RH-192): the copy bound onto itself (pacman's CheckSpace needs a mount
    // point), the shared package cache (QA-STORE-46) and arch-chroot's mounts end with it.
    let mut pacman = vec![PACMAN];
    pacman.extend(b.pacman);
    steps.push(in_copy(&work, &pacman));
    steps.push(in_copy(&work, &["mkinitcpio", "-P"]));
    if !b.files.is_empty() {
        steps.push(v(&["rm", "-rf", "--", &stage]));
    }
    // Units a release adds or newly enables (wolos-etc-units.service, boot-ok's WantedBy) reach
    // an installed system only through its presets; install runs them once, an update did not.
    // enable-only: the slot never loses what it had on (the person's own changes live in the
    // /etc overlay above it and stay as they are). After the unbind: systemctl reads the folder.
    steps.push(v(&[
        "systemctl",
        &format!("--root={work}"),
        "preset-all",
        "--preset-mode=enable-only",
    ]));
    let esp = format!("{ESP}/{slot}");
    steps.push(v(&["mkdir", "-p", &esp]));
    // Next to the slot's record, not over it: until the swap the slot's own image is checked
    // against its own digest.
    let digest = crate::seal::digest_file(slot);
    let digest_new = format!("{digest}.new");
    steps.push(v(&["wol", "seal", "apply", &work, &digest_new]));
    steps.extend(swap_steps(slot));
    steps.push(v(&["mv", "-f", &digest_new, &digest]));
    // After the seal: the signed image carries the slot's root digest.
    steps.push(v(&[
        "env",
        &format!("WOLOS_ROOT={root}"),
        &format!("WOLOS_SLOT={slot}"),
        &format!("WOLOS_RELEASE={}", b.release),
        &format!(
            "WOLOS_CMDLINE={}",
            crate::encrypt::slot_options(b.cmdline, slot)
        ),
        ESP_SYNC,
        "--new",
    ]));
    // The new slot boots next from here on: no longer an interrupted update (review 5).
    steps.push(v(&["rm", "-f", BUILDING]));
    // A rollback the person chose earlier stops being the default: newest good entry boots.
    steps.push(v(&["bootctl", "set-default", ""]));
    Some(steps)
}

/// `unshare --mount --propagation private wol update chroot WORK -- CMD...`: CMD in the copy at
/// WORK, every mount it needs private to that one process.
pub fn in_copy(work: &str, cmd: &[&str]) -> Step {
    let mut a: Vec<String> = [
        "unshare",
        "--mount",
        "--propagation",
        "private",
        "wol",
        "update",
        "chroot",
    ]
    .map(String::from)
    .to_vec();
    a.push(work.to_owned());
    a.push("--".to_owned());
    a.extend(cmd.iter().map(|c| (*c).to_owned()));
    Step::Run(a)
}

/// Present while a slot is being built (`<slot> <release>`), removed at its end or by its undo:
/// found with no build running ([`interrupted`]), the last one was cut off (QA-LIVE4-14). In the
/// shared `/var`, so either slot sees it.
pub const BUILDING: &str = "/var/lib/wolos/update-building";

/// The package cache every slot shares (pacman's default `CacheDir`, in the shared `/var`).
pub const CACHE: &str = "/var/cache/pacman/pkg";

/// The slot an update was building when it was cut off: [`BUILDING`]'s text while no build holds
/// [`LOCK`] (`lock_held`). None when nothing was cut off or a build is running now.
pub fn interrupted(building: Option<&str>, lock_held: bool) -> Option<&str> {
    let slot = building?.split_whitespace().next()?;
    (!lock_held && is_valid_be(slot)).then_some(slot)
}

/// `find TOP -maxdepth 1 -name NAME -exec ARGV {} TERM`: ARGV on the subvolume `@NAME` only when
/// it exists. With `+` a failing ARGV fails the step; with `;` it never does (an unmount of what
/// is not mounted).
fn on_subvolume(name: &str, argv: &[&str], strict: bool) -> Step {
    let mut a: Vec<String> = ["find", TOP, "-maxdepth", "1", "-name"]
        .map(String::from)
        .to_vec();
    a.push(format!("@{name}"));
    a.push("-exec".into());
    a.extend(argv.iter().map(|s| (*s).to_owned()));
    a.push("{}".into());
    a.push(if strict { "+" } else { ";" }.into());
    Step::Run(a)
}

/// Unmounts and deletes the scratch copy if there is one.
fn clear_scratch() -> Vec<Step> {
    vec![
        on_subvolume(SCRATCH, &["umount", "-R"], false),
        on_subvolume(SCRATCH, &["btrfs", "subvolume", "delete"], true),
    ]
}

/// Every boot entry of `slot`: blessed (`wolos-b.conf`), counted (`wolos-b+3.conf`, `+2-1`) and
/// the other kernels' (`wolos-b-linux.conf`).
fn entries_of(slot: &str) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "find".into(),
        format!("{ESP}/loader/entries"),
        "-maxdepth".into(),
        "1".into(),
        "(".into(),
    ];
    for (i, pat) in ["", "+*", "-*"].iter().enumerate() {
        if i > 0 {
            a.push("-o".into());
        }
        a.push("-name".into());
        a.push(format!("{slot}{pat}.conf"));
    }
    a.push(")".into());
    a
}

/// The swap, once the new copy is complete and sealed: the slot's entries go first (no entry ever
/// names a slot that is not there, QA-STORE-3), then the slot itself, then the copy takes its name.
fn swap_steps(slot: &str) -> Vec<Step> {
    let mut entries = entries_of(slot);
    entries.push("-delete".into());
    vec![
        Step::Run(entries),
        // FAT keeps a removed entry until it is flushed: on the disk before the root goes.
        Step::Run(flush_esp()),
        on_subvolume(slot, &["btrfs", "subvolume", "delete"], true),
        Step::Run(vec![
            "mv".into(),
            format!("{TOP}/@{SCRATCH}"),
            format!("{TOP}/@{slot}"),
        ]),
    ]
}

/// `sync -f /efi`: what was written to or removed from the ESP is on the disk (FAT has no
/// journal; esp-sync flushes the same way).
fn flush_esp() -> Vec<String> {
    vec!["sync".into(), "-f".into(), ESP.into()]
}

/// Unbinds a copy a failed step left mounted (the bind for pacman, a chroot's mounts): a mounted
/// subvolume cannot be deleted. Fails harmlessly when nothing is mounted there.
fn unmount(root: &str) -> Vec<String> {
    vec!["umount".into(), "-R".into(), root.into()]
}

/// After a step before the swap failed: the scratch copy and the unswapped digest go; the slot,
/// its entries and its image were never touched.
pub fn rollback_from(slot: &str, _from: Base) -> Vec<Vec<String>> {
    let mut argv: Vec<Vec<String>> = clear_scratch()
        .into_iter()
        .filter_map(|s| match s {
            Step::Run(a) => Some(a),
            Step::Write { .. } => None,
        })
        .collect();
    argv.push(vec![
        "rm".into(),
        "-f".into(),
        format!("{}.new", crate::seal::digest_file(slot)),
        BUILDING.into(),
    ]);
    argv
}

/// How to undo a build whose step `failed` (an index into its `steps`) did not succeed.
#[derive(Debug, PartialEq, Eq)]
pub struct Undo {
    /// Run in order, each whatever the one before gave.
    pub argv: Vec<Vec<String>>,
    /// The slot still holds what it held before (else it is removed with its boot entry).
    pub kept: bool,
    /// The new slot is complete, sealed and has its entry (only a step after esp-sync failed):
    /// nothing is removed, it boots next unless a default chosen earlier still holds.
    pub done: bool,
}

/// [`rollback_from`], knowing which step failed. Before the swap ([`swap_steps`]) nothing of the
/// slot changed: kept. From the swap on, the slot's entries are gone first; what is left of it
/// (the old copy, or the new one without its image) is removed with its images, which no entry
/// names any more (QA-STORE-3: never an image an entry still names), so it is not kept.
pub fn rollback_at(slot: &str, from: Base, steps: &[Step], failed: usize) -> Undo {
    if steps
        .iter()
        .position(|s| matches!(s, Step::Run(a) if a.last().is_some_and(|x| x == "--new")))
        .is_some_and(|esp| failed > esp)
    {
        return Undo {
            argv: vec![vec!["rm".into(), "-f".into(), BUILDING.into()]],
            kept: false,
            done: true,
        };
    }
    let swap = swap_steps(slot);
    let started = steps
        .iter()
        .position(|s| *s == swap[0])
        .is_some_and(|at| failed >= at);
    if !started {
        return Undo {
            argv: rollback_from(slot, from),
            kept: true,
            done: false,
        };
    }
    let mut argv = rollback(slot);
    argv.extend(rollback_from(slot, from));
    Undo {
        argv,
        kept: false,
        done: false,
    }
}

/// A slot whose swap started and did not finish: its entries (all of them), then its images, then
/// its subvolume.
pub fn rollback(slot: &str) -> Vec<Vec<String>> {
    let mut entries = entries_of(slot);
    entries.push("-delete".into());
    vec![
        entries,
        flush_esp(),
        vec![
            "find".into(),
            format!("{ESP}/EFI/wolos"),
            "-maxdepth".into(),
            "1".into(),
            "(".into(),
            "-name".into(),
            format!("{slot}.efi"),
            "-o".into(),
            "-name".into(),
            format!("{slot}-*.efi"),
            ")".into(),
            "-delete".into(),
        ],
        unmount(&format!("{TOP}/@{slot}")),
        match on_subvolume(slot, &["btrfs", "subvolume", "delete"], true) {
            Step::Run(a) => a,
            Step::Write { .. } => Vec::new(),
        },
    ]
}

/// Restore points = the two root slots, as systemd-boot lists them.
pub mod be {
    use serde::{Deserialize, Serialize};

    /// `bootctl list --json=short`: every boot entry, the selected (running) and default one.
    pub const LIST_ARGV: [&str; 3] = ["bootctl", "list", "--json=short"];
    /// Slots kept: the running one and the previous one.
    pub const KEEP: usize = 2;

    /// `wol restore list --json` keys (QA-LIVE3-12): `tries_left` (a number; null once the slot
    /// booted fine) and `release` (the slot's release date, "current" for a slot built on the
    /// repositories of the day). `space` and `created` held those two under names that said
    /// otherwise; they stay for one release (0.13) as aliases with their old text, then go.
    #[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
    pub struct BootEnv {
        pub name: String,
        /// Running now.
        pub active: bool,
        /// Boots next.
        pub next_boot: bool,
        pub mountpoint: Option<String>,
        /// Boot tries left (empty when the slot booted fine).
        pub space: String,
        /// Release snapshot date of the slot (entry `version`).
        pub created: String,
    }

    impl BootEnv {
        /// Boot tries left while the slot waits for its first good boot; None once it booted.
        pub fn tries_left(&self) -> Option<u64> {
            self.space.split(' ').next()?.parse().ok()
        }
    }

    impl Serialize for BootEnv {
        fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeStruct;
            let mut m = s.serialize_struct("BootEnv", 8)?;
            m.serialize_field("name", &self.name)?;
            m.serialize_field("active", &self.active)?;
            m.serialize_field("next_boot", &self.next_boot)?;
            m.serialize_field("mountpoint", &self.mountpoint)?;
            m.serialize_field("release", &self.created)?;
            m.serialize_field("tries_left", &self.tries_left())?;
            // Aliases for one release (0.13), then removed.
            m.serialize_field("space", &self.space)?;
            m.serialize_field("created", &self.created)?;
            m.end()
        }
    }

    /// WolOS slots from `bootctl list --json=short`.
    pub fn parse(text: &str) -> Vec<BootEnv> {
        let Ok(serde_json::Value::Array(entries)) = serde_json::from_str(text) else {
            return Vec::new();
        };
        entries
            .iter()
            .filter_map(|e| {
                let id = e["id"].as_str()?;
                let name = id.trim_end_matches(".conf");
                let name = name.split_once('+').map_or(name, |(n, _)| n);
                if !super::is_valid_be(name) {
                    return None;
                }
                let active = e["isSelected"].as_bool().unwrap_or(false);
                Some(BootEnv {
                    name: name.into(),
                    active,
                    next_boot: e["isDefault"].as_bool().unwrap_or(false),
                    mountpoint: active.then(|| "/".into()),
                    space: e["triesLeft"]
                        .as_u64()
                        .map_or_else(String::new, |t| format!("{t} tries left")),
                    created: e["version"]
                        .as_str()
                        .map(|v| v.split_once('-').map_or(v, |(_, date)| date))
                        .unwrap_or("")
                        .into(),
                })
            })
            .collect()
    }

    /// Slot to go back to: the running one while an update waits for its first boot,
    /// otherwise the other slot.
    pub fn rollback_target(bes: &[BootEnv]) -> Option<&BootEnv> {
        let running = bes.iter().find(|b| b.active);
        if bes.iter().any(|b| b.next_boot && !b.active) {
            return running;
        }
        bes.iter().find(|b| !b.active)
    }

    /// Two slots only: nothing to prune.
    pub fn prune(_bes: &[BootEnv], _keep: usize) -> Vec<String> {
        Vec::new()
    }

    pub fn activate_argv(name: &str) -> Vec<String> {
        vec![
            "bootctl".into(),
            "set-default".into(),
            format!("{name}.conf"),
        ]
    }

    pub fn destroy_argv(name: &str) -> Vec<String> {
        vec![
            "btrfs".into(),
            "subvolume".into(),
            "delete".into(),
            format!("{}/@{name}", super::TOP),
        ]
    }

    /// The btrfs top level, listed: every slot's root subvolume is `@<name>` in it.
    pub const TOP_ARGV: [&str; 2] = ["ls", super::TOP];

    /// QA-STORE-5/U3: entries whose root subvolume is gone (an interrupted update leaves a boot
    /// entry behind) are not restore points. `top` is [`TOP_ARGV`]'s output; nothing is dropped
    /// unless it lists the running slot's own `@name`, so an unreadable or unmounted top level
    /// (None, or an empty automount) keeps the list as bootctl gives it.
    pub fn real(bes: Vec<BootEnv>, top: Option<&str>) -> Vec<BootEnv> {
        let Some(top) = top else { return bes };
        let has = |n: &str| top.lines().any(|l| l.trim().strip_prefix('@') == Some(n));
        if !bes.iter().any(|b| b.active && has(&b.name)) {
            return bes;
        }
        bes.into_iter().filter(|b| has(&b.name)).collect()
    }

    /// bootctl's list as boot-ok and esp-sync last left it for every account (root-written, 0644,
    /// replaced whole): an account's own `bootctl list` fails, the ESP is root's (QA-LIVE4-5).
    pub const SLOTS_FILE: &str = "/run/wolos/slots.json";

    /// The text of the slot list: `bootctl list` when it answers (root), else [`SLOTS_FILE`].
    /// One source for `wol restore` and Settings > Restore.
    pub fn list_text(run: Option<String>, file: Option<String>) -> Option<String> {
        run.or(file)
    }

    /// The restore points of this computer: bootctl's slots that still have a root subvolume.
    pub fn read(host: &dyn crate::system::Host) -> Option<Vec<BootEnv>> {
        let text = list_text(host.run(&LIST_ARGV), host.read_text(SLOTS_FILE))?;
        Some(real(parse(&text), host.run(&TOP_ARGV).as_deref()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_copy_updates_in_place_with_no_snapshot_and_no_seal() {
        let steps: Vec<String> = plan_in_place(Some("2026/11/01"))
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert!(steps[0].starts_with("sed") && steps.last().unwrap().contains("pacman -Syu"));
        assert!(
            !steps
                .iter()
                .any(|s| s.contains("btrfs") || s.contains("seal"))
        );
        assert_eq!(plan_in_place(None).unwrap().len(), 1);
        assert!(plan_in_place(Some("soon")).is_none());
    }

    #[test]
    fn pending_table() {
        let cases = [
            (None, None),
            (Some(""), Some("")),
            (
                Some("wol-shell 0.12.1-1 -> 0.12.2-1\nwolos-keyring 1-1 -> 2-1\n"),
                Some("wol-shell,wolos-keyring"),
            ),
            (Some("error: no database\n"), Some("")),
        ];
        for (out, want) in cases {
            assert_eq!(parse_pending(out).map(|l| l.join(",")).as_deref(), want);
        }
    }

    #[test]
    fn summary_table() {
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(summary(&n(&["a", "b"]), 5), "2 package updates: a, b");
        assert_eq!(
            summary(&n(&["a", "b", "c", "d", "e", "f", "g"]), 5),
            "7 package updates: a, b, c, d, e, +2 more"
        );
    }

    #[test]
    fn slots_from_the_kernel_command_line() {
        let c =
            "initrd=/wolos-a/initramfs root=LABEL=wolos rootflags=subvol=@wolos-a,compress=zstd rw";
        assert_eq!(running_slot(c), Some("wolos-a"));
        assert_eq!(running_slot("rootflags=subvol=/@wolos-b"), Some("wolos-b"));
        assert_eq!(running_slot("rootflags=subvol=@root"), None);
        assert_eq!(running_slot(""), None);
        assert_eq!(other_slot("wolos-a"), Some("wolos-b"));
        assert_eq!(other_slot("wolos-b"), Some("wolos-a"));
        assert_eq!(other_slot("x"), None);
        for (d, ok) in [
            ("2026/10/20", true),
            ("2026-10-20", false),
            ("2026/10/2", false),
            ("../10/2026", false),
        ] {
            assert_eq!(is_snapshot_date(d), ok, "{d}");
        }
    }

    #[test]
    fn update_snapshots_the_running_root_into_the_other_slot() {
        let p = plan("wolos-b", true, Some("2026/11/02")).unwrap();
        let s: Vec<String> = p.iter().map(Step::to_string).collect();
        let (tx, b) = ("/run/wolos/top/@wolos-tx", "/run/wolos/top/@wolos-b");
        assert_eq!(
            s,
            [
                "write /var/lib/wolos/update-building",
                "find /run/wolos/top -maxdepth 1 -name @wolos-tx -exec umount -R {} ;",
                "find /run/wolos/top -maxdepth 1 -name @wolos-tx -exec btrfs subvolume delete {} +",
                &format!("btrfs subvolume snapshot / {tx}"),
                &format!("wol layers fold {tx}"),
                &format!(
                    "sed -i -E s#^overlay[[:space:]]+/etc[[:space:]]+overlay[[:space:]].*#{ETC_FSTAB}# \
{tx}/etc/fstab"
                ),
                &format!("write {tx}/etc/wolos/slot"),
                "/usr/lib/wolos/etc-settle",
                &format!("cp --remove-destination /etc/pacman.conf {tx}/etc/pacman.conf"),
                &format!(
                    "find {tx}/etc -maxdepth 2 -path {tx}/etc/kernel/cmdline -exec sed -i -E \
s#subvol=/?@wolos-[ab]([ ,]|$)#subvol=@wolos-b\\1# {{}} +"
                ),
                &format!(
                    "sed -i -E s#/repos/[0-9]{{4}}/[0-9]{{2}}/[0-9]{{2}}/#/repos/2026/11/02/# \
{tx}/etc/pacman.conf"
                ),
                &format!(
                    "sed -i -E s#[0-9]{{4}}-[0-9]{{2}}-[0-9]{{2}}#2026-11-02#g \
{tx}/etc/pacman.d/wolos-mirrorlist"
                ),
                // RH-192: each in a mount namespace of its own (`wol update chroot` binds the copy
                // and the shared cache there); nothing is left mounted, not even after a cut.
                &format!(
                    "unshare --mount --propagation private wol update chroot {tx} -- \
/usr/bin/pacman -Syu --noconfirm"
                ),
                &format!(
                    "unshare --mount --propagation private wol update chroot {tx} -- mkinitcpio -P"
                ),
                &format!("systemctl --root={tx} preset-all --preset-mode=enable-only"),
                "mkdir -p /efi/wolos-b",
                &format!("wol seal apply {tx} /efi/wolos-b/verity.new"),
                // The swap: entries, then the old slot, then the new copy takes its name.
                "find /efi/loader/entries -maxdepth 1 ( -name wolos-b.conf -o -name wolos-b+*.conf \
-o -name wolos-b-*.conf ) -delete",
                "sync -f /efi",
                "find /run/wolos/top -maxdepth 1 -name @wolos-b -exec btrfs subvolume delete {} +",
                &format!("mv {tx} {b}"),
                "mv -f /efi/wolos-b/verity.new /efi/wolos-b/verity",
                &format!(
                    "env WOLOS_ROOT={b} WOLOS_SLOT=wolos-b WOLOS_RELEASE=2026/11/02 \
WOLOS_CMDLINE=root=LABEL=WolOS rootflags=subvol=@wolos-b ro quiet splash \
/usr/lib/wolos/esp-sync --new"
                ),
                "rm -f /var/lib/wolos/update-building",
                // No pinned default: the newest good entry boots, a bad one sorts last.
                "bootctl set-default ",
            ]
        );
        // First update: no previous slot; same release: no new date.
        let first = plan("wolos-a", false, None).unwrap();
        assert_eq!(
            first[3].to_string(),
            "btrfs subvolume snapshot / /run/wolos/top/@wolos-tx"
        );
        // Only the fstab line is rewritten: no repo date moves.
        assert!(
            !first.iter().any(
                |s| s.to_string().starts_with("sed ") && !s.to_string().ends_with("/etc/fstab")
            )
        );
        for (slot, date) in [("x", None), ("wolos-a", Some("2026-11-02")), ("", None)] {
            assert!(plan(slot, false, date).is_none(), "{slot} {date:?}");
        }
        // An encrypted system's new entry keeps the unlock words; the slot is read-only.
        let enc = plan_for(
            "wolos-b",
            false,
            None,
            Some("root=/dev/mapper/wolosroot rd.luks.name=U=wolosroot rw"),
        )
        .unwrap();
        let t: Vec<String> = enc.iter().map(Step::to_string).collect();
        let sync = t.iter().position(|s| s.contains("esp-sync")).unwrap();
        assert!(t[sync].contains("WOLOS_CMDLINE=root=/dev/mapper/wolosroot rd.luks.name=U=wolosroot rootflags=subvol=@wolos-b ro "), "{}", t[sync]);
        let seal = t
            .iter()
            .position(|s| s.starts_with("wol seal apply"))
            .unwrap();
        assert!(
            seal < sync,
            "the kernel image is built from the sealed slot"
        );
        // Holds stay at their version in the slot (QA-STORE-49).
        let held = plan_from("wolos-b", Base::Pending, None, None, &["gimp".to_owned()]).unwrap();
        assert!(held.iter().any(|s| {
            s.to_string()
                .ends_with("/usr/bin/pacman -Syu --noconfirm --ignore=gimp")
        }));
    }

    /// FIX-slot 0.12.1: the copy pacman runs in is a mount for exactly the chroot steps (pacman's
    /// CheckSpace looks the root up in the mount table), and is let go before the seal, the swap of
    /// a stacked change and the boot image; the new slot's /etc overlay line is rewritten first.
    #[test]
    fn the_copy_is_a_mount_only_while_pacman_and_mkinitcpio_run_in_it() {
        // QA-STORE-3: every change is built in the scratch copy. RH-192: the plan itself mounts
        // nothing; pacman and mkinitcpio each run under `unshare` in `wol update chroot`, which
        // binds the copy (and the shared cache) in that namespace only.
        let work = "/run/wolos/top/@wolos-tx";
        let private = format!("unshare --mount --propagation private wol update chroot {work} -- ");
        for from in [Base::Running { replace: true }, Base::Pending] {
            let s: Vec<String> = plan_from("wolos-b", from, None, None, &[])
                .unwrap()
                .iter()
                .map(Step::to_string)
                .collect();
            let at = |p: &str| {
                s.iter()
                    .position(|x| x.starts_with(p))
                    .unwrap_or_else(|| panic!("{p}: {s:?}"))
            };
            assert!(
                !s.iter()
                    .any(|x| x.starts_with("mount ") || x.starts_with("umount ")),
                "{s:?}"
            );
            let runs: Vec<usize> = (0..s.len())
                .filter(|&i| s[i].starts_with(&private))
                .collect();
            assert_eq!(runs.len(), 2, "{s:?}");
            assert!(s[runs[0]].ends_with("/usr/bin/pacman -Syu --noconfirm"));
            assert!(s[runs[1]].ends_with("-- mkinitcpio -P"));
            assert_eq!(runs[1], runs[0] + 1);
            assert!(runs[1] < at("wol seal apply") && runs[1] < at("env WOLOS_ROOT="));
            assert!(runs[1] < at(&format!("mv {work} ")), "{s:?}");
            let fstab = at("sed -i -E s#^overlay");
            assert!(
                s[fstab].ends_with(&format!(" {work}/etc/fstab")),
                "{}",
                s[fstab]
            );
            assert!(fstab < runs[0] && at(ETC_SETTLE) < runs[0], "{s:?}");
        }
    }

    /// UPG-0121: the new slot's sealed /etc names that slot, not the one it was copied from (the
    /// copy of slot a said `a` after booting b). Written before the seal, in the copy pacman runs in.
    #[test]
    fn the_new_slot_names_itself_before_the_seal() {
        let work = "/run/wolos/top/@wolos-tx";
        for (slot, from, letter) in [
            ("wolos-b", Base::Running { replace: true }, "b\n"),
            ("wolos-a", Base::Running { replace: false }, "a\n"),
            ("wolos-b", Base::Pending, "b\n"),
        ] {
            let p = plan_from(slot, from, None, None, &[]).unwrap();
            let at = |pre: &str| {
                p.iter()
                    .position(|s| s.to_string().starts_with(pre))
                    .unwrap_or_else(|| panic!("{pre}: {p:?}"))
            };
            let writes: Vec<(usize, &Step)> = p
                .iter()
                .enumerate()
                .filter(|(_, s)| matches!(s, Step::Write { path, .. } if path != BUILDING))
                .collect();
            assert_eq!(writes.len(), 1, "{p:?}");
            let (i, w) = writes[0];
            assert_eq!(
                *w,
                Step::Write {
                    path: format!("{work}/etc/wolos/slot"),
                    text: letter.into()
                }
            );
            assert!(
                at("wol layers fold") < i && i < at("wol seal apply"),
                "{p:?}"
            );
        }
        // The pacman wrapper's slot changes share the step.
        let args = ["-U".to_owned(), "/x.pkg.tar.zst".to_owned()];
        let w = pkg::plan(
            "wolos-a",
            Base::Running { replace: true },
            &args,
            "current",
            None,
        )
        .unwrap()
        .into_iter()
        .find(|s| matches!(s, Step::Write { path, .. } if path != BUILDING))
        .unwrap();
        assert_eq!(
            w.to_string(),
            "write /run/wolos/top/@wolos-tx/etc/wolos/slot"
        );
        assert_eq!(slot_file_text("wolos-a"), "a\n");
        // install-run names the first slot the same way.
        let run = include_str!("../../../files/usr/lib/wolos/install-run");
        assert!(
            run.contains(&format!("echo a > \"$T{SLOT_FILE}\"")),
            "install-run"
        );
    }

    /// The /etc overlay never uses what overlayfs needs an unchanged lower for.
    #[test]
    fn the_etc_overlay_takes_a_changed_lower() {
        let opts = ETC_FSTAB.split_whitespace().nth(3).unwrap();
        for o in [
            "lowerdir=/etc",
            "upperdir=/var/lib/wolos/etc/upper",
            "workdir=/var/lib/wolos/etc/work",
            "index=off",
            "metacopy=off",
            "redirect_dir=follow",
            "xino=off",
            "nfs_export=off",
            "x-systemd.requires-mounts-for=/var",
        ] {
            assert!(opts.split(',').any(|x| x == o), "{o}: {opts}");
        }
        assert!(ETC_FSTAB.starts_with("overlay /etc overlay ") && ETC_FSTAB.ends_with(" 0 0"));
        // One line, nothing sed would read as its own syntax in the replacement.
        assert!(!ETC_FSTAB.contains(['#', '&', '\\', '\n']), "{ETC_FSTAB}");
    }

    #[test]
    fn staged_files_keep_their_name_in_a_folder_of_their_own() {
        assert_eq!(
            staged(0, "/home/a/x.pkg.tar.zst"),
            "/var/tmp/wolos-pkg/0/x.pkg.tar.zst"
        );
        assert_eq!(
            staged(3, "x.pkg.tar.zst"),
            "/var/tmp/wolos-pkg/3/x.pkg.tar.zst"
        );
        for (src, dir, pat) in [
            ("/home/a/x.pkg.tar.zst", "/home/a", "x.pkg.tar.zst.sig"),
            ("x.pkg.tar.zst", ".", "x.pkg.tar.zst.sig"),
            ("sub/x", "./sub", "x.sig"),
            ("/x[1]*.zst", "/", "x\\[1\\]\\*.zst.sig"),
        ] {
            assert_eq!(sig_pattern(src), (dir.into(), pat.into()), "{src}");
        }
    }

    pub(crate) const BOOTCTL: &str = r#"[
 {"id":"wolos-a.conf","version":"2026/10/20","isDefault":false,"isSelected":true},
 {"id":"wolos-b+2-1.conf","version":"2026/11/02","isDefault":true,"isSelected":false,"triesLeft":2},
 {"id":"auto-efi-shell","isDefault":false,"isSelected":false}
]"#;

    #[test]
    fn slots_from_bootctl() {
        let b = be::parse(BOOTCTL);
        assert_eq!(b.len(), 2, "only WolOS slots");
        assert_eq!(
            (
                b[0].name.as_str(),
                b[0].active,
                b[0].next_boot,
                b[0].mountpoint.as_deref()
            ),
            ("wolos-a", true, false, Some("/"))
        );
        assert_eq!(
            (
                b[1].name.as_str(),
                b[1].next_boot,
                b[1].space.as_str(),
                b[1].created.as_str()
            ),
            ("wolos-b", true, "2 tries left", "2026/11/02")
        );
        // QA-LIVE3-12: the JSON keys say what they hold; the old names stay as aliases.
        assert_eq!((b[1].tries_left(), b[0].tries_left()), (Some(2), None));
        let j = serde_json::to_value(&b[1]).unwrap();
        assert_eq!(
            (j["tries_left"].as_u64(), j["release"].as_str()),
            (Some(2), Some("2026/11/02"))
        );
        assert_eq!(
            (j["space"].as_str(), j["created"].as_str()),
            (Some("2 tries left"), Some("2026/11/02"))
        );
        assert!(serde_json::to_value(&b[0]).unwrap()["tries_left"].is_null());
        assert!(be::parse("not json").is_empty());
        // Update applied, not rebooted yet: rollback = keep booting the running slot.
        assert_eq!(be::rollback_target(&b).unwrap().name, "wolos-a");
        let booted = be::parse(&BOOTCTL.replace("\"isDefault\":true", "\"isDefault\":false"));
        assert_eq!(be::rollback_target(&booted).unwrap().name, "wolos-b");
        assert!(be::prune(&b, be::KEEP).is_empty());
        // A generation leads the version so the newest sorts first; the date is what people see.
        let gen_ = be::parse(&BOOTCTL.replace("2026/11/02", "7-2026.11.02"));
        assert_eq!(gen_[1].created, "2026.11.02");
        assert_eq!(
            be::activate_argv("wolos-a").join(" "),
            "bootctl set-default wolos-a.conf"
        );
        assert_eq!(
            be::destroy_argv("wolos-b").join(" "),
            "btrfs subvolume delete /run/wolos/top/@wolos-b"
        );
    }

    #[test]
    fn read_from_host() {
        let h = crate::system::fake::FakeHost::new(false);
        assert_eq!(be::read(&h), None, "no bootctl");
        h.command("bootctl list --json=short", BOOTCTL);
        assert_eq!(
            be::read(&h).unwrap().len(),
            2,
            "top level unreadable: as bootctl says"
        );
        // QA-STORE-5: an interrupted update leaves wolos-b's boot entry without its subvolume.
        h.command("ls /run/wolos/top", "@wolos-a\nsnapshots\n");
        let names: Vec<_> = be::read(&h).unwrap().into_iter().map(|b| b.name).collect();
        assert_eq!(names, ["wolos-a"], "the ghost slot is not a restore point");
        h.command("ls /run/wolos/top", "@wolos-a\n@wolos-b\n");
        assert_eq!(be::read(&h).unwrap().len(), 2, "both subvolumes exist");
        // QA-LIVE4-5: an account's own bootctl fails on the root-only ESP; the list boot-ok and
        // esp-sync left in /run is read instead. Neither: nothing.
        let user = crate::system::fake::FakeHost::new(false);
        assert_eq!(be::read(&user), None, "no bootctl, no list");
        user.file(be::SLOTS_FILE, BOOTCTL);
        assert_eq!(be::read(&user).unwrap().len(), 2, "the list from /run");
    }

    #[test]
    fn ghost_slots_are_dropped_only_with_proof() {
        let both = be::parse(BOOTCTL);
        let only = |t: Option<&str>| -> Vec<String> {
            be::real(both.clone(), t)
                .into_iter()
                .map(|b| b.name)
                .collect()
        };
        let a = || vec!["wolos-a".to_string()];
        let ab = || vec!["wolos-a".to_string(), "wolos-b".to_string()];
        assert_eq!(
            only(None),
            ab(),
            "top level unreadable: bootctl's list stands"
        );
        assert_eq!(
            only(Some("")),
            ab(),
            "an unmounted (empty) top level proves nothing"
        );
        assert_eq!(
            only(Some("@wolos-b\n")),
            ab(),
            "the running slot is not there: do not trust it"
        );
        assert_eq!(only(Some("@wolos-a\n")), a());
        assert_eq!(only(Some("@wolos-a\n@wolos-b\n")), ab());
        assert_eq!(be::TOP_ARGV, ["ls", TOP]);
    }
}
