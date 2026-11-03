# Security

On from the first boot (ADR 0015). Code: `wol_core::security` (defaults, status), `wol_core::update::check`, `wol_core::home_crypt`.
`wol security [--json]` shows the state; `wol firewall on|off|status` changes the firewall through wol-admind.

## Defaults
- Firewall: nftables, `/etc/wolos/nftables.conf` (loaded by a `nftables.service` drop-in), `nftables.service` enabled.
  - Inbound dropped except replies, ping, DHCP, IPv6 neighbour discovery and mDNS. Outbound allowed.
- No remote login: `sshd.service` stays disabled until Sharing turns it on.
- The log stays local: journald only, no network syslog daemon.
- Kernel hardening (`/usr/lib/sysctl.d/90-wolos.conf`, one reason per line): `kernel.dmesg_restrict=1`
  (the kernel log is for administrators), `kernel.kptr_restrict=2` (kernel addresses hidden),
  `kernel.yama.ptrace_scope=1` (own children only), `kernel.sysrq=0` (no debugger key chord),
  `fs.protected_hardlinks/symlinks=1`, `fs.protected_fifos/regular=1` (no link or FIFO tricks in
  shared sticky folders such as /tmp), `fs.suid_dumpable=0` (no core files from setuid programs),
  `kernel.unprivileged_bpf_disabled=1` (only administrators load BPF), `net.core.bpf_jit_harden=1`
  (constant blinding for ordinary accounts' BPF), `net.ipv4.tcp_syncookies=1` (half-open floods),
  `kernel.kexec_load_disabled=1` (no kernel swap without a firmware restart). The same file raises
  `vm.max_map_count` for games. `wol security` reads them from /proc/sys; **CI** the controller test sysctl_docs
  fails when this line names a setting no file sets.

## Updates (ADR 0016, 0017)
- A systemd timer runs `/usr/lib/wolos/update-check` hourly; it checks at most every 20 h:
  `pacman -Sy` (pinned snapshot), `pacman -Qu`, `arch-audit -q`, `pacman -Qq` (kernels), and the
  signed release manifest (`release.json` + `.sig`, verified with the package keys when read).
- Findings go to `/var/lib/wolos/updates`, readable by every account. Never installs.
- Release manifest: version, Arch snapshot date, `[wolos]` revision, linux-wolos version, image URL and sha256.
- Update Now (wol-admind `pkg-upgrade`) = `pacman -Syu` inside the release: only `[wolos]` moves.
- A new release = `wol update apply` (root): btrfs snapshot of the running root into the other slot
  (`@wolos-a`/`@wolos-b`, previous restore point deleted first), mirrorlist moved to the new date,
  `arch-chroot ... pacman -Syu` (wolos-base moves as one unit), `mkinitcpio -P`, kernel to the ESP,
  boot entry `<slot>+3.conf` (boot counting, always linux-wolos), `bootctl set-default`.
  Any failure deletes the new slot. The running slot stays the restore point and the fallback.
- Packages the person installed with pacman come along (the slot is a copy, not an image).
  Other kernels are allowed, reported by `wol update status` and `wol games doctor`, never removed.
- `pacman -S/-R/-U` on the sealed system (`/usr/local/bin/pacman`, runs `/usr/bin/pacman`): the change goes into a layer
  (systemd-sysext, merged over `/usr` at once; `/etc` stays live), live with no restart. The package database is per slot,
  where pacman.conf's `DBPath` puts it (`/usr/lib/pacman` on an installed slot), so a layer carries its own database
  entries. The next `wol update apply` folds the layers into the new slot (`wol layers fold`, named in the slot's
  `/etc/wolos/folded-layers`); the live layers stay while the old slot runs, and `wol layers clear` removes them after the
  new slot is blessed (a good boot, `boot-ok`). Kernel, initramfs and systemd packages and `-Syu` (also `-Syu <pkg>`) go to
  the pending slot instead (taking effect at the next restart; a second change stacks on it); queries pass through.
  A change that fails before pacman commits leaves nothing; one that fails after pacman committed keeps its layer, which
  merges at the next restart.
- Developer mode: `wol seal off` (root) makes the running system writable and stops the boot check until the next update.
- `wol base verify` checks every file in `/usr/share/wolos/base.manifest` (sha256) and lists changed or missing ones.

## Malware check (ADR 0024, 0027)
A layer of protection against known malware. Code: `crates/xguard`, `wol_core::malware`. Runs on this computer only: no file, hash or verdict leaves it, and sample submission does not exist.

Threat model:
- Covers: a known bad file arriving as a download, on a removable disk, as an AppImage, `.wol`, Windows program, or a program marked executable under `/home`, `/tmp` and removable disks, and whatever wopkg is about to install.
- Does not cover: new or altered malware with no signature, and an attacker already running on the computer, who can stop nothing here without root but can also do harm without ever opening a new file.
- Ways past the first-use check, known and accepted: code run by an interpreter (`python x.py`, `wine x.exe` from a terminal; double-click goes through the check), `dlopen` and `mmap` of a file that was never executed, code in `memfd`, and packed Windows programs (no unpacker runs).
- The signature database is data from public lists. Formats read in part: hashes, section hashes and plain patterns are read; bytecode, targets that need normalised text, HTML or mail, alternatives, negations and regular expressions are left out and counted at build time. See the ADR for the numbers.

How it works:
- `wolos-xguard.service`: holds `FAN_OPEN_EXEC_PERM` on the mounts in scope and `FAN_OPEN_PERM` on tagged files. Files outside scope, and the sealed root, cost nothing.
- Tag: `user.wolos.quarantine`, set when a file lands in a Downloads folder, or `user.xdg.origin.url`, set by browsers, and by curl and wget through the shipped `.curlrc` and `.wgetrc`.
- Verdict: a clean file gets an evictable kernel ignore mark, removed by the kernel when the file changes. The verdict store (`/var/lib/wolos/xguard/verdicts`) keeps file identity and sha256, or the fs-verity digest when there is one, so a restart reads nothing again.
- Fails open, except for downloads: the guard dying, or a check it cannot finish, lets the file run. A stopped file needs a signature match. The one exception (audit S4): when the deep check cannot run at all on a tagged download (its worker ended or the rule sets did not load), the file is refused for now with the answer `unchecked`; that is not remembered, so the next open asks again.
- Size: a file up to 64 MiB is read whole and may get the deep check; a larger one, up to 1 GiB, is scanned in pieces (hashes and patterns, streamed with an overlap so a pattern across two pieces is still seen), and the deep worker reads it in pieces too, with its whole-file look left to the background pass. Above 1 GiB (audit S4-5): a file that is neither a tagged download nor asked about with `wol xguard check` runs at once and the background pass looks later; a tagged download or an asked file gets the streamed pass over its first 1 GiB, is refused if a pattern is found there, and otherwise is refused for now with `unchecked` (not remembered, no ignore mark) until the background pass clears it or the person chooses Open Anyway (`crates/xguard/src/guard.rs`, `check_large`).
- The guard's socket (`/run/wolos/xguard.sock`, 0666) is served on its own thread, never on the thread that answers the kernel (audit S4-3): at most 64 requests at once, 8 per account, each with 2 s to send its file; a caller over its share hears `unchecked` at once. Questions about the same file share one scan. `LimitNOFILE` in the unit covers the queue (`crates/xguard/tests/units.rs`).
- Engines: the mapped database in the guard; rule sets (YARA-X) only in a short-lived worker with no privileges, for tagged files, removable disks, Windows programs, AppImages and `.wol`; and, after the person opts in, ClamAV for the same files.
- Full signatures, off by default: a switch in Privacy & Security enables `wolos-clam-update.timer`. freshclam fetches the data to this computer; it is never in the image. `wol-xguard-clam` is the only program that links libclamav; it starts on demand from `wolos-clam.socket`, takes 1 GB while loaded, and exits after 120 idle seconds. Off deletes the data.
- `wol-analyze`, from Files' right-click (`desktop/nautilus/wolos-analyze.c`), runs `wol-xguard scan --clamav`: without an answer from ClamAV the result is Incomplete, never clean. With the signatures off it checks nothing and offers Settings; only the switch there turns them on.
- The guard's unit must keep the host's mount namespace: no `ProtectSystem`, `PrivateTmp`, `PrivateNetwork` or the like, or its kernel marks land on copies and it stops nothing.
- Background pass: `wolos-xguard-sweep.timer`, on mains power and while everyone is away and no game runs, at idle priority. It stops at the first sign of activity.
- Updates: `wolos-xguard-update.timer` downloads a signed bundle; `wol-xguard rules install` refuses anything unsigned, older or altered, then the guard restarts so files are looked at again.
- People see nothing unless a file is stopped: then one notification, and a row in Settings > Privacy & Security with Move to Trash and a deliberate Open Anyway. Each account's list is readable by that account only. `wol security`, `wol security scan <path>`, `wol security allow <path>`.

For installers (wopkg and the like): `libxguard.so.1` (`include/xguard.h`) or `wol-xguard scan --json [--rules DIR] PATH`, exit 0 clean, 1 found, 2 incomplete, 3 could not run. Archives, AppImages and `.wol` are read in memory, never unpacked to disk. The report gives `verdict`, `findings`, `skipped` and `signatures_age_days`, so a caller can treat old signatures as it likes.

## Error reports (off by default)
- `/usr/lib/wolos/report` (`wolos-report.timer`, hourly) sends only when `/etc/wolos/reports.conf` has `send=1`.
- Each new failed unit or coredump goes once to `https://wolos.squareweb.app/report`:
  `kind`, WolOS version, kernel, `detail` (unit name, or program and signal). No logs, user or host names.
- Turn on: `echo send=1 | sudo tee /etc/wolos/reports.conf`. A Settings > Privacy switch comes next.

## Disk unlock (TPM disks)
- Order at start: the security chip (TPM2), then the account's own password, then the 64-character
  recovery key. systemd-cryptsetup asks once; the password and the recovery key both answer it.
- The password slot is the installer account's password (`install-run`, `wol encrypt enroll ... PASSFILE`),
  never a shared one; the account is named in `/var/lib/wolos/disk-unlock-user`.
- After a start the chip refused (Secure Boot turned off, new firmware: PCR 7 changed),
  `wolos-tpm-reseal.service` (`wol encrypt reseal`) binds the chip again to the PCRs measured now. The
  secret comes from systemd-cryptsetup's password cache: what was typed stays ~150 s in root's
  user keyring as the `user` key `cryptsetup` (`password-cache=yes`, the default). It goes to
  systemd-cryptenroll as `$PASSWORD`, then the key is invalidated. Nothing typed: nothing happens.
- Changing the password in Settings (wol-admind `user-password`, the current password every time)
  also replaces the disk slot (`wol encrypt password USER`, unlocked by the chip). `passwd` in a
  terminal does not: the disk keeps the previous password (the chip and the recovery key still work).

## Encrypted homes (off by default)
- `home_crypt`: a systemd-homed LUKS home (`homectl create --identity=-`, record and password on stdin).
- wol-login activates it with the password it has just checked (`PASSWORD` in homectl's
  environment) and deactivates it after logout unless something still uses it.
- Off until proven on a real install. A new password needs `home_crypt::change_key` (`homectl passwd`).

## App permissions

| Permission | Enforced by | Why |
|---|---|---|
| Screen capture, cursor position, input capture | The compositor (`hypr/permissions.conf`, first match wins) | Only the portal and WolOS's own tools (`security::SCREENCOPY_SYSTEM`) copy the screen directly. Apps ask through the portal, which asks which screen to share. The rules are static (`security::hypr_permissions`): no grant adds a program to them. |
| X11 apps (Xwayland) | Xwayland's access list (`hyprland.conf`: `xhost +si:localuser`, `-local:`, `-`) | Xwayland starts without an auth cookie. Only the signed-in account may connect to its X server; other accounts, system users and root may not. Within one account, every X11 app can read the keys and the screen of every other X11 window: X11 apps share one trust domain per account. Wayland apps are not affected. |
| Compositor plugins | The compositor | Loaded only from `/usr/lib/hyprland/plugins` (`security::PLUGIN_DIR`). |
| Camera | advisory | Every account needs group video for the GPU, and `/dev/video*` belongs to it. |
| Microphone | advisory | PipeWire gives the microphone to every app of the account. |
| Location | advisory | WolOS ships no location service. |
| Files outside home, virtual keyboards, clipboard history, window list | advisory | Apps run as the account, with no per-app sandbox. |

Advisory means a grant is recorded and shown to helpers that ask. An app that ignores it is not stopped. Settings says which kind each permission is (`security::enforcement`).

Screen recording is not a grant in the consent database (`/etc/wolos/consent.json`, `consent::SCREEN_CAPTURE`): nothing would read it, since the compositor rules are static and every app asks through the portal each time. The administrator `consent` action refuses it (`consent::refusal`), an old `screen-capture` entry is ignored when the file is read and dropped the next time it is written, and Settings > Privacy & Security shows one row saying apps ask each time, not a switch per app.

## What runs as root
| Program | Does | Takes input from |
|---|---|---|
| wold | starts and restarts system jobs; serves the control/portal socket, the QoS socket and the Memory Saver socket (all 0666, identity from the kernel, ADR 0033 hello first); runs Allow-sheet package changes (`wol pkg`, system update, Undo Update) after the person said yes on screen, each stopped after 30 minutes. Runs with 4096 descriptors (`LimitNOFILE=4096:524288`); every job it starts goes back to soft 1024, so the session inherits nothing. A peer whose executable changed since it connected may not publish or subscribe either (audit S4-26) | its job files; the three sockets: peer uid, pid and executable from the kernel; QoS acts only on processes of the peer's own account (audit S4-1) |
| wol-admind | runs a fixed allow-list of actions (`AdminAction`), each validated, audited, with a password unless it is an everyday change by the account at the computer | a Unix socket (ADR 0033 hello first, each connection on its own thread with a 2 s request deadline, a password check bounded to 10 s; audit S4-7), peer uid from the kernel |
| wol-login | checks passwords (PAM), starts sessions, opens encrypted homes | the login window's socket |
| wol-xguard-clam (runs as `_xguard`, not root) | ClamAV, on demand, no network | open files over a Unix socket |
| wol-xguard guard | answers first-use checks, reads the files it is asked about | kernel events, file content, a Unix socket with file descriptors, peer uid from the kernel |
| systemd units and timers | `wol power apply`, `update-check` | none |

No Capsicum or jails yet. Each daemon runs argv lists it builds itself (never a shell with user text). Each parses only its own small wire format.

## Needs hardware
- pf at boot (pf.ko is in the image).
- Compositor permission prompts on a real GPU (only `--verify-config` checked).
- freebsd-update against update.FreeBSD.org.
- Encrypted homes on a real install.
