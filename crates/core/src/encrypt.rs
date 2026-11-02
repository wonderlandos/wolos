// SPDX-License-Identifier: BSD-2-Clause
//! "Encrypt disk" at install: LUKS2 (argon2id) under btrfs. The disk is formatted with a
//! throw-away random key; that key unlocks the volume to enroll the real ones (TPM2 when the
//! machine has one, the person's passphrase otherwise, always a recovery key shown once), then
//! the throw-away key slot is removed. Pure: decisions, argv lists, boot options. Secrets are
//! always files or stdin, never arguments (`ps` shows arguments).

/// Name of the opened volume (`/dev/mapper/wolosroot`).
pub const MAPPER: &str = "wolosroot";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// On: TPM2 when the machine has one, else a passphrase is needed.
    Auto,
    Yes,
    No,
    /// A password at every start, even where a TPM2 could unlock it.
    Passphrase,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "auto" => Some(Mode::Auto),
            "yes" | "on" => Some(Mode::Yes),
            "no" | "off" => Some(Mode::No),
            "passphrase" => Some(Mode::Passphrase),
            _ => None,
        }
    }
}

/// How the volume is unlocked at boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unlock {
    Off,
    Tpm2,
    Passphrase,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Encryption without a TPM needs a passphrase from the person.
    NeedPassphrase,
    TooShort,
}

pub const MIN_PASSPHRASE: usize = 8;

/// Why a secret's file is not something a person typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadSecret {
    /// Not UTF-8 text.
    NotText,
    /// A control character (a tab, a second line, an escape): nobody types it at the prompt.
    ControlCharacter,
}

/// The one reading of a password or passphrase file (audit3 R2B-6): what the person typed, from
/// the file's bytes. One line end goes (`\n`, or `\r\n` from a Windows editor: what `echo`
/// leaves), nothing else is trimmed, so the length checked is the length enrolled. Err: not
/// text, or a control character left inside. `wol encrypt decide|enroll`, `wol install
/// check|account` and `plan::unlock_problem` all read secrets through this.
pub fn typed_secret(bytes: &[u8]) -> Result<String, BadSecret> {
    let s = std::str::from_utf8(bytes).map_err(|_| BadSecret::NotText)?;
    let s = s
        .strip_suffix('\n')
        .map_or(s, |t| t.strip_suffix('\r').unwrap_or(t));
    if s.chars().any(char::is_control) {
        return Err(BadSecret::ControlCharacter);
    }
    Ok(s.to_owned())
}

/// `/sys/class/tpm/tpm0/tpm_version_major` says 2.
pub fn has_tpm2(version_major: Option<&str>) -> bool {
    version_major.is_some_and(|v| v.trim() == "2")
}

/// The Install app's default: the disk is encrypted whether or not a TPM2 exists (with one the
/// chip unlocks it, without one the person types a passphrase). Opting out stays possible.
pub fn default_on(_tpm2: bool) -> bool {
    true
}

pub fn decide(mode: Mode, tpm2: bool, passphrase: Option<&str>) -> Result<Unlock, Refusal> {
    let on = match mode {
        Mode::Auto => default_on(tpm2),
        m => m != Mode::No,
    };
    if !on {
        return Ok(Unlock::Off);
    }
    if tpm2 && mode != Mode::Passphrase {
        return Ok(Unlock::Tpm2);
    }
    match passphrase {
        None => Err(Refusal::NeedPassphrase),
        Some(p) if p.chars().count() < MIN_PASSPHRASE => Err(Refusal::TooShort),
        Some(_) => Ok(Unlock::Passphrase),
    }
}

fn v(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| (*s).to_owned()).collect()
}

/// systemd-repart flags for the encrypted root (`Encrypt=key-file` in the definition).
pub fn repart_flags(keyfile: &str) -> Vec<String> {
    vec![format!("--key-file={keyfile}")]
}

/// Commands after partitioning, each run in order; all must succeed. `part`: the LUKS partition,
/// `key`: the throw-away key file, `pass`: the passphrase file (Unlock::Passphrase).
/// The recovery key comes on stdout of the `--recovery-key` step (the caller keeps it once).
/// `tpm_pub`: the machine's TPM policy key. The disk then follows any kernel it signed, and `key`
/// stays until the first boot, which binds the firmware state ([`rebind`]).
pub fn enroll(
    part: &str,
    key: &str,
    unlock: Unlock,
    pass: Option<&str>,
    tpm_pub: Option<&str>,
) -> Vec<Vec<String>> {
    let unlock_key = format!("--unlock-key-file={key}");
    let mut s = vec![v(&[
        "systemd-cryptenroll",
        "--recovery-key",
        &unlock_key,
        part,
    ])];
    let add_pass = |p: &str| {
        v(&[
            "cryptsetup",
            "luksAddKey",
            "--batch-mode",
            &format!("--key-file={key}"),
            part,
            p,
        ])
    };
    match (unlock, pass) {
        (Unlock::Tpm2, _) => {
            // The fallback when the chip refuses (firmware or Secure Boot changed): the person's
            // own account password, never a shared one. Then the recovery key.
            if let Some(p) = pass {
                s.push(add_pass(p));
            }
            match tpm_pub {
                Some(p) => {
                    s.push(v(&[
                        "systemd-cryptenroll",
                        "--tpm2-device=auto",
                        "--tpm2-pcrs=",
                        &format!("--tpm2-public-key={p}"),
                        "--tpm2-public-key-pcrs=11",
                        &unlock_key,
                        part,
                    ]));
                    return s;
                }
                None => s.push(v(&[
                    "systemd-cryptenroll",
                    "--tpm2-device=auto",
                    "--tpm2-pcrs=7",
                    &unlock_key,
                    part,
                ])),
            }
        }
        (Unlock::Passphrase, Some(p)) => s.push(add_pass(p)),
        _ => {}
    }
    // The throw-away key goes last: nothing but the real ones opens the disk afterwards.
    s.push(v(&[
        "cryptsetup",
        "luksRemoveKey",
        "--batch-mode",
        part,
        key,
    ]));
    s
}

/// First boot: the firmware's keys are final, so PCR values read now are the ones later boots
/// will show. Replaces the TPM slot, then the install key goes.
pub fn rebind(part: &str, key: &str, tpm_pub: &str, pcrs: &str) -> Vec<Vec<String>> {
    vec![
        v(&[
            "systemd-cryptenroll",
            "--wipe-slot=tpm2",
            "--tpm2-device=auto",
            &format!("--tpm2-pcrs={pcrs}"),
            &format!("--tpm2-public-key={tpm_pub}"),
            "--tpm2-public-key-pcrs=11",
            &format!("--unlock-key-file={key}"),
            part,
        ]),
        v(&["cryptsetup", "luksRemoveKey", "--batch-mode", part, key]),
        v(&["rm", "-f", key]),
    ]
}

/// Kernel keyring entry (`user` type, root's user keyring) where systemd-cryptsetup caches what
/// was typed at the unlock prompt (`password-cache=yes`, the default; about 150 s). Present after
/// boot only when the chip did not unlock the disk: the password or recovery key was typed.
pub const PASSWORD_CACHE_KEY: &str = "cryptsetup";

/// The account whose password is the disk's fallback slot (written by the installer, TPM disks only).
pub const UNLOCK_USER_FILE: &str = "/var/lib/wolos/disk-unlock-user";

/// The keyring entry's payload: NUL-separated, one per try (wrong tries too).
pub fn parse_cache(raw: &[u8]) -> Vec<String> {
    raw.split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .filter_map(|p| std::str::from_utf8(p).ok().map(str::to_owned))
        .collect()
}

/// After a boot the chip refused: seal the disk key again to the firmware state measured now, so
/// the next start is automatic again. Same policy as [`rebind`]; the typed secret unlocks it
/// (`$PASSWORD`, never an argument). The enrollment happens before the old TPM slot is wiped.
pub fn reseal(part: &str, tpm_pub: Option<&str>, pcrs: &str) -> Vec<String> {
    let mut a = v(&[
        "systemd-cryptenroll",
        "--wipe-slot=tpm2",
        "--tpm2-device=auto",
        &format!("--tpm2-pcrs={pcrs}"),
    ]);
    if let Some(p) = tpm_pub {
        a.push(format!("--tpm2-public-key={p}"));
        a.push("--tpm2-public-key-pcrs=11".into());
    }
    a.push(part.into());
    a
}

/// The account password changed: replace the disk's password slot (the chip unlocks it; the new
/// password on `$NEWPASSWORD`). The recovery key and the TPM slot stay.
pub fn password_argv(part: &str) -> Vec<String> {
    v(&[
        "systemd-cryptenroll",
        "--unlock-tpm2-device=auto",
        "--wipe-slot=password",
        "--password",
        part,
    ])
}

pub fn open_argv(part: &str, key: &str) -> Vec<String> {
    v(&[
        "cryptsetup",
        "open",
        &format!("--key-file={key}"),
        part,
        MAPPER,
    ])
}

/// Kernel command line words for the initramfs (`sd-encrypt`). `discard` either way: SSD TRIM
/// reaches the disk through the encrypted volume, whatever unlocks it.
pub fn cmdline_words(luks_uuid: &str, unlock: Unlock) -> String {
    let opts = if unlock == Unlock::Tpm2 {
        "tpm2-device=auto,discard"
    } else {
        "discard"
    };
    format!(
        "rd.luks.name={luks_uuid}={MAPPER} rd.luks.options={luks_uuid}={opts} root=/dev/mapper/{MAPPER}"
    )
}

/// mkinitcpio HOOKS with `sd-encrypt` before `filesystems`.
pub fn hooks(line: &str) -> String {
    if line.contains("sd-encrypt") {
        return line.to_owned();
    }
    line.replace(" filesystems", " sd-encrypt filesystems")
}

/// Kernel command line for the other slot's boot entry: the running system's root words
/// (root=, rd.luks.*) kept, the slot swapped, read-only (sealed).
pub fn slot_options(running_cmdline: Option<&str>, slot: &str) -> String {
    let keep: Vec<&str> = running_cmdline
        .unwrap_or("")
        .split_whitespace()
        .filter(|w| w.starts_with("root=") || w.starts_with("rd.luks."))
        .collect();
    // The label repart.d/20-root.conf writes; by-label lookups are case-sensitive.
    let root = if keep.iter().any(|w| w.starts_with("root=")) {
        String::new()
    } else {
        "root=LABEL=WolOS ".into()
    };
    format!(
        "{root}{} rootflags=subvol=@{slot} ro quiet splash",
        keep.join(" ")
    )
    .replace("  ", " ")
    .trim_start()
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_what_was_typed() {
        assert_eq!(
            typed_secret(b"white rabbit\r\n").as_deref(),
            Ok("white rabbit")
        );
        assert_eq!(
            typed_secret(b"white rabbit\n").as_deref(),
            Ok("white rabbit")
        );
        assert_eq!(typed_secret(b"white rabbit").as_deref(), Ok("white rabbit"));
        assert_eq!(typed_secret(b"\n").as_deref(), Ok(""));
        assert_eq!(
            typed_secret("café au lait\n".as_bytes()).as_deref(),
            Ok("café au lait")
        );
        assert_eq!(typed_secret(b"\xff"), Err(BadSecret::NotText));
        assert_eq!(
            typed_secret(b"two\nlines\n"),
            Err(BadSecret::ControlCharacter)
        );
        assert_eq!(typed_secret(b"tab\there"), Err(BadSecret::ControlCharacter));
        assert_eq!(typed_secret(b"pass\r"), Err(BadSecret::ControlCharacter));
        // Eight typed characters stay eight: decide and enroll see the same secret.
        let p = typed_secret(b"12345678\r\n").unwrap();
        assert_eq!(
            decide(Mode::Passphrase, false, Some(&p)),
            Ok(Unlock::Passphrase)
        );
        let p = typed_secret(b"1234567\r\n").unwrap();
        assert_eq!(
            decide(Mode::Passphrase, false, Some(&p)),
            Err(Refusal::TooShort)
        );
    }

    #[test]
    fn on_by_default_with_or_without_a_tpm() {
        assert!(has_tpm2(Some("2\n")) && !has_tpm2(Some("1")) && !has_tpm2(None));
        assert!(default_on(true) && default_on(false));
        assert_eq!(decide(Mode::Auto, true, None), Ok(Unlock::Tpm2));
        assert_eq!(
            decide(Mode::Auto, false, None),
            Err(Refusal::NeedPassphrase),
            "no chip: the person's passphrase, never a plain disk by accident"
        );
        assert_eq!(
            decide(Mode::Auto, false, Some("correct horse")),
            Ok(Unlock::Passphrase)
        );
        assert_eq!(decide(Mode::No, true, None), Ok(Unlock::Off));
        assert_eq!(
            decide(Mode::No, false, None),
            Ok(Unlock::Off),
            "the opt-out"
        );
        assert_eq!(decide(Mode::Yes, true, None), Ok(Unlock::Tpm2));
        assert_eq!(
            decide(Mode::Passphrase, true, Some("correct horse")),
            Ok(Unlock::Passphrase),
            "the person chose a password over the chip"
        );
        assert_eq!(
            decide(Mode::Passphrase, true, None),
            Err(Refusal::NeedPassphrase)
        );
    }

    #[test]
    fn without_a_tpm_a_good_passphrase_is_required() {
        assert_eq!(decide(Mode::Yes, false, None), Err(Refusal::NeedPassphrase));
        assert_eq!(
            decide(Mode::Yes, false, Some("short")),
            Err(Refusal::TooShort)
        );
        assert_eq!(
            decide(Mode::Yes, false, Some("correct horse")),
            Ok(Unlock::Passphrase)
        );
    }

    #[test]
    fn mode_words() {
        assert_eq!(Mode::parse("auto"), Some(Mode::Auto));
        assert_eq!(Mode::parse("off"), Some(Mode::No));
        assert_eq!(Mode::parse("x"), None);
    }

    #[test]
    fn enrollment_order_and_no_secret_in_arguments() {
        let t: Vec<String> = enroll("/dev/sda2", "/run/k", Unlock::Tpm2, None, None)
            .iter()
            .map(|a| a.join(" "))
            .collect();
        assert_eq!(
            t[0],
            "systemd-cryptenroll --recovery-key --unlock-key-file=/run/k /dev/sda2"
        );
        assert_eq!(
            t[1],
            "systemd-cryptenroll --tpm2-device=auto --tpm2-pcrs=7 --unlock-key-file=/run/k /dev/sda2"
        );
        assert_eq!(
            t[2],
            "cryptsetup luksRemoveKey --batch-mode /dev/sda2 /run/k"
        );
        let p = enroll(
            "/dev/sda2",
            "/run/k",
            Unlock::Passphrase,
            Some("/run/p"),
            None,
        );
        assert!(p[1].join(" ").starts_with("cryptsetup luksAddKey"));
        assert_eq!(p.last().unwrap()[1], "luksRemoveKey");
        assert_eq!(
            enroll("/d", "/k", Unlock::Passphrase, Some("/p"), None).len(),
            3
        );
    }

    /// PCR 7 at install is not PCR 7 at the first real boot (the keys change it), so the disk key
    /// starts on the signed kernel policy and binds PCR 7 from inside the first boot.
    #[test]
    fn a_signed_policy_survives_updates_and_binds_the_firmware_state_at_first_boot() {
        let t: Vec<String> = enroll("/d", "/k", Unlock::Tpm2, None, Some("/p.pem"))
            .iter()
            .map(|a| a.join(" "))
            .collect();
        assert_eq!(
            t[1],
            "systemd-cryptenroll --tpm2-device=auto --tpm2-pcrs= --tpm2-public-key=/p.pem \
--tpm2-public-key-pcrs=11 --unlock-key-file=/k /d"
        );
        assert_eq!(t.len(), 2, "the install key stays for the first boot");
        let r: Vec<String> = rebind("/d", "/k", "/p.pem", "7+14")
            .iter()
            .map(|a| a.join(" "))
            .collect();
        assert_eq!(
            r,
            [
                "systemd-cryptenroll --wipe-slot=tpm2 --tpm2-device=auto --tpm2-pcrs=7+14 \
--tpm2-public-key=/p.pem --tpm2-public-key-pcrs=11 --unlock-key-file=/k /d",
                "cryptsetup luksRemoveKey --batch-mode /d /k",
                "rm -f /k"
            ]
        );
    }

    /// The chip refusing (Secure Boot turned off changes PCR 7) must not leave only the 64-character
    /// recovery key: the account password opens the disk too, and the next boot is automatic again.
    #[test]
    fn a_tpm_disk_also_opens_with_the_account_password_and_is_sealed_again_after_it() {
        for tpm_pub in [None, Some("/p.pem")] {
            let t: Vec<String> = enroll("/d", "/k", Unlock::Tpm2, Some("/run/pw"), tpm_pub)
                .iter()
                .map(|a| a.join(" "))
                .collect();
            assert!(t[0].starts_with("systemd-cryptenroll --recovery-key"));
            assert_eq!(
                t[1], "cryptsetup luksAddKey --batch-mode --key-file=/k /d /run/pw",
                "a file, never the password itself"
            );
            assert!(t[2].contains("--tpm2-device=auto"));
        }
        assert_eq!(
            reseal("/d", Some("/p.pem"), "7+14").join(" "),
            "systemd-cryptenroll --wipe-slot=tpm2 --tpm2-device=auto --tpm2-pcrs=7+14 --tpm2-public-key=/p.pem --tpm2-public-key-pcrs=11 /d"
        );
        assert_eq!(
            reseal("/d", None, "7").join(" "),
            "systemd-cryptenroll --wipe-slot=tpm2 --tpm2-device=auto --tpm2-pcrs=7 /d"
        );
        assert_eq!(
            password_argv("/d").join(" "),
            "systemd-cryptenroll --unlock-tpm2-device=auto --wipe-slot=password --password /d"
        );
        assert_eq!(parse_cache(b"wrong\0right pass\0"), ["wrong", "right pass"]);
        assert!(parse_cache(b"").is_empty());
    }

    #[test]
    fn initramfs_and_boot_entry_words() {
        assert_eq!(
            cmdline_words("U-1", Unlock::Tpm2),
            "rd.luks.name=U-1=wolosroot rd.luks.options=U-1=tpm2-device=auto,discard root=/dev/mapper/wolosroot"
        );
        assert!(cmdline_words("U-1", Unlock::Passphrase).contains("=discard"));
        // TRIM reaches an SSD under a TPM install too (audit2 B15).
        for u in [Unlock::Tpm2, Unlock::Passphrase] {
            assert!(
                cmdline_words("U-1", u)
                    .split_whitespace()
                    .any(|w| w.starts_with("rd.luks.options=") && w.ends_with("discard")),
                "{u:?}"
            );
        }
        let h = hooks("HOOKS=(base systemd block filesystems fsck)");
        assert_eq!(h, "HOOKS=(base systemd block sd-encrypt filesystems fsck)");
        assert_eq!(hooks(&h), h);
        let run = "BOOT_IMAGE=/x root=/dev/mapper/wolosroot rootflags=subvol=@wolos-a rw rd.luks.name=U=wolosroot quiet";
        assert_eq!(
            slot_options(Some(run), "wolos-b"),
            "root=/dev/mapper/wolosroot rd.luks.name=U=wolosroot rootflags=subvol=@wolos-b ro quiet splash"
        );
        assert_eq!(
            slot_options(None, "wolos-b"),
            "root=LABEL=WolOS rootflags=subvol=@wolos-b ro quiet splash"
        );
    }
}
