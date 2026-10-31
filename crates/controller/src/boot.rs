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
    match env.capture(&["/usr/lib/wolos/boot-enroll"]).as_deref().map(str::trim) {
        Some("enrolled") => env.out("This computer's key is already enrolled.\n"),
        Some(_) => {
            env.out("Restart to finish.\n");
            for s in boot::key_screen_steps() {
                env.out(&format!("{s}\n"));
            }
        }
        None => return fail(env, ErrorCode::Failed, "the key could not be queued; run this as root"),
    }
    0
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

/// `wol boot note TIER [yes|no]`: the note, then the key screen's steps when it will appear.
pub fn note(env: &mut dyn Env, inv: &Invocation) -> i32 {
    match tier_arg(env, inv) {
        Ok(t) => {
            env.out(&format!("{}\n", boot::note(t)));
            if inv.get("screen") == Some("yes") {
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
    if boot::epoch_allowed(epoch, number(env.read_file(&state("floor")))) {
        return 0;
    }
    let fb = seal::fallback(slot, true);
    let msg = format!(
        "This copy of WolOS is older than the version last used here, so it will not stay running. \
         Slot {slot} was refused.\n"
    );
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
    for name in names.lines() {
        let Some(slot) = update::SLOTS.into_iter().find(|s| {
            name.strip_prefix(s)
                .is_some_and(|r| r == ".conf" || r.starts_with('+'))
        }) else {
            continue;
        };
        let path = PathBuf::from(format!("{dir}/{name}"));
        let Some(text) = env.read_file(&path) else {
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
    0
}
