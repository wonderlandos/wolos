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
- `wol base verify` checks every file in `/usr/share/wolos/base.manifest` (sha256) and lists changed or missing ones.

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
| systemd units and timers | `wol power apply`, `update-check` | none |

No Capsicum or jails yet. Each daemon runs argv lists it builds itself (never a shell with user text). Each parses only its own small wire format.

## Needs hardware
- pf at boot (pf.ko is in the image).
- Hyprland permission prompts on a real GPU (only `--verify-config` checked).
- freebsd-update against update.FreeBSD.org.
- Encrypted homes on a real install.
