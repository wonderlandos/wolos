# Security

On from the first boot (ADR 0015). Code: `wol_core::security` (defaults, status), `wol_core::update::check`, `wol_core::home_crypt`.
`wol security [--json]` shows the state; `wol firewall on|off|status` changes the firewall through wol-admind.

## Defaults
- Firewall (pf, `/usr/local/share/wolos/pf.conf`):
  - Inbound is blocked except replies, ping, DHCP, IPv6 neighbour discovery and mDNS. Outbound is allowed.
  - `pf_enable=YES` on the live and the installed system.
- No remote login: sshd stays off until Sharing turns it on.
- The system log does not listen on the network (`syslogd -ss`).
- sendmail is off, and `/tmp` is cleared at boot.
- Kernel hardening (`/etc/sysctl.conf.local`, one reason per line):
  - Accounts cannot see other accounts' processes (uids, gids).
  - Only root reads the kernel message buffer.
  - No hard links to other accounts' files.
  - Random PIDs.
  - ASLR is FreeBSD 15's default.
  - `unprivileged_proc_debug` stays on: apps debug their own processes, and a session finds its own lock screen.

## Updates
- cron runs `/usr/local/libexec/wolos/update-check` hourly. The script checks at most every 20 h:
  - `pkg update`, `pkg version`;
  - `pkg audit -F`;
  - `freebsd-update fetch`.
- It only downloads. Findings go to `/var/db/wolos/updates`, which every account can read.
- The shell shows "Updates Available" once per check. Security is named only when an update replaces a package with a known vulnerability.
- Update Now runs wol-admind `pkg-upgrade`: a restore point (boot environment), then fetched FreeBSD fixes, then packages.
- On a kernel other than GENERIC, freebsd-update is limited to `world`, so it never replaces WolKernel. Kernel fixes come as a new WolKernel.
- `wol update status` shows the findings.

## Encrypted homes (off by default)
- `home_crypt`: the home is its own dataset `zroot/home/<name>`.
  - Encrypted with AES-256-GCM under a key that ZFS derives from the login password.
  - `canmount=noauto`, so it stays locked at boot.
- wol-login opens the home with the password it has just checked, and locks it again after logout unless something still uses it.
- Passwords travel only on stdin.
- Off until it is proven on a real install. The Setup Assistant owns the switch.
- A new password needs `home_crypt::change_key`; `passwd(1)` does not re-key the home yet.

## App permissions

| Permission | Enforced by | Why |
|---|---|---|
| Screen capture, cursor position, input capture | Hyprland (`hypr/permissions.conf`, first match wins) | Only the portal and WolOS's own tools copy the screen directly. Apps ask through the portal. |
| Compositor plugins | Hyprland | Loaded only from `/usr/local/lib/hyprland/plugins`. |
| Camera | advisory | Every account needs group video for the GPU, and `/dev/video*` belongs to it. |
| Microphone | advisory | `/dev/dsp` plays and records on one device. |
| Location | advisory | FreeBSD has no location service. |
| Files outside home, virtual keyboards, clipboard history, window list | advisory | Apps run as the account, with no per-app sandbox. |

Advisory means a grant is recorded and shown to helpers that ask. An app that ignores it is not stopped. Settings says which kind each permission is (`security::enforcement`).

## What runs as root
| Program | Does | Takes input from |
|---|---|---|
| wold | starts and restarts system jobs | its job files |
| wol-admind | runs a fixed allow-list of actions (`AdminAction`), each validated, audited, with a password unless it is an everyday change by the account at the computer | a Unix socket, peer uid from the kernel |
| wol-login | checks passwords (PAM), starts sessions, opens encrypted homes | the login window's socket |
| devd, rc.d, cron | `wol power apply`, `wol power suspend`, `update-check` | none |

No Capsicum or jails yet. Each daemon runs argv lists it builds itself (never a shell with user text). Each parses only its own small wire format.

## Needs hardware
- pf at boot (pf.ko is in the image).
- Hyprland permission prompts on a real GPU (only `--verify-config` checked).
- freebsd-update against update.FreeBSD.org.
- Encrypted homes on a real install.
