//! "Encrypt disk" at install: LUKS2 (argon2id) under btrfs. The disk is formatted with a
//! throw-away random key; that key unlocks the volume to enroll the real ones (TPM2 when the
//! machine has one, the person's passphrase otherwise, always a recovery key shown once), then
//! the throw-away key slot is removed. Pure: decisions, argv lists, boot options. Secrets are
//! always files or stdin, never arguments (`ps` shows arguments).

/// Name of the opened volume (`/dev/mapper/wolosroot`).
pub const MAPPER: &str = "wolosroot";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// On when a TPM2 exists.
    Auto,
    Yes,
    No,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "auto" => Some(Mode::Auto),
            "yes" | "on" => Some(Mode::Yes),
            "no" | "off" => Some(Mode::No),
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

/// `/sys/class/tpm/tpm0/tpm_version_major` says 2.
pub fn has_tpm2(version_major: Option<&str>) -> bool {
    version_major.is_some_and(|v| v.trim() == "2")
}

/// The Install app's checkbox default.
pub fn default_on(tpm2: bool) -> bool {
    tpm2
}

pub fn decide(mode: Mode, tpm2: bool, passphrase: Option<&str>) -> Result<Unlock, Refusal> {
    let on = match mode {
        Mode::Auto => default_on(tpm2),
        m => m == Mode::Yes,
    };
    if !on {
        return Ok(Unlock::Off);
    }
    if tpm2 {
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
pub fn enroll(part: &str, key: &str, unlock: Unlock, pass: Option<&str>) -> Vec<Vec<String>> {
    let unlock_key = format!("--unlock-key-file={key}");
    let mut s = vec![v(&[
        "systemd-cryptenroll",
        "--recovery-key",
        &unlock_key,
        part,
    ])];
    match (unlock, pass) {
        (Unlock::Tpm2, _) => s.push(v(&[
            "systemd-cryptenroll",
            "--tpm2-device=auto",
            "--tpm2-pcrs=7",
            &unlock_key,
            part,
        ])),
        (Unlock::Passphrase, Some(p)) => s.push(v(&[
            "cryptsetup",
            "luksAddKey",
            "--batch-mode",
            &format!("--key-file={key}"),
            part,
            p,
        ])),
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

pub fn open_argv(part: &str, key: &str) -> Vec<String> {
    v(&[
        "cryptsetup",
        "open",
        &format!("--key-file={key}"),
        part,
        MAPPER,
    ])
}

/// Kernel command line words for the initramfs (`sd-encrypt`).
pub fn cmdline_words(luks_uuid: &str, unlock: Unlock) -> String {
    let opts = if unlock == Unlock::Tpm2 {
        "tpm2-device=auto"
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
    let root = if keep.iter().any(|w| w.starts_with("root=")) {
        String::new()
    } else {
        "root=LABEL=wolos ".into()
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
    fn on_by_default_only_with_a_tpm() {
        assert!(has_tpm2(Some("2\n")) && !has_tpm2(Some("1")) && !has_tpm2(None));
        assert_eq!(decide(Mode::Auto, true, None), Ok(Unlock::Tpm2));
        assert_eq!(decide(Mode::Auto, false, None), Ok(Unlock::Off));
        assert_eq!(decide(Mode::No, true, None), Ok(Unlock::Off));
        assert_eq!(decide(Mode::Yes, true, None), Ok(Unlock::Tpm2));
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
        let t: Vec<String> = enroll("/dev/sda2", "/run/k", Unlock::Tpm2, None)
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
        let p = enroll("/dev/sda2", "/run/k", Unlock::Passphrase, Some("/run/p"));
        assert!(p[1].join(" ").starts_with("cryptsetup luksAddKey"));
        assert_eq!(p.last().unwrap()[1], "luksRemoveKey");
        assert_eq!(enroll("/d", "/k", Unlock::Passphrase, Some("/p")).len(), 3);
    }

    #[test]
    fn initramfs_and_boot_entry_words() {
        assert_eq!(
            cmdline_words("U-1", Unlock::Tpm2),
            "rd.luks.name=U-1=wolosroot rd.luks.options=U-1=tpm2-device=auto root=/dev/mapper/wolosroot"
        );
        assert!(cmdline_words("U-1", Unlock::Passphrase).contains("=discard"));
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
            "root=LABEL=wolos rootflags=subvol=@wolos-b ro quiet splash"
        );
    }
}
