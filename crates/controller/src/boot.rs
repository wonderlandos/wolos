// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol boot ...`: the installer's questions about the firmware, and the boot-time rollback check.
//! Decisions live in `wol_core::boot`; this reads firmware variables and runs the tools.
use crate::cli::Invocation;
use crate::run::{Env, fail};
use shared::errors::ErrorCode;
use std::path::{Path, PathBuf};
use wol_core::boot::{self, Firmware, Status, Tier};
use wol_core::{seal, update};

fn state(name: &str) -> PathBuf {
    Path::new(boot::STATE_DIR).join(name)
}

fn efi_var(dir: &Path, name: &str, guid: &str) -> Option<Vec<u8>> {
    std::fs::read(dir.join(format!("{name}-{guid}"))).ok()
}

fn out_of_tree(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.flatten().any(|d| {
            let r = |f: &str| std::fs::read_to_string(d.path().join(f)).unwrap_or_default();
            boot::is_nvidia_display(&r("vendor"), &r("class"))
        })
    })
}

fn firmware(env: &dyn Env) -> Firmware {
    let vars = PathBuf::from(
        env.var("WOLOS_EFIVARS")
            .unwrap_or_else(|| "/sys/firmware/efi/efivars".into()),
    );
    let pci = env
        .var("WOLOS_PCI_DIR")
        .unwrap_or_else(|| "/sys/bus/pci/devices".into());
    let shim = env
        .var("WOLOS_SHIM_DIR")
        .unwrap_or_else(|| "/usr/share/wolos/shim".into());
    let flag = |n: &str| efi_var(&vars, n, boot::EFI_GLOBAL).is_some_and(|v| boot::efi_bool(&v));
    Firmware {
        uefi: vars.is_dir(),
        secure_boot: flag("SecureBoot"),
        setup_mode: flag("SetupMode"),
        third_party_ca: efi_var(&vars, "db", boot::EFI_DB)
            .is_some_and(|d| boot::has_third_party_ca(&d)),
        out_of_tree: out_of_tree(Path::new(&pci)),
        first_stage: Path::new(&shim).join("shimx64.efi").is_file(),
        // install-run says so (WOLOS_WINDOWS=yes): Windows stays on this computer's disks.
        windows: env.var("WOLOS_WINDOWS").as_deref() == Some("yes"),
    }
}

fn tier_arg(env: &mut dyn Env, inv: &Invocation) -> Result<Tier, i32> {
    inv.get("tier").and_then(Tier::parse).ok_or_else(|| {
        fail(
            env,
            ErrorCode::Usage,
            "tier: own-keys, mok, off or firmware",
        )
    })
}

/// `wol boot password`: the word for the firmware's key screen.
pub fn password(env: &mut dyn Env) -> i32 {
    env.out(&format!("{}\n", boot::MOK_PASSWORD));
    0
}

/// `wol boot enroll`: queue the key again; the steps follow.
pub fn enroll(env: &mut dyn Env) -> i32 {
    match env
        .capture(&["/usr/lib/wolos/boot-enroll"])
        .as_deref()
        .map(str::trim)
    {
        Some("enrolled") => env.out("This computer's key is already enrolled.\n"),
        Some(_) => {
            env.out("Restart to finish.\n");
            for s in boot::key_screen_steps() {
                env.out(&format!("{s}\n"));
            }
        }
        None => {
            let why = enroll_refusal(&firmware(env), env.is_root());
            return fail(env, ErrorCode::Failed, &why);
        }
    }
    0
}

/// Why `boot-enroll` queued nothing (QA-LIVE3-11: it blamed the account for a computer with
/// Secure Boot off, as root): the first true one of the firmware, then the account.
pub fn enroll_refusal(fw: &boot::Firmware, root: bool) -> String {
    if !fw.uefi {
        "This computer did not start in UEFI mode, so there is no key screen to enroll a key for."
            .into()
    } else if !fw.secure_boot {
        "Secure Boot is off on this computer, so there is nothing to enroll. Turn it on in the \
         firmware's settings first; then run this again."
            .into()
    } else if !root {
        "the key could not be queued; run this as root".into()
    } else {
        "the firmware did not take the key; run `wol boot status` to see why".into()
    }
}

/// `wol boot detect [yes|no]`: prints the tier.
pub fn detect(env: &mut dyn Env, inv: &Invocation) -> i32 {
    let tier = boot::choose(&firmware(env), inv.get("want") != Some("no"));
    env.out(&format!("{}\n", tier.as_str()));
    0
}

/// `wol boot key-screen TIER`: prints yes or no.
pub fn key_screen(env: &mut dyn Env, inv: &Invocation) -> i32 {
    match tier_arg(env, inv) {
        Ok(t) => {
            let yes = boot::needs_key_screen(t, &firmware(env));
            env.out(if yes { "yes\n" } else { "no\n" });
            0
        }
        Err(c) => c,
    }
}

/// `wol boot words TIER`: kernel command line words.
pub fn words(env: &mut dyn Env, inv: &Invocation) -> i32 {
    match tier_arg(env, inv) {
        Ok(t) => {
            env.out(&format!("{}\n", boot::words(t, &firmware(env))));
            0
        }
        Err(c) => c,
    }
}

/// `wol boot note TIER [yes|no]`: the note, the BitLocker line when a Windows with BitLocker
/// stays and the tier changes its keys (`WOLOS_BITLOCKER=yes` from install-run), then the key
/// screen's steps when it will appear.
pub fn note(env: &mut dyn Env, inv: &Invocation) -> i32 {
    match tier_arg(env, inv) {
        Ok(t) => {
            let screen = inv.get("screen") == Some("yes");
            let note = if t == Tier::Firmware && screen {
                boot::FIRMWARE_KEY_SCREEN_NOTE
            } else {
                boot::note(t)
            };
            env.out(&format!("{note}\n"));
            let bitlocker = env.var("WOLOS_BITLOCKER").as_deref() == Some("yes");
            if let Some(b) = boot::bitlocker_note(t, bitlocker) {
                env.out(&format!("{b}\n"));
            }
            if screen {
                for s in boot::key_screen_steps() {
                    env.out(&format!("{s}\n"));
                }
            }
            0
        }
        Err(c) => c,
    }
}

fn number(text: Option<String>) -> u64 {
    text.and_then(|t| t.trim().parse().ok()).unwrap_or(0)
}

/// `wol boot status`: JSON for Settings.
pub fn status(env: &mut dyn Env) -> i32 {
    let fw = firmware(env);
    let cmdline = env
        .read_file(Path::new("/proc/cmdline"))
        .unwrap_or_default();
    let tpm_dir = env
        .var("WOLOS_TPM_DIR")
        .unwrap_or_else(|| "/sys/class/tpm/tpm0".into());
    let s = Status {
        secure_boot: fw.secure_boot,
        setup_mode: fw.setup_mode,
        tier: env
            .read_file(&state("tier"))
            .map_or_else(|| "off".to_owned(), |t| t.trim().to_owned()),
        lockdown: boot::parse_lockdown(
            &env.read_file(Path::new("/sys/kernel/security/lockdown"))
                .unwrap_or_default(),
        ),
        tpm2: wol_core::encrypt::has_tpm2(
            env.read_file(&Path::new(&tpm_dir).join("tpm_version_major"))
                .as_deref(),
        ),
        // Binary, so it is looked at and not read.
        measured: env
            .capture(&["ls", "/sys/kernel/security/tpm0/binary_bios_measurements"])
            .is_some(),
        sealed: boot::cmdline_word(&cmdline, "wolos.seal").is_some(),
        epoch: boot::cmdline_word(&cmdline, "wolos.epoch").and_then(|e| e.parse().ok()),
        floor: number(env.read_file(&state("floor"))),
    };
    env.out(&serde_json::to_string(&s).unwrap_or_default());
    env.out("\n");
    0
}

/// `wol boot check` (early boot): an older signed kernel, restored by hand, does not stay up.
pub fn check(env: &mut dyn Env) -> i32 {
    let cmdline = env
        .read_file(Path::new("/proc/cmdline"))
        .unwrap_or_default();
    let Some(slot) = update::running_slot(&cmdline) else {
        return 0;
    };
    let epoch = boot::cmdline_word(&cmdline, "wolos.epoch").and_then(|e| e.parse().ok());
    let mark = crate::seal::failed_mark(crate::seal::EPOCH_MARK, slot);
    if boot::epoch_allowed(epoch, number(env.read_file(&state("floor")))) {
        // Updated since it was refused: the other slot may fall back to it again.
        let _ = env.remove_file(&mark);
        crate::seal::clear_notice_if_healthy(env);
        return 0;
    }
    // The same rule as `wol seal check`: only to a slot that has an entry and failed neither
    // check, so two slots under the floor stop here instead of restarting into each other.
    let other = update::other_slot(slot).unwrap_or("");
    let fb = seal::fallback(slot, crate::seal::fallback_entry(env, other).as_deref());
    let _ = env.write_file(&mark, "older than the floor\n");
    let msg = if matches!(fb, seal::Fallback::Boot { .. }) {
        format!(
            "This copy of WolOS is older than the version last used here, so it will not stay running. \
             Slot {slot} was refused.\n"
        )
    } else {
        format!(
            "This copy of WolOS (slot {slot}) is older than the version last used here, and there \
             is no newer one to start; please reinstall or restore.\n"
        )
    };
    let _ = env.write_file(Path::new(seal::NOTICE), &msg);
    env.err(&msg);
    if let seal::Fallback::Boot { argv } = fb {
        for a in argv {
            let _ = env.exec(&a);
        }
    }
    1
}

/// `wol boot bless` (the boot was good): the floor rises to the older kept slot, and the other
/// slot's menu entry says it is the previous version.
pub fn bless(env: &mut dyn Env) -> i32 {
    const PREVIOUS: &str = " (previous version)";
    let cmdline = env
        .read_file(Path::new("/proc/cmdline"))
        .unwrap_or_default();
    let Some(running) = update::running_slot(&cmdline) else {
        return 0;
    };
    let dir = format!("{}/loader/entries", update::ESP);
    let names = env.capture(&["ls", "-1", &dir]).unwrap_or_default();
    let mut generations = Vec::new();
    // Every slot entry and fallback entry as it is after this loop (QA-BOOT-13: `keep_twins`).
    let mut entries: Vec<(String, String)> = Vec::new();
    for name in names.lines() {
        let twin = update::SLOTS
            .into_iter()
            .any(|s| seal::twin::counter(s, seal::twin::TAG, name).is_some());
        let Some(slot) = update::SLOTS.into_iter().find(|s| {
            name.strip_prefix(s)
                .is_some_and(|r| r == ".conf" || r.starts_with('+'))
        }) else {
            if twin && let Some(text) = env.read_file(&PathBuf::from(format!("{dir}/{name}"))) {
                entries.push((name.to_owned(), text));
            }
            continue;
        };
        let path = PathBuf::from(format!("{dir}/{name}"));
        let Some(text) = env.read_file(&path) else {
            entries.push((name.to_owned(), String::new()));
            continue;
        };
        generations.extend(boot::entry_generation(&text));
        let new: String = text
            .lines()
            .map(|l| match l.strip_prefix("title ") {
                Some(t) => {
                    let t = t.trim_end_matches(PREVIOUS);
                    let tag = if slot == running { "" } else { PREVIOUS };
                    format!("title {t}{tag}\n")
                }
                None => format!("{l}\n"),
            })
            .collect();
        if new != text {
            let _ = env.write_file(&path, &new);
        }
        entries.push((name.to_owned(), new));
    }
    let floor = boot::next_floor(number(env.read_file(&state("floor"))), &generations);
    if env
        .write_file(&state("floor"), &format!("{floor}\n"))
        .is_err()
    {
        return fail(
            env,
            ErrorCode::Failed,
            "could not record the rollback floor",
        );
    }
    // QA-LIVE4-4: the other slot's image is damaged (the firmware could not load it, so this
    // boot was picked from the menu): this slot is the default from now on, or every cold start
    // fails the same way. The next update unpins it (`bootctl set-default ""`).
    if let Some(other) = update::other_slot(running)
        && crate::seal::image_damaged(env, other)
    {
        let set = vec![
            "bootctl".to_owned(),
            "set-default".to_owned(),
            format!("{running}.conf"),
        ];
        let msg = format!(
            "The other version of WolOS ({other}) is damaged and cannot start. This version \
             ({running}) starts from now on; the next update replaces the damaged one.\n"
        );
        let _ = env.write_file(Path::new(seal::NOTICE), &msg);
        env.err(&msg);
        if env.exec(&set).unwrap_or(1) != 0 {
            return fail(
                env,
                ErrorCode::Failed,
                "could not make this version the one that starts",
            );
        }
    }
    // QA-LIVE4-3: the other slot spent the one counted try it gets at every shutdown
    // (boot-rearm) and did not finish that start, yet its image is whole: the cut came from
    // elsewhere (a power cut while it started). It gets three tries again and is the newest, so
    // the next start goes back to it; a damaged image stays behind (above).
    if let Some(other) = update::other_slot(running)
        && names.lines().any(|n| n == format!("{other}+0-1.conf"))
        && !crate::seal::image_damaged(env, other)
    {
        if let Err(msg) = rearm_probe(env, &dir, other) {
            return fail(env, ErrorCode::Failed, &msg);
        }
        let from = format!("{other}+0-1.conf");
        for e in entries.iter_mut().filter(|e| e.0 == from) {
            e.0 = format!("{other}+3.conf");
        }
    }
    keep_twins(env, &dir, running, &entries);
    // The line the service log shows: which slot was marked good and where the floor stands.
    env.out(&format!(
        "{running} is marked as a good start; the rollback floor is {floor}.\n"
    ));
    0
}

/// QA-BOOT-13 (ADR 0025, fallback entry): each slot's fallback entry follows its own entry (the
/// same counter, the same image; written for a system installed before it had one), and a slot
/// whose own entry is lost or garbled while it started from its fallback gets the entry back.
/// Each change is on the disk (the ESP flushed) before the next, so a cut leaves at most one of
/// the two files damaged. A failure is said and changes nothing else: the bless stands.
fn keep_twins(env: &mut dyn Env, dir: &str, running: &str, entries: &[(String, String)]) {
    let pairs: Vec<(&str, &str)> = entries
        .iter()
        .map(|(n, t)| (n.as_str(), t.as_str()))
        .collect();
    for slot in update::SLOTS {
        for step in seal::twin::steps(slot, slot == running, &pairs) {
            let done = match &step {
                seal::twin::Step::Write { name, text } => {
                    if name.starts_with(&format!("{slot}.")) {
                        env.err(&format!(
                            "The menu entry of {slot} was missing or damaged; it is written again \
                             from its fallback entry.\n"
                        ));
                    }
                    env.write_file(&PathBuf::from(format!("{dir}/{name}")), text)
                }
                seal::twin::Step::Remove { name } => {
                    env.remove_file(&PathBuf::from(format!("{dir}/{name}")))
                }
            };
            let flushed = done.is_ok()
                && env
                    .exec(&["sync".to_owned(), "-f".to_owned(), update::ESP.to_owned()])
                    .unwrap_or(1)
                    == 0;
            if !flushed {
                env.err(&format!(
                    "Could not keep the fallback menu entry of {slot} in step ({step:?}); the next \
                     good start tries again.\n"
                ));
                break;
            }
        }
    }
}

/// `other+0-1.conf` -> `other+3.conf` in the entries folder `dir`: the new name on the disk
/// before the old one goes (FAT, a power cut: never a moment without the slot's entry).
fn rearm_probe(env: &mut dyn Env, dir: &str, other: &str) -> Result<(), String> {
    let from = PathBuf::from(format!("{dir}/{other}+0-1.conf"));
    let to = PathBuf::from(format!("{dir}/{other}+3.conf"));
    let text = env
        .read_file(&from)
        .ok_or_else(|| format!("could not read {}", from.display()))?;
    env.write_file(&to, &text)
        .map_err(|e| format!("could not give {other} its tries back: {e}"))?;
    let flushed = env
        .exec(&["sync".to_owned(), "-f".to_owned(), update::ESP.to_owned()])
        .unwrap_or(1);
    if flushed != 0 {
        return Err(format!("could not flush the startup partition for {other}"));
    }
    env.remove_file(&from)
        .map_err(|e| format!("could not remove {}: {e}", from.display()))?;
    env.err(&format!(
        "The other version of WolOS ({other}) did not finish its last start, but its image is \
         whole: it starts again next time, with three tries.\n"
    ));
    Ok(())
}
