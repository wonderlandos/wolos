// SPDX-License-Identifier: BSD-2-Clause
//! Live installs: the sealed slot stays read-only, a package change lands in a layer. The real
//! pacman runs in a chroot whose `/usr` is an overlay over the live one (`/etc` is the live,
//! writable one), the overlay's upper directory is a systemd-sysext extension, and
//! `systemd-sysext refresh` merges it over `/usr` at once. The package database is wherever
//! pacman.conf's `DBPath` puts it ([`db_path`]): an installed slot keeps it in `/usr/lib/pacman`
//! (ADR 0016), so a layer carries its own database entries in its `usr/`.
//! Removing a package that is in the sealed base leaves overlay whiteouts in the layer.
//! The next system update folds the layers into the new slot ([`super::build`]) and names them
//! in the slot's [`FOLDED`]. The live layers stay while the old slot runs; when the new slot
//! boots, `wol layers clear` ([`to_clear`]) removes the ones it already holds; the layers that
//! stay are unmerged only for systemd-sysext's own swap ([`clear_steps`]).
//! Pure: argv lists, texts and decisions. `wol pkg` and `wol layers fold|clear` run them.
use super::{PACMAN, Step};

/// Extensions systemd-sysext looks in; each layer is a directory here.
pub const DIR: &str = "/var/lib/extensions";
pub const PREFIX: &str = "wolos-layer-";
/// Overlay work directories (same file system as [`DIR`]).
pub const WORK_DIR: &str = "/var/lib/wolos/layers";
/// The chroot the real pacman runs in.
pub const CHROOT: &str = "/run/wolos/layer-root";
/// What a layer holds besides `usr/`: the packages it added and removed.
pub const RECORD: &str = "wolos-layer.json";
/// Layers already copied into a slot (so a later fold does not undo that slot's own upgrades),
/// relative to the slot's root. In the slot's own sealed `/usr` (QA-STORE-44): not under `/var`
/// (the shared `@var` at boot) and not in `/etc`, which is the shared overlay at boot, so a
/// copy written there once lands in its upper directory and shadows every later slot's list.
/// Written by the fold before the seal, read-only afterwards: `wol layers clear` never writes
/// it (layer names are never handed out twice, [`next_index`], so a listed name that is gone
/// from [`DIR`] matches nothing again).
pub const FOLDED: &str = "usr/lib/wolos/folded-layers";
/// Where [`FOLDED`] was kept by slots folded before QA-STORE-44 (the slot's `/etc`).
pub const ETC_FOLDED: &str = "etc/wolos/folded-layers";
/// Where [`FOLDED`] was kept before that (inside the slot's `var` mount point, hidden at boot).
pub const LEGACY_FOLDED: &str = "var/lib/wolos/folded";

/// The booted slot's folded list: [`FOLDED`] when the slot has one (`usr`), else the list a
/// slot from before QA-STORE-44 kept in `/etc` (`etc`, read through the overlay).
pub fn folded_list<'a>(usr: Option<&'a str>, etc: Option<&'a str>) -> &'a str {
    usr.or(etc).unwrap_or("")
}
/// The highest layer number ever handed out (decimal text). Layer names are never reused: a
/// cleared layer's name may still be in a slot's [`FOLDED`] list (R2C-1).
pub const COUNTER: &str = "/var/lib/wolos/layer-counter";
/// pacman's own default `DBPath`.
pub const DEFAULT_DB_PATH: &str = "/var/lib/pacman";

/// pacman.conf's `DBPath` without the trailing `/` (an installed slot: `/usr/lib/pacman`);
/// [`DEFAULT_DB_PATH`] when the file has none or names a relative path or `..`.
pub fn db_path(pacman_conf: &str) -> String {
    pacman_conf
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == "DBPath").then(|| v.trim().trim_end_matches('/').to_owned())
        })
        .filter(|p| p.starts_with('/') && p.len() > 1 && !p.split('/').any(|c| c == ".."))
        .unwrap_or_else(|| DEFAULT_DB_PATH.to_owned())
}

/// The installed-package records (`<DBPath>/local`) of a pacman.conf.
pub fn local_db(pacman_conf: &str) -> String {
    format!("{}/local", db_path(pacman_conf))
}

/// `pacman -Q` of the layer while its chroot is still mounted (inside [`begin`]'s namespace): the chroot's `/usr` is the
/// layer's overlay, so a database under `/usr` shows the change before the layer is merged.
pub fn after_query(db: &str) -> Vec<String> {
    [PACMAN, "--dbpath", &format!("{CHROOT}{db}/"), "-Q"]
        .map(String::from)
        .to_vec()
}

/// A layer directory name: [`PREFIX`] and digits only (never a path).
pub fn is_layer_name(n: &str) -> bool {
    n.strip_prefix(PREFIX)
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// Live layers the booted slot already holds: names in its [`FOLDED`] text that are still in
/// `ls` of [`DIR`]. Anything that is not a layer name is ignored; empty when nothing matches.
pub fn to_clear(folded: &str, ls: &str) -> Vec<String> {
    let live: Vec<&str> = ls.lines().map(str::trim).collect();
    folded
        .lines()
        .map(str::trim)
        .filter(|n| is_layer_name(n) && live.contains(n))
        .map(str::to_owned)
        .collect()
}

/// Layers in `ls` of [`DIR`] without their [`RECORD`] (`has_record` false): pacman never finished
/// in them (killed, power lost; [`finish`] writes the record first), so they are removed, never
/// merged (audit8 stability2 #3). Only while no change runs: the one being made has none yet.
pub fn unfinished(ls: &str, has_record: impl Fn(&str) -> bool) -> Vec<String> {
    ls.lines()
        .map(str::trim)
        .filter(|n| is_layer_name(n) && !has_record(n))
        .map(str::to_owned)
        .collect()
}

/// Where [`clear_steps`] moves the layers it clears until the new merge is up (same file system
/// as [`DIR`], so the move is a rename and copies nothing).
pub const CLEARED_DIR: &str = "/var/lib/wolos/layers-cleared";

/// The step that merges [`DIR`] as it is now over `/usr`, in place of the merge that is up.
/// systemd-sysext swaps the two itself; it documents a brief moment between them (systemd 262),
/// far shorter than an unmerge, a removal and a merge one after the other.
/// Through systemd-sysext.service (its reload is `systemd-sysext refresh`), not by running the
/// tool here: `wol layers clear` runs from wolos-boot-ok.service, whose sandbox is its own mount
/// namespace, and a merge made there never reaches the system (the stale merge after a clear,
/// store-update-pkg-r2 R2-10). Not running: started, which merges.
pub fn refresh_step() -> Vec<String> {
    ["systemctl", "reload-or-restart", "systemd-sysext.service"]
        .map(String::from)
        .to_vec()
}

/// Steps that remove the layers [`to_clear`] found while the login window may already be up
/// (audit core-install #5): each moves out of [`DIR`] into [`CLEARED_DIR`] (a rename: nothing in
/// it changes under the live merge), [`refresh_step`] merges what is left, and only then are
/// the moved layers and their work directories removed, so the layers that stay are never
/// unmerged for as long as a removal takes. Empty when there is nothing to remove.
/// A caller that stops before the refresh still runs it; one that stops at or after it runs
/// nothing more (a failed refresh may still be using the moved layers).
pub fn clear_steps(names: &[String]) -> Vec<Vec<String>> {
    let s = |a: &[&str]| a.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    let names: Vec<&String> = names.iter().filter(|n| is_layer_name(n)).collect();
    if names.is_empty() {
        return Vec::new();
    }
    let mut out = vec![s(&["mkdir", "-p", CLEARED_DIR])];
    out.extend(names.iter().map(|n| {
        s(&[
            "mv",
            "-T",
            &format!("{DIR}/{n}"),
            &format!("{CLEARED_DIR}/{n}"),
        ])
    }));
    out.push(refresh_step());
    out.push(s(&["rm", "-rf", CLEARED_DIR]));
    out.extend(
        names
            .iter()
            .map(|n| s(&["rm", "-rf", &format!("{WORK_DIR}/{n}")])),
    );
    out
}

/// Where `wol layers clear` keeps the layers the booted slot folded in, out of the merge, for as
/// long as the other slot (the restore point) does not hold them (QA-LIVE3-1: a rollback lost
/// every program installed since the install). Same file system as [`DIR`]: moves are renames.
pub const KEPT_DIR: &str = "/var/lib/wolos/layers-kept";

/// What one `wol layers clear` does, by layer name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tidy {
    /// Folded into the booted slot: out of the merge, kept.
    pub keep: Vec<String>,
    /// Kept, but the booted slot does not hold them (a rollback): back into the merge.
    pub restore: Vec<String>,
    /// Never finished ([`unfinished`]): removed.
    pub drop: Vec<String>,
    /// Kept, and both slots hold them: removed for good.
    pub prune: Vec<String>,
}

/// [`Tidy`] from the booted slot's folded list, the other slot's (None: unknown, nothing is
/// pruned), `ls` of [`DIR`] and of [`KEPT_DIR`], and the unfinished layers.
pub fn tidy(
    folded: &str,
    other_folded: Option<&str>,
    ls: &str,
    kept: &str,
    unfinished: &[String],
) -> Tidy {
    let names = |t: &str| -> Vec<String> {
        t.lines()
            .map(str::trim)
            .filter(|n| is_layer_name(n))
            .map(str::to_owned)
            .collect()
    };
    let (booted, kept) = (names(folded), names(kept));
    let keep: Vec<String> = to_clear(folded, ls)
        .into_iter()
        .filter(|n| !unfinished.contains(n))
        .collect();
    let restore = kept
        .iter()
        .filter(|n| !booted.contains(n))
        .cloned()
        .collect();
    let other = other_folded.map(names);
    let prune = kept
        .iter()
        .filter(|n| booted.contains(n) && other.as_ref().is_some_and(|o| o.contains(n)))
        .cloned()
        .collect();
    let drop = unfinished
        .iter()
        .filter(|n| is_layer_name(n))
        .cloned()
        .collect();
    Tidy {
        keep,
        restore,
        drop,
        prune,
    }
}

/// [`Tidy`]'s steps, as [`clear_steps`] orders them: renames out of (and back into) [`DIR`]
/// first, then one [`refresh_step`], then the removals. Empty when there is nothing to do.
pub fn tidy_steps(t: &Tidy) -> Vec<Vec<String>> {
    let s = |a: &[&str]| a.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    let mv = |from: &str, to: &str, n: &str| {
        s(&["mv", "-T", &format!("{from}/{n}"), &format!("{to}/{n}")])
    };
    let moves = !(t.keep.is_empty() && t.restore.is_empty() && t.drop.is_empty());
    if !moves && t.prune.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if moves {
        let mut mkdir = vec!["mkdir", "-p"];
        if !t.keep.is_empty() {
            mkdir.push(KEPT_DIR);
        }
        if !t.drop.is_empty() {
            mkdir.push(CLEARED_DIR);
        }
        if mkdir.len() > 2 {
            out.push(s(&mkdir));
        }
        out.extend(t.keep.iter().map(|n| mv(DIR, KEPT_DIR, n)));
        out.extend(t.drop.iter().map(|n| mv(DIR, CLEARED_DIR, n)));
        out.extend(t.restore.iter().map(|n| mv(KEPT_DIR, DIR, n)));
        out.push(refresh_step());
        if !t.drop.is_empty() {
            out.push(s(&["rm", "-rf", CLEARED_DIR]));
        }
        out.extend(
            t.keep
                .iter()
                .chain(&t.drop)
                .map(|n| s(&["rm", "-rf", &format!("{WORK_DIR}/{n}")])),
        );
    }
    out.extend(
        t.prune
            .iter()
            .map(|n| s(&["rm", "-rf", &format!("{KEPT_DIR}/{n}")])),
    );
    out
}

pub fn name(n: u32) -> String {
    format!("{PREFIX}{n:04}")
}

/// Next layer number: one above the highest of `ls` of [`DIR`], the persisted [`COUNTER`] and
/// every name in the booted slot's [`FOLDED`] list, so a name a slot still lists is never handed
/// out again (R2C-1). Later layers sort after earlier ones.
pub fn next_index(ls: &str, counter: &str, folded: &str) -> u32 {
    let num = |l: &str| l.trim().strip_prefix(PREFIX)?.parse::<u32>().ok();
    ls.lines()
        .chain(folded.lines())
        .filter_map(num)
        .chain(counter.trim().parse::<u32>().ok())
        .max()
        .map_or(1, |m| m.saturating_add(1))
}

/// The extension matches any system (`ID=_any`): a layer follows the person to the next slot
/// only by being folded into it, so it never needs to name the release.
pub fn extension_release() -> String {
    "ID=_any\nARCHITECTURE=x86-64\n".into()
}

fn v(a: &[&str]) -> Step {
    Step::Run(a.iter().map(|s| (*s).to_owned()).collect())
}

/// The overlay of the layer over one top directory of the chroot (`usr`, `opt`).
fn overlay(l: &str, w: &str, top: &str) -> String {
    format!("lowerdir={CHROOT}/{top},upperdir={l}/{top},workdir={w}/{top}")
}

/// Where the in-namespace `pacman -Q` of the layer ([`after_query`]) is left for `wol pkg`: the
/// chroot is gone once [`begin`]'s private namespace ends, so the query runs inside it.
pub fn after_path(n: u32) -> String {
    format!("{WORK_DIR}/{}/after", name(n))
}

/// The shell program [`begin`] runs in its private mount namespace: `$1` is the live `DBPath`,
/// the rest pacman's arguments. pacman's own exit status is the step's; the query after it is
/// best effort (pacman has committed by then, CORE-5).
fn chroot_script(n: u32) -> String {
    let l = format!("{DIR}/{}", name(n));
    let w = format!("{WORK_DIR}/{}", name(n));
    format!(
        "set -e\n\
mount --rbind / {CHROOT}\n\
mount --make-rprivate {CHROOT}\n\
mount -t overlay overlay -o {usr} {CHROOT}/usr\n\
mount -t overlay overlay -o {opt} {CHROOT}/opt\n\
db=$1\n\
shift\n\
arch-chroot {CHROOT} {PACMAN} \"$@\" || exit\n\
{PACMAN} --dbpath \"{CHROOT}$db/\" -Q > {after} 2>/dev/null || :\n",
        usr = overlay(&l, &w, "usr"),
        opt = overlay(&l, &w, "opt"),
        after = after_path(n),
    )
}

/// Steps up to and including the real pacman in the layer's chroot. The layer has no
/// extension-release yet: until [`finish`] writes it, systemd-sysext ignores the directory, so a
/// layer cut off mid-pacman (power loss, the 1800 s cap) never merges (QA-STORE-45). `/usr` and
/// `/opt` are overlays (both merge as system extensions): a package that installs under /opt
/// would otherwise meet the sealed, read-only root (QA-L2-32).
/// The chroot (the rbind of `/`, the two overlays, arch-chroot's own mounts) lives only in a
/// mount namespace of its own (`unshare --mount --propagation private`), gone when pacman's
/// step ends, killed or not: an rbind of the shared `/` in PID 1's namespace copied the whole
/// mount tree into every service's namespace, and `umount -R` never reached those copies, so
/// after about nine installs no mount could be made any more (QA-STORE-58, RH-192). `db` is
/// the live `DBPath` ([`db_path`]); the layer's `pacman -Q` lands in [`after_path`].
pub fn begin(n: u32, args: &[String], db: &str) -> Vec<Step> {
    let l = format!("{DIR}/{}", name(n));
    let w = format!("{WORK_DIR}/{}", name(n));
    let mut private = super::private_namespace();
    private.extend(["sh", "-c"].map(String::from));
    private.push(chroot_script(n));
    private.push("wol-layer".into());
    private.push(db.to_owned());
    private.extend(args.iter().cloned());
    vec![
        v(&[
            "mkdir",
            "-p",
            &format!("{l}/usr/lib/extension-release.d"),
            &format!("{l}/opt"),
            &format!("{w}/usr"),
            &format!("{w}/opt"),
            CHROOT,
        ]),
        // A chroot an earlier run left behind (QA-STORE-53, from before QA-STORE-58: in PID 1's
        // namespace) is unmounted before this plan runs, under the update lock (controller pkg.rs,
        // `chroot_mounted`/`unmount_chroot`).
        Step::Run(private),
    ]
}

/// Steps after pacman succeeded (the chroot went with [`begin`]'s namespace): keep the record, make the layer an extension (its
/// extension-release, written last: QA-STORE-45), merge it. pacman has committed by now, so a
/// failure here never removes the layer (its files and database entries are the change): the
/// caller stops, and the next good boot finishes it (`wol layers clear` writes a missing
/// extension-release of a layer that has its record) and merges it.
pub fn finish(n: u32, rec: &Record) -> Vec<Step> {
    let l = format!("{DIR}/{}", name(n));
    vec![
        Step::Write {
            path: format!("{l}/{RECORD}"),
            text: rec.to_json(),
        },
        Step::Write {
            path: release_path(n),
            text: extension_release(),
        },
        v(&["rm", "-rf", &format!("{WORK_DIR}/{}", name(n))]),
        v(&["systemd-sysext", "refresh"]),
    ]
}

/// A layer's extension-release file.
pub fn release_path(n: u32) -> String {
    release_file(&name(n))
}

/// The extension-release file of the layer called `layer` (a name of [`DIR`]).
pub fn release_file(layer: &str) -> String {
    format!("{DIR}/{layer}/usr/lib/extension-release.d/extension-release.{layer}")
}

/// Layers in `ls` of [`DIR`] that committed (they have their [`RECORD`]) but whose
/// extension-release is missing: [`finish`] writes the record first and the release after, so a
/// cut at that point left a layer systemd-sysext refuses. `wol layers clear` writes the release
/// (QA-STORE-45; it was boot-ok's shell loop, which shared its guard with [`unfinished`]).
pub fn unreleased(
    ls: &str,
    has_record: impl Fn(&str) -> bool,
    has_release: impl Fn(&str) -> bool,
) -> Vec<String> {
    ls.lines()
        .map(str::trim)
        .filter(|n| is_layer_name(n) && has_record(n) && !has_release(n))
        .map(str::to_owned)
        .collect()
}

/// Whether `/proc/self/mountinfo` text shows something mounted at or under [`CHROOT`]: a run
/// from before QA-STORE-58 (its chroot was in PID 1's namespace) that stopped before its `umount -R` left its rbind and overlay there (audit8 stability2
/// #8), and the next [`begin`] must not stack another rbind on them.
pub fn chroot_mounted(mountinfo: &str) -> bool {
    mountinfo.lines().any(|l| {
        l.split_whitespace().nth(4).is_some_and(|m| {
            m == CHROOT || m.strip_prefix(CHROOT).is_some_and(|r| r.starts_with('/'))
        })
    })
}

/// Unmounts what [`chroot_mounted`] found.
pub fn unmount_chroot() -> Vec<String> {
    ["umount", "-R", CHROOT].map(String::from).to_vec()
}

/// After a failed [`begin`] step (pacman did not commit): nothing of the layer is left, the
/// running system is untouched.
pub fn abort(n: u32) -> Vec<Vec<String>> {
    let s = |a: &[&str]| a.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    vec![
        s(&["rm", "-rf", &format!("{DIR}/{}", name(n))]),
        s(&["rm", "-rf", &format!("{WORK_DIR}/{}", name(n))]),
    ]
}

/// Packages a layer added and removed, as `name-version` (the directory names in the pacman database).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Record {
    pub installed: Vec<String>,
    pub removed: Vec<String>,
}

impl Record {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default() + "\n"
    }
    pub fn parse(text: &str) -> Record {
        serde_json::from_str(text).unwrap_or_default()
    }
}

/// `pacman -Q` before and after.
pub fn delta(before: &str, after: &str) -> Record {
    let set = |t: &str| -> std::collections::BTreeSet<String> {
        t.lines()
            .filter_map(|l| {
                let mut w = l.split_whitespace();
                Some(format!("{}-{}", w.next()?, w.next()?))
            })
            .collect()
    };
    let (b, a) = (set(before), set(after));
    Record {
        installed: a.difference(&b).cloned().collect(),
        removed: b.difference(&a).cloned().collect(),
    }
}

/// Whether the two `pacman -Q` answers around a change can be compared at all (QA-A7-4). The
/// query in the layer's namespace is best effort; when it fails it leaves an empty (or cut)
/// file, which [`delta`] would read as "every package was removed" and the record would then
/// strip every package's entry from the next slot. Empty, or fewer than half of what was there
/// before, is a failed query: no change to a layer removes half of the system.
pub fn readable(before: &str, after: Option<&str>) -> bool {
    let count = |t: &str| {
        t.lines()
            .filter(|l| l.split_whitespace().count() >= 2)
            .count()
    };
    let (b, a) = (count(before), after.map_or(0, count));
    b > 0 && a > 0 && a * 2 >= b
}

/// Package name of a database directory `name-version-release`.
pub fn pkg_name(dir: &str) -> &str {
    let mut it = dir.rsplitn(3, '-');
    it.next();
    it.next();
    it.next().unwrap_or(dir)
}

/// Kernel, initramfs and init packages: nothing can swap them in a running system.
pub fn needs_restart(name: &str) -> bool {
    !name.contains("headers")
        && (name.starts_with("linux")
            || name.starts_with("systemd")
            || name.starts_with("mkinitcpio")
            || name.ends_with("-ucode")
            || name == "dracut")
}

/// An overlay whiteout: a character device 0:0.
pub fn is_whiteout(is_char_device: bool, rdev: u64) -> bool {
    is_char_device && rdev == 0
}

/// Layer directories in the order they apply (oldest first).
pub fn ordered(mut names: Vec<String>) -> Vec<String> {
    names.retain(|n| n.starts_with(PREFIX));
    names.sort();
    names
}

/// Where a live layer stands (`wol layers list`, QA-PWR-7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Installed on this system only; the next system update copies it into the new slot.
    LiveOnly,
    /// The booted slot already holds it (its folded list names it); the next restart removes it.
    InThisSystem,
    /// pacman never finished in it (no record); it is removed, never merged.
    Unfinished,
}

impl State {
    /// What the listing says happens to a layer in this state.
    pub fn words(self) -> &'static str {
        match self {
            State::LiveOnly => "live only, folded into the system at the next update",
            State::InThisSystem => "already part of this system, removed at the next restart",
            State::Unfinished => "unfinished, removed at the next restart",
        }
    }
    /// The state's word in `--json`.
    pub fn id(self) -> &'static str {
        match self {
            State::LiveOnly => "live-only",
            State::InThisSystem => "in-system",
            State::Unfinished => "unfinished",
        }
    }
}

/// One live layer as `wol layers list` shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    /// Bytes on disk (0 when unknown).
    pub bytes: u64,
    /// `YYYY-MM-DD` the layer was finished (its record's date); empty when unknown.
    pub date: String,
    /// Packages added and removed, `name-version-release`.
    pub installed: Vec<String>,
    pub removed: Vec<String>,
    pub state: State,
}

/// `du` of the layer directories: bytes, one line per path.
pub fn size_argv(names: &[String]) -> Vec<String> {
    let mut v = vec!["du".to_owned(), "-sb".into(), "--".into()];
    v.extend(
        names
            .iter()
            .filter(|n| is_layer_name(n))
            .map(|n| format!("{DIR}/{n}")),
    );
    v
}

/// `find` that prints the day each layer's record was written: `YYYY-MM-DD <path>`.
pub fn date_argv() -> Vec<String> {
    [
        "find",
        DIR,
        "-mindepth",
        "2",
        "-maxdepth",
        "2",
        "-name",
        RECORD,
        "-printf",
        "%TY-%Tm-%Td %p\\n",
    ]
    .map(String::from)
    .to_vec()
}

/// The rows of `wol layers list`, oldest layer first. `ls` is `ls -1` of [`DIR`], `du` the output
/// of [`size_argv`], `dates` that of [`date_argv`] (None = could not be read: unknown, not 0), `record(name)` the text of a layer's
/// [`RECORD`] (None = not there), `folded` the booted slot's folded list ([`folded_list`]).
pub fn list(
    ls: &str,
    du: Option<&str>,
    dates: Option<&str>,
    record: impl Fn(&str) -> Option<String>,
    folded: &str,
) -> Vec<Row> {
    let names = ordered(
        ls.lines()
            .map(str::trim)
            .filter(|n| is_layer_name(n))
            .map(str::to_owned)
            .collect(),
    );
    let by_path = |text: Option<&str>, name: &str| -> Option<String> {
        text?.lines().find_map(|l| {
            let (head, path) = l.split_once(['\t', ' '])?;
            let path = path.trim();
            let dir = path.strip_prefix(DIR)?.trim_start_matches('/');
            (dir.split('/').next() == Some(name)).then(|| head.to_owned())
        })
    };
    names
        .iter()
        .map(|n| {
            let text = record(n);
            let rec = text.as_deref().map(Record::parse).unwrap_or_default();
            let state = if text.is_none() {
                State::Unfinished
            } else if folded.lines().any(|f| f.trim() == n) {
                State::InThisSystem
            } else {
                State::LiveOnly
            };
            Row {
                name: n.clone(),
                bytes: by_path(du, n).and_then(|b| b.parse().ok()).unwrap_or(0),
                date: by_path(dates, n).unwrap_or_default(),
                installed: rec.installed,
                removed: rec.removed,
                state,
            }
        })
        .collect()
}

/// What `wol layers list` prints for `rows` (a sentence when there are none).
pub fn table(rows: &[Row]) -> String {
    if rows.is_empty() {
        return "No live layers: every package on this system is part of the system itself.\n"
            .into();
    }
    let mut out = String::new();
    for r in rows {
        let size = if r.bytes == 0 {
            String::new()
        } else {
            crate::storage::human(r.bytes)
        };
        out.push_str(&format!(
            "{}  {}  {}  {}\n",
            r.name,
            if r.date.is_empty() { "-" } else { &r.date },
            if size.is_empty() { "-" } else { &size },
            r.state.words()
        ));
        if !r.installed.is_empty() {
            out.push_str(&format!("  added:   {}\n", r.installed.join(" ")));
        }
        if !r.removed.is_empty() {
            out.push_str(&format!("  removed: {}\n", r.removed.join(" ")));
        }
    }
    out
}

/// Database directories to copy into a slot and package names to drop from it.
pub fn db_changes(recs: &[Record]) -> (Vec<String>, Vec<String>) {
    let mut add: Vec<String> = Vec::new();
    let mut drop: Vec<String> = Vec::new();
    for r in recs {
        for d in &r.removed {
            add.retain(|a| pkg_name(a) != pkg_name(d));
            drop.push(pkg_name(d).to_owned());
        }
        for d in &r.installed {
            drop.retain(|x| x != pkg_name(d));
            add.retain(|a| pkg_name(a) != pkg_name(d));
            add.push(d.clone());
        }
    }
    (add, drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// QA-PWR-7: which packages are live-only, how big, since when.
    #[test]
    fn layers_list_names_packages_size_date_and_what_happens_next() {
        let rec = |n: &str| match n {
            "wolos-layer-0001" => Some(
                Record {
                    installed: vec!["htop-3.4.1-1".into(), "base-devel-1-2".into()],
                    removed: vec!["nano-8.2-1".into()],
                }
                .to_json(),
            ),
            "wolos-layer-0002" => Some(Record::default().to_json()),
            _ => None,
        };
        let du = "1234567\t/var/lib/extensions/wolos-layer-0001\n99\t/var/lib/extensions/wolos-layer-0002\n5\t/var/lib/extensions/wolos-layer-0010\n";
        let dates = "2026-11-03 /var/lib/extensions/wolos-layer-0001/wolos-layer.json\n2026-11-02 /var/lib/extensions/wolos-layer-0002/wolos-layer.json\n";
        let rows = list(
            "wolos-layer-0010\nwolos-layer-0002\nwolos-layer-0001\nstray\n",
            Some(du),
            Some(dates),
            rec,
            "wolos-layer-0002\n",
        );
        // A size or a date that could not be read is unknown, not 0.
        let blind = list("wolos-layer-0001\n", None, None, |_| Some("{}".into()), "");
        assert_eq!((blind[0].bytes, blind[0].date.as_str()), (0, ""));
        assert!(
            table(&blind).contains("wolos-layer-0001  -  -  live only"),
            "{}",
            table(&blind)
        );
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            ["wolos-layer-0001", "wolos-layer-0002", "wolos-layer-0010"]
        );
        assert_eq!(rows[0].bytes, 1_234_567);
        assert_eq!(rows[0].date, "2026-11-03");
        assert_eq!(rows[0].installed, ["htop-3.4.1-1", "base-devel-1-2"]);
        assert_eq!(rows[0].state, State::LiveOnly);
        assert_eq!(rows[1].state, State::InThisSystem);
        assert_eq!(rows[2].state, State::Unfinished);
        let t = table(&rows);
        assert!(t.contains("wolos-layer-0001  2026-11-03  1.2 MB  live only, folded into the system at the next update"), "{t}");
        assert!(
            t.contains("  added:   htop-3.4.1-1 base-devel-1-2\n"),
            "{t}"
        );
        assert!(t.contains("  removed: nano-8.2-1\n"), "{t}");
        assert!(table(&[]).starts_with("No live layers"));
        // The du/find lines: layer names only, never a path from the listing.
        assert_eq!(
            size_argv(&["wolos-layer-0001".into(), "../etc".into()]),
            ["du", "-sb", "--", "/var/lib/extensions/wolos-layer-0001"]
        );
    }

    #[test]
    fn layers_number_after_the_last() {
        assert_eq!(next_index("", "", ""), 1);
        assert_eq!(
            next_index("wolos-layer-0001\nwolos-layer-0007\nother\n", "", ""),
            8
        );
        // R2C-1: after a clear removed 0001-0003, neither the counter nor the folded list
        // lets 0001 come back.
        assert_eq!(next_index("", "3\n", ""), 4);
        assert_eq!(next_index("", "", "wolos-layer-0003\n"), 4);
        assert_eq!(next_index("wolos-layer-0002\n", "9", "junk\n"), 10);
        assert_eq!(next_index("", "x", ""), 1);
        assert_eq!(name(12), "wolos-layer-0012");
    }

    #[test]
    fn pacman_runs_in_an_overlay_chroot_and_the_layer_is_merged_at_once() {
        let args: Vec<String> = ["-S", "vim"].map(String::from).to_vec();
        let t: Vec<String> = begin(3, &args, DEFAULT_DB_PATH)
            .iter()
            .map(|s| s.to_string())
            .collect();
        let l = "/var/lib/extensions/wolos-layer-0003";
        assert!(t[0].starts_with(&format!("mkdir -p {l}/usr/lib/extension-release.d")));
        // audit8 stability2 #3: nothing makes the layer an extension before pacman is done.
        assert!(
            !t.iter().any(|x| x.contains("extension-release.wolos")),
            "{t:?}"
        );
        assert!(
            t[1].contains("\nmount --rbind / /run/wolos/layer-root\n"),
            "{t:?}"
        );
        assert!(t[1].contains(&format!(
            "lowerdir=/run/wolos/layer-root/usr,upperdir={l}/usr,workdir=/var/lib/wolos/layers/wolos-layer-0003/usr"
        )));
        assert!(
            t[1].contains("\narch-chroot /run/wolos/layer-root /usr/bin/pacman \"$@\" || exit\n"),
            "{t:?}"
        );
        assert!(t[1].ends_with(" wol-layer /var/lib/pacman -S vim"), "{t:?}");
        assert_eq!(t.len(), 2, "{t:?}");
        let rec = Record {
            installed: vec!["vim-9.1-1".into()],
            removed: vec![],
        };
        let f: Vec<String> = finish(3, &rec).iter().map(|s| s.to_string()).collect();
        assert_eq!(f[0], format!("write {l}/wolos-layer.json"));
        assert!(!f.iter().any(|x| x.contains("umount")), "{f:?}");
        assert_eq!(f.last().unwrap(), "systemd-sysext refresh");
        assert!(
            f.iter()
                .any(|s| s == &format!("write {l}/wolos-layer.json"))
        );
    }

    /// QA-STORE-45: a layer becomes a system extension only once pacman committed and its
    /// record is written: begin writes no extension-release, finish writes it after the record.
    /// A power cut or the 1800 s cap mid-pacman leaves a directory sysext ignores.
    #[test]
    fn extension_release_written_last() {
        let args: Vec<String> = ["-S", "vim"].map(String::from).to_vec();
        let b: Vec<String> = begin(3, &args, DEFAULT_DB_PATH)
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(
            !b.iter()
                .any(|s| s.contains("extension-release.wolos-layer")),
            "{b:?}"
        );
        let f: Vec<String> = finish(3, &Record::default())
            .iter()
            .map(|s| s.to_string())
            .collect();
        let rec = f
            .iter()
            .position(|s| s.ends_with("/wolos-layer.json"))
            .unwrap();
        let rel = f
            .iter()
            .position(|s| {
                s == "write /var/lib/extensions/wolos-layer-0003/usr/lib/extension-release.d/\
extension-release.wolos-layer-0003"
            })
            .unwrap();
        assert!(rec < rel, "{f:?}");
        let refresh = f
            .iter()
            .position(|s| s == "systemd-sysext refresh")
            .unwrap();
        assert!(rel < refresh, "{f:?}");
    }

    /// QA-STORE-53: a chroot an earlier run (from before QA-STORE-58) left mounted is unmounted
    /// before `begin`, under the lock (controller pkg.rs; tests/pkg.rs), never stacked; finish and
    /// abort have nothing left to unmount (the chroot went with its namespace), and no step of
    /// the plan is a bare shell string (RR-020).
    #[test]
    fn umount_lazy_fallback() {
        let f: Vec<String> = finish(3, &Record::default())
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(!f.iter().any(|s| s.contains("umount")), "{f:?}");
        let a: Vec<String> = abort(3).iter().map(|v| v.join(" ")).collect();
        assert!(!a.iter().any(|s| s.contains("umount")), "{a:?}");
        let args: Vec<String> = ["-S", "vim"].map(String::from).to_vec();
        let b: Vec<String> = begin(3, &args, DEFAULT_DB_PATH)
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(!b.iter().any(|s| s.starts_with("sh ")), "{b:?}");
        assert!(
            b[1].starts_with("unshare --mount --propagation private --fork --kill-child=SIGKILL ")
                && b[1].contains(&format!("\nmount --rbind / {CHROOT}\n")),
            "{b:?}"
        );
    }

    /// QA-STORE-58, RH-192: no mount of the layer chroot is made in the caller's (PID 1's)
    /// namespace: every mount and arch-chroot run in one `unshare --mount --propagation
    /// private` step, so nothing reaches the services' namespaces and nothing outlives pacman.
    /// The layer's `pacman -Q` runs in there too, before the chroot goes.
    #[test]
    fn the_layer_chroot_lives_in_a_private_namespace() {
        let args: Vec<String> = ["-S", "vim"].map(String::from).to_vec();
        let steps = begin(3, &args, "/usr/lib/pacman");
        let argvs: Vec<&Vec<String>> = steps
            .iter()
            .filter_map(|s| match s {
                Step::Run(a) => Some(a),
                Step::Write { .. } => None,
            })
            .collect();
        for a in &argvs {
            assert!(
                !matches!(a[0].as_str(), "mount" | "arch-chroot"),
                "{a:?} runs in PID 1's namespace"
            );
        }
        let ns = argvs
            .iter()
            .find(|a| a.iter().any(|x| x.contains("--rbind")))
            .expect("a chroot step");
        assert_eq!(
            ns[..6],
            [
                "unshare",
                "--mount",
                "--propagation",
                "private",
                "--fork",
                "--kill-child=SIGKILL"
            ]
        );
        let script = &ns[8];
        let bind = script.find("mount --rbind /").unwrap();
        let chroot = script.find("arch-chroot").unwrap();
        let query = script
            .find("-Q > /var/lib/wolos/layers/wolos-layer-0003/after")
            .unwrap();
        assert!(bind < chroot && chroot < query, "{script}");
        assert!(script.starts_with("set -e\n"), "{script}");
        let parsed = std::process::Command::new("sh")
            .args(["-n", "-c", script])
            .status()
            .unwrap();
        assert!(parsed.success(), "{script}");
        // The live DBPath and pacman's arguments are arguments, never spliced into the program.
        assert_eq!(ns[9..], ["wol-layer", "/usr/lib/pacman", "-S", "vim"]);
        assert_eq!(
            after_path(3),
            "/var/lib/wolos/layers/wolos-layer-0003/after"
        );
    }

    /// QA-L2-32: pacman sees /opt writable in the layer chroot too (ROCm and other large
    /// packages install there; systemd-sysext merges /opt as well as /usr).
    #[test]
    fn opt_is_layered_too() {
        let args: Vec<String> = ["-S", "blender"].map(String::from).to_vec();
        let b: Vec<String> = begin(4, &args, DEFAULT_DB_PATH)
            .iter()
            .map(|s| s.to_string())
            .collect();
        let l = "/var/lib/extensions/wolos-layer-0004";
        assert!(
            b.iter().any(|s| s.contains(&format!("upperdir={l}/opt"))
                && s.contains(&format!("{CHROOT}/opt\n"))),
            "{b:?}"
        );
    }

    /// QA-STORE-44: the folded list lives in the slot's sealed /usr (never in the shared /etc
    /// overlay, whose upper copy would shadow every later slot's list); a slot sealed before
    /// keeps it in /etc and is read there when /usr has none.
    #[test]
    fn folded_path_usr_lib_with_legacy() {
        assert_eq!(FOLDED, "usr/lib/wolos/folded-layers");
        assert!(!FOLDED.starts_with("etc/"));
        let new = "wolos-layer-0004\n";
        let old = "wolos-layer-0002\n";
        assert_eq!(
            folded_list(Some(new), Some(old)),
            new,
            "the slot's own list wins"
        );
        assert_eq!(
            folded_list(None, Some(old)),
            old,
            "a slot from before: /etc"
        );
        assert_eq!(folded_list(None, None), "");
        // An empty list in the /etc upper (what clear wrote before) hides nothing any more.
        assert_eq!(folded_list(Some(new), Some("")), new);
    }

    #[test]
    fn a_chroot_left_mounted_is_seen() {
        let line =
            |m: &str| format!("36 25 0:32 / {m} rw,relatime shared:1 - btrfs /dev/vda2 rw\n");
        let left = line("/") + &line("/run/wolos/layer-root") + &line("/run/wolos/layer-root/usr");
        assert!(chroot_mounted(&left));
        assert!(chroot_mounted(&line("/run/wolos/layer-root/usr")));
        assert!(!chroot_mounted(
            &(line("/") + &line("/run/wolos/layer-rootx"))
        ));
        assert!(!chroot_mounted(""));
        assert_eq!(
            unmount_chroot().join(" "),
            "umount -R /run/wolos/layer-root"
        );
    }

    #[test]
    fn a_committed_layer_without_its_release_is_unreleased() {
        let ls = "wolos-layer-0001\nwolos-layer-0002\nwolos-layer-0003\njunk\n";
        // 0001 whole, 0002 cut after its record, 0003 never committed.
        let got = unreleased(ls, |n| n != "wolos-layer-0003", |n| n == "wolos-layer-0001");
        assert_eq!(got, ["wolos-layer-0002"]);
        assert_eq!(
            release_file("wolos-layer-0002"),
            "/var/lib/extensions/wolos-layer-0002/usr/lib/extension-release.d/extension-release.wolos-layer-0002"
        );
        assert_eq!(release_path(2), release_file("wolos-layer-0002"));
    }

    #[test]
    fn a_layer_without_its_record_is_unfinished() {
        let ls = "wolos-layer-0001\nwolos-layer-0002\nother\n";
        assert_eq!(
            unfinished(ls, |n| n == "wolos-layer-0001"),
            ["wolos-layer-0002"]
        );
        assert!(unfinished(ls, |_| true).is_empty());
        assert!(unfinished("", |_| false).is_empty());
    }

    #[test]
    fn a_failed_install_leaves_nothing_behind() {
        let a: Vec<String> = abort(2).iter().map(|v| v.join(" ")).collect();
        // Nothing to unmount: the chroot went with its namespace (QA-STORE-58).
        assert_eq!(a.len(), 2, "{a:?}");
        assert!(a[0].starts_with("rm -rf /var/lib/extensions/wolos-layer-0002"));
        assert!(a[1].starts_with("rm -rf /var/lib/wolos/layers/wolos-layer-0002"));
    }

    #[test]
    fn what_a_layer_added_and_removed() {
        let r = delta(
            "bash 5.2-1\nvim 9.0-1\nold 1-1\n",
            "bash 5.2-1\nvim 9.1-1\ngit 2.4-1\n",
        );
        assert_eq!(r.installed, ["git-2.4-1", "vim-9.1-1"]);
        assert_eq!(r.removed, ["old-1-1", "vim-9.0-1"]);
        assert_eq!(Record::parse(&r.to_json()), r);
        assert_eq!(Record::parse("junk"), Record::default());
        assert_eq!(pkg_name("linux-api-headers-6.1-1"), "linux-api-headers");
    }

    #[test]
    fn only_kernel_and_init_packages_wait_for_a_restart() {
        for n in [
            "linux-wolos",
            "linux-firmware",
            "systemd",
            "systemd-libs",
            "mkinitcpio",
            "intel-ucode",
        ] {
            assert!(needs_restart(n), "{n}");
        }
        for n in [
            "vim",
            "firefox",
            "linux-api-headers",
            "libsystemd-fake",
            "util-linux",
        ] {
            assert!(!needs_restart(n), "{n}");
        }
    }

    #[test]
    fn whiteouts_and_order() {
        assert!(is_whiteout(true, 0));
        assert!(!is_whiteout(true, 5));
        assert!(!is_whiteout(false, 0));
        let o = ordered(vec![
            "wolos-layer-0002".into(),
            "x".into(),
            "wolos-layer-0001".into(),
        ]);
        assert_eq!(o, ["wolos-layer-0001", "wolos-layer-0002"]);
    }

    #[test]
    fn the_package_database_is_where_pacman_conf_says() {
        let slot = "[options]\nHoldPkg = pacman\nDBPath = /usr/lib/pacman/\n[core]\n";
        assert_eq!(db_path(slot), "/usr/lib/pacman");
        assert_eq!(local_db(slot), "/usr/lib/pacman/local");
        assert_eq!(
            db_path("[options]\n#DBPath = /usr/lib/pacman/\n"),
            DEFAULT_DB_PATH
        );
        assert_eq!(db_path(""), "/var/lib/pacman");
        assert_eq!(db_path("DBPath = relative/\n"), DEFAULT_DB_PATH);
        assert_eq!(db_path("DBPath = /usr/../etc\n"), DEFAULT_DB_PATH);
        assert_eq!(
            after_query("/usr/lib/pacman").join(" "),
            "/usr/bin/pacman --dbpath /run/wolos/layer-root/usr/lib/pacman/ -Q"
        );
    }

    #[test]
    fn the_booted_slot_clears_only_the_layers_it_holds() {
        let folded = "wolos-layer-0001\nwolos-layer-0002\n../../etc\nwolos-layer-x\n";
        let ls = "wolos-layer-0002\nwolos-layer-0003\n";
        assert_eq!(to_clear(folded, ls), ["wolos-layer-0002"]);
        let s: Vec<String> = clear_steps(&to_clear(folded, ls))
            .iter()
            .map(|a| a.join(" "))
            .collect();
        // audit core-install #5: no unmerge; the layer moves aside, the new merge replaces the
        // old one, and only then is anything deleted.
        assert_eq!(
            s,
            [
                "mkdir -p /var/lib/wolos/layers-cleared",
                "mv -T /var/lib/extensions/wolos-layer-0002 /var/lib/wolos/layers-cleared/wolos-layer-0002",
                "systemctl reload-or-restart systemd-sysext.service",
                "rm -rf /var/lib/wolos/layers-cleared",
                "rm -rf /var/lib/wolos/layers/wolos-layer-0002",
            ]
        );
        assert!(!s.iter().any(|x| x.contains("unmerge")));
        assert!(clear_steps(&[]).is_empty());
        assert!(clear_steps(&["../x".into()]).is_empty());
        assert!(to_clear("", ls).is_empty());
    }

    #[test]
    fn database_changes_follow_the_layers_in_order() {
        let r = |i: &[&str], d: &[&str]| Record {
            installed: i.iter().map(|s| s.to_string()).collect(),
            removed: d.iter().map(|s| s.to_string()).collect(),
        };
        let (add, drop) = db_changes(&[
            r(&["vim-9.0-1", "git-2-1"], &[]),
            r(&["vim-9.1-1"], &["vim-9.0-1"]),
            r(&[], &["git-2-1"]),
        ]);
        assert_eq!(add, ["vim-9.1-1"]);
        assert_eq!(drop, ["git"]);
    }
}
