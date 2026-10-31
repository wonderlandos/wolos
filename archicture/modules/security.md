# Security

On from the first boot (ADR 0015). Code: `wol_core::security` (defaults, status), `wol_core::update::check`, `wol_core::home_crypt`.
`wol security [--json]` shows the state; `wol firewall on|off|status` changes the firewall through wol-admind.

## Defaults
- Firewall: nftables, `/etc/wolos/nftables.conf` (loaded by a `nftables.service` drop-in), `nftables.service` enabled.
  - Inbound dropped except replies, ping, DHCP, IPv6 neighbour discovery and mDNS. Outbound allowed.
- No remote login: `sshd.service` stays disabled until Sharing turns it on.
- The log stays local: journald only, no network syslog daemon.
- Kernel hardening (`/etc/sysctl.d/90-wolos.conf`, one reason per line): `kernel.kptr_restrict=2`,
  `dmesg_restrict`, `yama.ptrace_scope=1` (own children only), `unprivileged_bpf_disabled`,
  `fs.protected_hardlinks/symlinks`, `net.ipv4.tcp_syncookies`. `wol security` reads them from /proc/sys.

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
  (systemd-sysext, merged over `/usr` at once; `/etc` and the package database stay live), live with no restart; the next
  `wol update apply` folds the layers into the new slot. Kernel, initramfs and systemd packages and `-Syu` go to the pending
  slot instead (taking effect at the next restart; a second change stacks on it); queries pass through. A failed change leaves nothing.
- Developer mode: `wol seal off` (root) makes the running system writable and stops the boot check until the next update.
- `wol base verify` checks every file in `/usr/share/wolos/base.manifest` (sha256) and lists changed or missing ones.

## Malware check (ADR 0024)
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
- Fails open: the guard dying, or any check it cannot finish, lets the file run. A stopped file needs a signature match; nothing else stops anything.
- Two engines: the mapped database in the guard; rule sets (YARA-X) only in a short-lived worker with no privileges, for tagged files, removable disks, Windows programs, AppImages and `.wol`.
- Background pass: `wolos-xguard-sweep.timer`, on mains power and while everyone is away and no game runs, at idle priority. It stops at the first sign of activity.
- Updates: `wolos-xguard-update.timer` downloads a signed bundle; `wol-xguard rules install` refuses anything unsigned, older or altered, then the guard restarts so files are looked at again.
- People see nothing unless a file is stopped: then one notification, and a row in Settings > Privacy & Security with Move to Trash and a deliberate Open Anyway. Each account's list is readable by that account only. `wol security`, `wol security scan <path>`, `wol security allow <path>`.

For installers (wopkg and the like): `libxguard.so.1` (`include/xguard.h`) or `wol-xguard scan --json [--rules DIR] PATH`, exit 0 clean, 1 found, 2 incomplete, 3 could not run. Archives, AppImages and `.wol` are read in memory, never unpacked to disk. The report gives `verdict`, `findings`, `skipped` and `signatures_age_days`, so a caller can treat old signatures as it likes.

## Error reports (off by default)
- `/usr/lib/wolos/report` (`wolos-report.timer`, hourly) sends only when `/etc/wolos/reports.conf` has `send=1`.
- Each new failed unit or coredump goes once to `https://wolos.squareweb.app/report`:
  `kind`, WolOS version, kernel, `detail` (unit name, or program and signal). No logs, user or host names.
- Turn on: `echo send=1 | sudo tee /etc/wolos/reports.conf`. A Settings > Privacy switch comes next.

## Encrypted homes (off by default)
- `home_crypt`: a systemd-homed LUKS home (`homectl create --identity=-`, record and password on stdin).
- wol-login activates it with the password it has just checked (`PASSWORD` in homectl's
  environment) and deactivates it after logout unless something still uses it.
- Off until proven on a real install. A new password needs `home_crypt::change_key` (`homectl passwd`).

## App permissions

| Permission | Enforced by | Why |
|---|---|---|
| Screen capture, cursor position, input capture | Hyprland (`hypr/permissions.conf`, first match wins) | Only the portal and WolOS's own tools copy the screen directly. Apps ask through the portal. |
| Compositor plugins | Hyprland | Loaded only from `/usr/local/lib/hyprland/plugins`. |
| Camera | advisory | Every account needs group video for the GPU, and `/dev/video*` belongs to it. |
| Microphone | advisory | PipeWire gives the microphone to every app of the account. |
| Location | advisory | WolOS ships no location service. |
| Files outside home, virtual keyboards, clipboard history, window list | advisory | Apps run as the account, with no per-app sandbox. |

Advisory means a grant is recorded and shown to helpers that ask. An app that ignores it is not stopped. Settings says which kind each permission is (`security::enforcement`).

## What runs as root
| Program | Does | Takes input from |
|---|---|---|
| wold | starts and restarts system jobs | its job files |
| wol-admind | runs a fixed allow-list of actions (`AdminAction`), each validated, audited, with a password unless it is an everyday change by the account at the computer | a Unix socket, peer uid from the kernel |
| wol-login | checks passwords (PAM), starts sessions, opens encrypted homes | the login window's socket |
| wol-xguard guard | answers first-use checks, reads the files it is asked about | kernel events, file content, a Unix socket with file descriptors, peer uid from the kernel |
| systemd units and timers | `wol power apply`, `update-check` | none |

No Capsicum or jails yet. Each daemon runs argv lists it builds itself (never a shell with user text). Each parses only its own small wire format.

## Needs hardware
- pf at boot (pf.ko is in the image).
- Hyprland permission prompts on a real GPU (only `--verify-config` checked).
- freebsd-update against update.FreeBSD.org.
- Encrypted homes on a real install.
