// SPDX-License-Identifier: GPL-2.0-or-later
//! The boot chain (ADR 0025). Firmware facts decide the protection tier; the chain, the kernel
//! command line words and the words shown to the person follow from it. Pure: the `wol boot`
//! verbs read the firmware variables and run the tools.

/// EFI global variables (`SecureBoot`, `SetupMode`).
pub const EFI_GLOBAL: &str = "8be4df61-93ca-11d2-aa0d-00e098032b8c";
/// EFI image security database (`db`).
pub const EFI_DB: &str = "d719b2cb-3d3a-4596-a3bc-dad00e67656f";
/// Where the machine's own keys and the chosen tier live (shared `@var`, both slots use them).
pub const STATE_DIR: &str = "/var/lib/wolos/secureboot";
/// The password typed once on the firmware's blue key screen. It proves a person is at the
/// keyboard; it is not a secret. Its letters sit on the same keys on QWERTY, QWERTZ and AZERTY.
pub const MOK_PASSWORD: &str = "boot";
/// Subjects of the certificates that let a distribution's first-stage loader start.
const THIRD_PARTY_CA: [&[u8]; 2] = [
    b"Microsoft Corporation UEFI CA 2011",
    b"Microsoft UEFI CA 2023",
];

/// What the firmware and hardware offer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Firmware {
    pub uefi: bool,
    pub secure_boot: bool,
    pub setup_mode: bool,
    /// `db` holds a third-party CA, so the signed first stage can start.
    pub third_party_ca: bool,
    /// A GPU that needs a module built outside the kernel tree.
    pub out_of_tree: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Setup Mode: the machine's own keys go into the firmware.
    OwnKeys,
    /// The machine's key is enrolled once on the firmware's key screen.
    Mok,
    /// No enforcement: installs and starts, and says so.
    Off,
    /// Secure Boot is on and nothing we can ship will start until a firmware setting changes.
    Firmware,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::OwnKeys => "own-keys",
            Tier::Mok => "mok",
            Tier::Off => "off",
            Tier::Firmware => "firmware",
        }
    }
    pub fn parse(s: &str) -> Option<Tier> {
        [Tier::OwnKeys, Tier::Mok, Tier::Off, Tier::Firmware]
            .into_iter()
            .find(|t| t.as_str() == s.trim())
    }
}

/// The strongest chain this machine can run. `want`: the person kept Secure Boot protection on.
pub fn choose(fw: &Firmware, want: bool) -> Tier {
    if !fw.uefi || !want {
        return if fw.secure_boot {
            Tier::Firmware
        } else {
            Tier::Off
        };
    }
    if fw.setup_mode {
        Tier::OwnKeys
    } else if fw.third_party_ca {
        Tier::Mok
    } else if fw.secure_boot {
        Tier::Firmware
    } else {
        Tier::Off
    }
}

/// The key screen is needed once: for the tier itself, or because a module built on this machine
/// can only be trusted through it (the kernel takes module keys from there, not from `db`).
pub fn needs_key_screen(tier: Tier, fw: &Firmware) -> bool {
    tier == Tier::Mok || (tier == Tier::OwnKeys && fw.out_of_tree)
}

/// Kernel command line words of a protected chain.
pub fn protected_words(tier: Tier) -> &'static str {
    match tier {
        Tier::OwnKeys | Tier::Mok => "lockdown=integrity module.sig_enforce=1",
        Tier::Off | Tier::Firmware => "",
    }
}

/// PCRs the disk key is bound to besides the signed kernel policy: 14 holds the enrolled keys.
pub fn tpm_pcrs(chain_has_shim: bool) -> &'static str {
    if chain_has_shim { "7+14" } else { "7" }
}

/// What the installer says about the tier, before the restart.
pub fn note(tier: Tier) -> &'static str {
    match tier {
        Tier::OwnKeys => {
            "Secure Boot: this computer will trust only its own WolOS keys, plus Microsoft's so \
             Windows and add-in cards keep working."
        }
        Tier::Mok => {
            "Secure Boot: after the restart a blue screen asks once to accept this computer's \
             WolOS key."
        }
        Tier::Off => {
            "Secure Boot is off. WolOS starts normally, but the computer will not notice if \
             someone changes the system while it is off. You can turn it on later in Settings."
        }
        Tier::Firmware => {
            "Secure Boot is on and this computer only trusts Windows. In the firmware settings, \
             allow third-party certificates or clear the Secure Boot keys, then install again."
        }
    }
}

/// The blue screen, explained before it appears. Steps name what the firmware shows.
pub fn key_screen_steps() -> Vec<String> {
    vec![
        "The next screen is blue and comes from your computer's firmware, not from WolOS. It \
         waits ten seconds, so have this page in mind."
            .into(),
        "Press a key, choose Enroll MOK, then Continue, then Yes.".into(),
        format!("When it asks for a password, type {MOK_PASSWORD}. Nothing shows while you type."),
        "Choose Reboot. You will not see this screen again.".into(),
    ]
}

/// EFI boolean variable: four attribute bytes, then the value.
pub fn efi_bool(var: &[u8]) -> bool {
    var.get(4) == Some(&1)
}

/// `db` contents (any EFI signature list) name a CA that lets the first stage start.
pub fn has_third_party_ca(db: &[u8]) -> bool {
    THIRD_PARTY_CA
        .iter()
        .any(|ca| db.windows(ca.len()).any(|w| w == *ca))
}

/// PCI vendor and class from sysfs: an NVIDIA display controller.
pub fn is_nvidia_display(vendor: &str, class: &str) -> bool {
    vendor.trim() == "0x10de" && class.trim().starts_with("0x03")
}

/// A `key=value` word of the kernel command line.
pub fn cmdline_word<'a>(cmdline: &'a str, key: &str) -> Option<&'a str> {
    cmdline
        .split_whitespace()
        .find_map(|w| w.strip_prefix(key)?.strip_prefix('='))
}

/// Generation of a boot entry: the leading number of its `version` line. Sorting by it (newest
/// first, bad entries last) is what makes a failed update fall back on its own.
pub fn entry_generation(entry: &str) -> Option<u64> {
    entry
        .lines()
        .find_map(|l| l.strip_prefix("version "))?
        .trim()
        .split('-')
        .next()?
        .parse()
        .ok()
}

/// The oldest generation that may still start: the older of the slots kept, so going back to
/// the previous version keeps working and anything older is refused.
pub fn epoch_floor(kept: &[u64]) -> u64 {
    kept.iter().copied().min().unwrap_or(0)
}

/// The floor never moves back.
pub fn next_floor(stored: u64, kept: &[u64]) -> u64 {
    stored.max(epoch_floor(kept))
}

/// A signed kernel older than the floor is a rollback. No epoch word: a system from before.
pub fn epoch_allowed(epoch: Option<u64>, floor: u64) -> bool {
    epoch.is_none_or(|e| e >= floor)
}

/// `wol boot status`, the Settings row's source.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Status {
    pub secure_boot: bool,
    pub setup_mode: bool,
    pub tier: String,
    pub lockdown: String,
    pub tpm2: bool,
    pub measured: bool,
    pub sealed: bool,
    pub epoch: Option<u64>,
    pub floor: u64,
}

impl Status {
    /// One line for Settings, in plain words.
    pub fn summary(&self) -> String {
        match (self.secure_boot, self.tier.as_str()) {
            (true, "own-keys") => "On, with this computer's own keys".into(),
            (true, "mok") => "On, with this computer's enrolled key".into(),
            (true, _) => "On".into(),
            (false, "own-keys" | "mok") => "Off in the firmware settings".into(),
            (false, _) => "Off".into(),
        }
    }
}

/// Bracketed word of `/sys/kernel/security/lockdown` (`none [integrity] confidentiality`).
pub fn parse_lockdown(text: &str) -> String {
    text.split_once('[')
        .and_then(|(_, r)| r.split_once(']'))
        .map_or_else(|| "none".to_owned(), |(w, _)| w.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fw(secure_boot: bool, setup_mode: bool, third_party_ca: bool) -> Firmware {
        Firmware {
            uefi: true,
            secure_boot,
            setup_mode,
            third_party_ca,
            out_of_tree: false,
        }
    }

    #[test]
    fn the_strongest_chain_the_firmware_allows() {
        for (f, want, t) in [
            (fw(false, true, false), true, Tier::OwnKeys),
            (fw(false, true, true), true, Tier::OwnKeys),
            (fw(true, false, true), true, Tier::Mok),
            (fw(false, false, true), true, Tier::Mok),
            (fw(false, false, false), true, Tier::Off),
            (fw(true, false, false), true, Tier::Firmware),
            (fw(false, true, false), false, Tier::Off),
            (fw(true, false, true), false, Tier::Firmware),
        ] {
            assert_eq!(choose(&f, want), t, "{f:?} want={want}");
        }
        assert_eq!(choose(&Firmware::default(), true), Tier::Off, "BIOS boot");
    }

    #[test]
    fn the_key_screen_appears_only_when_it_does_something() {
        let plain = fw(false, true, true);
        let nvidia = Firmware {
            out_of_tree: true,
            ..plain
        };
        assert!(needs_key_screen(Tier::Mok, &plain));
        assert!(!needs_key_screen(Tier::OwnKeys, &plain));
        assert!(needs_key_screen(Tier::OwnKeys, &nvidia));
        assert!(!needs_key_screen(Tier::Off, &nvidia));
    }

    #[test]
    fn the_password_survives_common_keyboard_layouts() {
        // Letters that move on AZERTY or QWERTZ, and digits (shifted on AZERTY), are out.
        assert!(
            MOK_PASSWORD
                .chars()
                .all(|c| !"aqwzym0123456789".contains(c))
        );
        assert!(key_screen_steps().iter().any(|s| s.contains(MOK_PASSWORD)));
    }

    #[test]
    fn firmware_variables() {
        assert!(efi_bool(&[6, 0, 0, 0, 1]));
        assert!(!efi_bool(&[6, 0, 0, 0, 0]));
        assert!(!efi_bool(&[6, 0]));
        let mut db = vec![0u8; 64];
        assert!(!has_third_party_ca(&db));
        db.extend_from_slice(b"..CN=Microsoft Corporation UEFI CA 2011..");
        assert!(has_third_party_ca(&db));
        assert!(has_third_party_ca(b"xx Microsoft UEFI CA 2023 yy"));
        assert!(!has_third_party_ca(b"Windows UEFI CA 2023"));
        assert!(is_nvidia_display("0x10de\n", "0x030000\n"));
        assert!(
            !is_nvidia_display("0x10de", "0x040300"),
            "its audio function"
        );
        assert!(!is_nvidia_display("0x8086", "0x030000"));
    }

    #[test]
    fn tiers_round_trip_and_protect() {
        for t in [Tier::OwnKeys, Tier::Mok, Tier::Off, Tier::Firmware] {
            assert_eq!(Tier::parse(t.as_str()), Some(t));
            assert!(!note(t).is_empty());
        }
        assert_eq!(Tier::parse("x"), None);
        assert!(protected_words(Tier::Mok).contains("lockdown=integrity"));
        assert!(protected_words(Tier::OwnKeys).contains("module.sig_enforce=1"));
        assert_eq!(protected_words(Tier::Off), "");
        assert_eq!(tpm_pcrs(true), "7+14");
        assert_eq!(tpm_pcrs(false), "7");
    }

    #[test]
    fn a_rollback_below_the_kept_slots_is_refused() {
        assert_eq!(entry_generation("title W\nversion 7-2026.11.02\n"), Some(7));
        assert_eq!(entry_generation("version 2026/10/20\n"), None);
        assert_eq!(entry_generation("title W\n"), None);
        assert_eq!(epoch_floor(&[7, 6]), 6);
        assert_eq!(epoch_floor(&[]), 0);
        assert_eq!(next_floor(5, &[7, 6]), 6);
        assert_eq!(next_floor(9, &[7, 6]), 9, "never moves back");
        assert!(
            epoch_allowed(Some(6), 6),
            "the previous version still starts"
        );
        assert!(!epoch_allowed(Some(5), 6));
        assert!(epoch_allowed(None, 6));
        let c = "root=x wolos.epoch=12 wolos.seal=ab quiet";
        assert_eq!(cmdline_word(c, "wolos.epoch"), Some("12"));
        assert_eq!(cmdline_word(c, "wolos.seal"), Some("ab"));
        assert_eq!(cmdline_word(c, "wolos.none"), None);
    }

    #[test]
    fn status_in_plain_words() {
        assert_eq!(
            parse_lockdown("none [integrity] confidentiality\n"),
            "integrity"
        );
        assert_eq!(parse_lockdown("[none] integrity"), "none");
        assert_eq!(parse_lockdown(""), "none");
        let s = |secure_boot, tier: &str| Status {
            secure_boot,
            tier: tier.into(),
            ..Status::default()
        };
        assert!(s(true, "own-keys").summary().contains("own keys"));
        assert!(s(true, "mok").summary().contains("enrolled"));
        assert!(s(false, "mok").summary().contains("firmware"));
        assert_eq!(s(false, "off").summary(), "Off");
    }
}
