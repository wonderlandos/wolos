# Security (ADR 0040)

On from the first boot (ADR 0040). Code: `wol_core::security` (defaults, status), `wol_core::update::check`, `wol_core::home_crypt`.
`wol security [--json]` shows the state; `wol firewall on|off|status` changes the firewall through wol-admind.

## Defaults
- Firewall: nftables, `/etc/wolos/nftables.conf` (loaded by a `nftables.service` drop-in), `nftables.service` enabled.
  - Inbound dropped except replies, ping, DHCP, IPv6 neighbour discovery and mDNS (to the multicast groups only). Outbound allowed.
  - The switch is Settings > Network > Firewall. The unit is a oneshot that stays active (`RemainAfterExit=yes`) and takes the table out on stop, so Off unloads every rule; `wol security` calls the firewall running when the kernel has the table.
  - A setting that opens a port writes a file in `/etc/wolos/nftables.d/` (Remote Login: `ssh.conf`, port 22, 10 new connections a minute) and loads the ruleset again with `nft -f /etc/wolos/nftables.conf` when the firewall is on. The live medium opens 22 with `live-ssh.conf` there.
- No remote login: `sshd.service` stays disabled until Sharing turns it on.
- The log stays local: journald only, no network syslog daemon.
- Kernel hardening (`/usr/lib/sysctl.d/90-wolos.conf`, one reason per line): `kernel.dmesg_restrict=1`
  (the kernel log is for administrators), `kernel.kptr_restrict=2` (kernel addresses hidden),
  `kernel.yama.ptrace_scope=1` (own children only), `kernel.sysrq=0` (no debugger key chord),
  `fs.protected_hardlinks/symlinks=1`, `fs.protected_fifos/regular=1` (no link or FIFO tricks in
  shared sticky folders such as /tmp), `fs.suid_dumpable=0` (no core files from setuid programs),
  `kernel.unprivileged_bpf_disabled=1` (only administrators load BPF), `net.core.bpf_jit_harden=1`
  (constant blinding for ordinary accounts' BPF), `net.ipv4.tcp_syncookies=1` (half-open floods),
  `kernel.kexec_load_disabled=1` (no kernel swap without a firmware restart),
  `kernel.io_uring_disabled=1` (only administrators create io_uring instances; a hardening default,
  revisit when a shipped app needs it). The same file raises
  `vm.max_map_count` for games. `wol security` reads them from /proc/sys; **CI** the controller test sysctl_docs
  fails when this line names a setting no file sets.

## Updates (ADR 0016, 0017)
- A systemd timer runs `/usr/lib/wolos/update-check` hourly; it checks at most every 20 h:
  `pacman -Sy` (pinned snapshot) and `pacman -Qu` against a temporary copy of the package database
  (`local` linked, `sync` copied: the real one is on the read-only seal, ADR 0016), `arch-audit -q`,
  `pacman -Qq` (kernels), and the signed release manifest (`release.json` + `.sig`, verified with
  the package keys when read) from `https://wolos.squareweb.app/latest/`, or from the latest GitHub
  release of wolos-packages when the CDN answers 404 or cannot be reached.
- Findings go to `/var/lib/wolos/updates`, readable by every account. Never installs.
  `wol update check` and Settings > Software Update > Check Now ask wol-admind (`pkg-refresh`) to
  run the check now, then read the same files; a stamp older than the request means it could not
  refresh, and an empty list is then never shown as "up to date".
- Release manifest: version, Arch snapshot date, `[wolos]` revision, linux-wolos version, image URL and sha256.
- Computers still on 0.12.0 (QA-UPG-1). Their daily check (wolos-system 0.12.0-2, the script of tag v0.12.0)
  runs `checkupdates` first; it needs fakeroot, which the image does not ship, so the check stops there every
  time: no stamp, no package list, no release manifest. Nothing a server publishes reaches that check. What does
  reach 0.12.0: Settings > Software Update > Check Now (`pkg-refresh`, then `pacman -Qu`), which lists newer
  packages of the `[wolos]` repo its mirror list names (release `wolos-2026-10-31` of wolos-packages). So a release
  is found by 0.12.0 only when (1) a newer `wolos-system`, with today's update-check, is in that repo (then Check Now
  shows it, and after it is installed the daily check works), and (2) the latest GitHub release of wolos-packages,
  the only address 0.12.0 reads, carries `release.json` and `release.json.sig` that 0.12.0 accepts: the strings
  `version` (not empty), `snapshot` (`YYYY/MM/DD`), `wolos_repo`, `kernel`, `image`, `sha256` (64 hex digits), any
  other field ignored, signed (raw Ed25519, 64 bytes) by `wolos-2026.pub` or `wolos-2026b.pub`. The release tooling
  writes that form (wolos-iso `release/release-manifest.sh`, wolos-build `release --iso` marks a stable release
  the latest); **CI** the controller tests `the_0120_client_reads_the_published_release` and
  `check_runs_without_checkupdates_or_fakeroot` fail when either side changes.
- Update Now (wol-admind `pkg-upgrade`) = `pacman -Syu` inside the release (only `[wolos]` moves), made in the
  other root slot by the same builder as the pacman wrapper below (`wol_core::update::pkg::slot_change`). The running
  slot is the restore point Software Update promises: wol-admind logs its name before the first step, and no step
  touches it. The update takes effect at the next restart; a failed step deletes the half-made slot. A one-copy install
  (ADR 0023, `/etc/wolos/single-copy`) has no restore point: Update Now updates the running system in place, and
  Software Update says so before the button is used. A system with neither is refused and nothing runs. A failed
  Update Now answers `failed <sentence>` (one line, at most 200 bytes; older clients read it as `failed`) and Software
  Update shows that sentence. Through wopkg it travels the whole way: the shell's administrator sheet hands it to wold,
  wold answers wopkg with it word for word, wopkg prints it as one stderr line `wopkg: system: <sentence>` (exit code
  unchanged), and Settings reads that line.
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
  merges at the next restart. `wol layers list` (no administrator, `--json` too) shows the live layers: each one's packages
  added and removed, size, date and whether it is still live only (folded at the next update) or already part of this
  system (removed at the next restart).
- Developer mode: `wol seal off` (root) makes the running system writable and stops the boot check until the next update.
- `wol base verify` checks every file in `/usr/share/wolos/base.manifest` (sha256) and lists changed or missing ones: the first 20 on a terminal, and in `--json` up to 500 in `findings` with the whole count in `findings_total` (`changed`, `missing`, `truncated`; QA-LIVE3-25).

## Malware check (ADR 0024, 0027)
A layer of protection against known malware. Code: `crates/xguard`, `wol_core::malware`. Runs on this computer only: no file, hash or verdict leaves it, and sample submission does not exist.

Threat model:
- Covers: a known bad file arriving as a download, on a removable disk, as an AppImage, `.wol`, Windows program, or a program marked executable under `/home`, `/tmp` and removable disks, and whatever wopkg is about to install.
- Does not cover: new or altered malware with no signature, and an attacker already running on the computer, who can stop nothing here without root but can also do harm without ever opening a new file.
- Ways past the first-use check, known and accepted: code run by an interpreter (`python x.py`, `ruby x.rb`, `sh x.sh`, `wol script run x.rb`, `wine x.exe` from a terminal; double-click goes through the check; the interpreter is checked once, not each script it reads; ADR 0041 adds `ruby` to the system and widens this gap by one interpreter), `dlopen` and `mmap` of a file that was never executed, code in `memfd`, and packed Windows programs (no unpacker runs).
- The signature database is data from public lists. Formats read in part: hashes, section hashes and plain patterns are read; bytecode, targets that need normalised text, HTML or mail, alternatives, negations and regular expressions are left out and counted at build time. See the ADR for the numbers.

How it works:
- `wolos-xguard.service`: holds `FAN_OPEN_EXEC_PERM` on the mounts in scope and `FAN_OPEN_PERM` on tagged files. Files outside scope, and the sealed root, cost nothing.
- Tag: `user.wolos.quarantine`, set when a file lands in a Downloads folder, or `user.xdg.origin.url`, set by browsers, and by curl and wget through the shipped `.curlrc` and `.wgetrc`.
- Verdict: a clean file gets an evictable kernel ignore mark, removed by the kernel when the file changes. The verdict store (`/var/lib/wolos/xguard/guard/verdicts`, 0600) keeps file identity and sha256, or the fs-verity digest when there is one, so a restart reads nothing again.
- Fails open, except for downloads: the guard dying, or a check it cannot finish, lets the file run. A stopped file needs a signature match. The one exception (audit S4): when the deep check cannot run at all on a tagged download (its worker ended or the rule sets did not load), the file is refused for now with the answer `unchecked`; that is not remembered, so the next open asks again.
- Size: a file up to 64 MiB is read whole and may get the deep check; a larger one, up to 1 GiB, is scanned in pieces (hashes and patterns, streamed with an overlap so a pattern across two pieces is still seen), and the deep worker reads it in pieces too, with its whole-file look left to the background pass. Above 1 GiB (audit S4-5): a file that is neither a tagged download nor asked about through the guard's socket (`wol-xguard check PATH`; `wol security scan PATH` runs `wol-xguard scan`, which checks the file itself and does not ask the guard) runs at once and the background pass looks later; a tagged download or an asked file gets the streamed pass over its first 1 GiB, is refused if a pattern is found there, and otherwise is refused for now with `unchecked` (not remembered, no ignore mark) until the background pass clears it or the person chooses Open Anyway (`crates/xguard/src/guard.rs`, `check_large`).
- The guard's socket (`/run/wolos/xguard.sock`, 0666) is served on its own thread, never on the thread that answers the kernel (audit S4-3): at most 64 requests at once, 8 per account, each with 2 s to send its file; a caller over its share hears `unchecked` at once. Questions about the same file share one scan. `LimitNOFILE` in the unit covers the queue (`crates/xguard/tests/units.rs`).
- Engines: the mapped database in the guard; rule sets (YARA-X) only in a short-lived worker with no privileges, for tagged files, removable disks, Windows programs, AppImages and `.wol`; and, after the person opts in, the full signature engine (`wol-xguard-clam`) for the same files.
- Full signatures, off by default: a switch in Privacy & Security enables `wolos-clam-update.timer`. freshclam fetches the data to this computer; it is never in the image. `wol-xguard-clam` is the only program that links libclamav; it starts on demand from `wolos-clam.socket`, takes 1 GB while loaded, and exits after 120 idle seconds. Off deletes the data.
- `wol-analyze`, from Files' right-click "Check for Malware" (`desktop/nautilus/wolos-analyze.c`), runs `wol-xguard scan --clamav`: without an answer from the full engine the result is Incomplete, never clean. With the signatures off it checks nothing and offers Settings; only the switch there turns them on.
- The guard's unit must keep the host's mount namespace: no `ProtectSystem`, `PrivateTmp`, `PrivateNetwork` or the like, or its kernel marks land on copies and it stops nothing.
- Background pass: `wolos-xguard-sweep.timer` (it looks at executables, tagged downloads and `.AppImage`, `.exe`, `.msi`, `.wol`, `.sh`, `.rb` and `.desktop` files: `xguard::sweep::in_scope`), on mains power and while everyone is away and no game runs, at idle priority. It stops at the first sign of activity.
- Updates: `wolos-xguard-update.timer` downloads a signed bundle; `wol-xguard rules install` refuses anything unsigned, older or altered, then the guard restarts so files are looked at again.
- People see nothing unless a file is stopped: then one notification, and a row in Settings > Privacy & Security with Move to Trash and a deliberate Open Anyway. Each account's list is readable by that account only. `wol security`, `wol security scan <path>`, `wol security allow <path>`.

For installers (wopkg and the like): `libxguard.so.1` (`include/xguard.h`) or `wol-xguard scan --json [--rules DIR] PATH`, exit 0 clean, 1 found, 2 incomplete, 3 could not run. Archives, AppImages and `.wol` are read in memory, never unpacked to disk. The report gives `verdict`, `findings`, `skipped` and `signatures_age_days`, so a caller can treat old signatures as it likes.

## Error reports (off by default)
- `/usr/lib/wolos/report` (`wolos-report.timer`, hourly) sends only when `/etc/wolos/reports.conf` has `send=1`.
- Each new failed unit or coredump goes once to `https://cdn.wolos.org/report`, or to the same
  server under its own name `https://wolos.squareweb.app/report` when that cannot be reached
  (`wol_core::cdn::REPORT`):
  `kind`, WolOS version, kernel, `detail` (unit name, or program and signal). No logs, user or host names.
- Turn on: `wol admin reports on` (off: `wol admin reports off`); it writes `send=1|0` in that file and enables or disables the timer. A Settings > Privacy switch uses the same request.

## Disk unlock (TPM disks)
- Order at start: the security chip (TPM2), then the account's own password, then the 64-character
  recovery key. systemd-cryptsetup asks once; the password and the recovery key both answer it.
- The password slot is the installer account's password (`install-run`, `wol encrypt enroll ... PASSFILE`),
  never a shared one; the account is named in `/var/lib/wolos/disk-unlock-user`.
- After a start the chip refused (Secure Boot turned off, new firmware: PCR 7 changed),
  `wolos-tpm-reseal.service` (`wol encrypt reseal`) binds the chip again to the PCRs measured now. The
  secret comes from systemd-cryptsetup's password cache: what was typed stays ~150 s in root's
  user keyring as the `user` key `cryptsetup` (`password-cache=yes`, the default). It goes to
  systemd-cryptenroll as `$PASSWORD`, then the key is invalidated. Nothing typed (the chip opened the disk):
  nothing happens. A passphrase-only install (no `tpm2-device` in `rd.luks.options`) never asked for the chip:
  the service does nothing and exits 0, so there is no failed unit and no chip slot added behind the choice.
- Neither the first-boot bind (`wolos-tpm-bind.service`) nor the reseal runs on a start the firmware
  did not measure (`ConditionSecurity=measured-uki`: the boot image is not in PCR 11). Such a start
  (seen with a virtual TPM: no event log, PCRs 2-5 and 11 never extended) asks for the password or
  recovery key, since the chip cannot vouch for it; sealing to it would make the next good start ask
  again. The install key stays until a measured start binds (test `tpm_binding_waits_for_a_start_the_firmware_measured`).
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
| Screen capture, cursor position, input capture | The compositor (`hypr/permissions.lua`, `hl.permission` calls, first match wins) | Only the portal and WolOS's own tools (`security::SCREENCOPY_SYSTEM`, which includes the colour picker `hyprpicker` of Ctrl+Super+P) copy the screen directly. Apps ask through the portal, which asks which screen to share. The rules are static (`security::hypr_permissions`): no grant adds a program to them. |
| X11 apps (Xwayland) | Xwayland's access list (`hyprland.lua`: `xhost +si:localuser`, `-local:`, `-`) | Xwayland starts without an auth cookie. Only the signed-in account may connect to its X server; other accounts, system users and root may not. Within one account, every X11 app can read the keys and the screen of every other X11 window: X11 apps share one trust domain per account. Wayland apps are not affected. |
| Compositor plugins | The compositor | Loaded only from `/usr/lib/hyprland/plugins` (`security::PLUGIN_DIR`). |
| Camera | advisory | Every account needs group video for the GPU, and `/dev/video*` belongs to it. |
| Microphone | advisory | PipeWire gives the microphone to every app of the account. |
| Location | advisory | WolOS ships no location service. |
| Files outside home, virtual keyboards, clipboard history, window list | advisory | Apps run as the account, with no per-app sandbox. |

Advisory means a grant is recorded and shown to helpers that ask. An app that ignores it is not stopped. Settings says which kind each permission is (`security::enforcement`).

Screen recording is not a grant in the consent database (`/etc/wolos/consent.json`, `consent::SCREEN_CAPTURE`): nothing would read it, since the compositor rules are static and every app asks through the portal each time. The administrator `consent` action refuses it (`consent::refusal`), an old `screen-capture` entry is ignored when the file is read and dropped the next time it is written, and Settings > Privacy & Security shows one row saying apps ask each time, not a switch per app.

Bundled apps work on a fresh system through vendor defaults: a package of a WolOS app installs `/usr/share/wolos/consent.d/<app>.json` (`consent::DEFAULTS_DIR`, the same `{"grants":[...]}` shape; Gallery's is `wolos-crystal/apps/gallery/org.wolos.gallery.consent.json`, `trash` and `share`). Wold and Settings hold what the person has not switched off plus the person's own grants (`consent::effective`); a default is accepted only for a program directly in `/usr/bin` and only for the harmless verbs in `consent::DEFAULTABLE` (`trash`, `share`, `notifications`, `file-panels`, `app-preferences`, `search-provider`), anything else in the file is ignored. Switching a default off in Settings writes the pair to `"revoked"` in `/etc/wolos/consent.json` (`consent::set_over`) and switching it on removes it, so the choice survives updates and the package never touches the person's file. A consent file that is not valid JSON grants nothing, defaults included.

An app with no grant is refused: that is the default for an app WolOS has never seen (ADR 0040). So that the refusal's "Turn them on in System Settings > Privacy & Security" leads to a switch, wold notes each refused (app, capability) once in the account's own list, `/var/lib/wolos/consent-asked/<uid>.json` (`consent::ASKED_DIR`, `{"asked":[{"app","capability"}]}`, newest 64, owned by the account, mode 0600, read at most 64 KiB; `portal::Remembering`). Settings lists those apps under their permission with the switch off; turning it on is the administrator `consent` action as for any grant. The list grants nothing: the consent database alone decides.

## Who may answer an ask (the shower)
Asks that need a person (`SessionAsk`) go to the account's shell over `subscribe account` and come back as `session.answer`. wold knows the shell by its executable, `/usr/bin/wol-shell` (`portal::SHOWERS`), and any process of the account can run that file (audit P6-1). So a peer that is a shower by path is also refused when its pinned `/proc/<pid>` shows foreign code (`PeerIdentity::injected`, `crates/api/src/peer.rs`): `LD_*`, `GTK_MODULES` and the other plug-in variables in its environment (or no environment), executable code mapped from outside `/usr` and `/lib`, a tracer attached, a mount namespace other than wold's, or anything unreadable. Other apps keep their identity for grants: a game launcher with `LD_LIBRARY_PATH` is still an app. Known and accepted: the check is defence in depth; a process that puts code into the real shell after it started is not caught (the shell should be non-dumpable), and a parent check is not possible because the compositor double-forks its `exec-once` programs. The real fix, a token only the session's shell holds, is ADR 0037 (proposed). Tests: `peer::tests::a_shower_binary_under_ld_preload_is_not_a_shower`, `server::tests::a_shower_binary_under_ld_preload_cannot_subscribe_to_the_account`.

Also: the sweep (`wolos-xguard-sweep.service`, user `_xguard`) reads every home and writes none: `CAP_DAC_READ_SEARCH` to read, `ProtectHome=read-only`, `ReadWritePaths=/var/lib/wolos/xguard/sweep`, its own folder (audit P6-2). It keeps `CAP_CHOWN` for one job: each account's findings list, `sweep-<uid>.json`, is owned by that account and 0600 before it appears, so the shell reads it as that account and no other account can (audit R7-1; without it the write fails and a finding is never shown). The capability reaches that folder only: homes, the system, `/sys` and cgroups are read-only to the sweep, `/tmp` and `/dev` are private to the run (tests `the_sweep_reads_homes_and_cannot_change_them`, `a_sweep_hit_reaches_the_account_that_owns_the_file`). It remembers finished files in its own `sweep-verdicts`, never in the guard's `verdicts` (test `the_pass_leaves_the_guards_verdicts_alone`). Folders (audit F10, `files/usr/lib/tmpfiles.d/wolos.conf`): `/var/lib/wolos/xguard` is root:_xguard 1775, so the full database's updater can write there but nothing `_xguard` runs can move or replace root's entries; `guard/` (root:root 0755) holds the guard's verdicts (0600), status and each account's `findings-<uid>.json`; `sweep/` (_xguard) holds the pass's memory and `sweep-<uid>.json`. The guard refuses a state folder that is a link, not its own or writable by others, opens its cache without following links and only when it owns it, and reads a findings list only when the account it names owns it (tests `the_guards_state_is_out_of_xguards_reach`, `the_guard_keeps_its_state_in_a_folder_only_it_may_write`, `a_link_in_place_of_the_cache_is_refused`). Files from before the split are removed at boot, not adopted. The signatures the guard loads live there too: `guard/rules` (signed bundles, `wol-xguard rules install` as root) and `guard/local` (the full database's lean build). The full-database download (`clam-update`, as `_xguard`) only fetches whole signed files into `clamav/`; root then builds `guard/local/full.xgdb` (`ExecStartPost=+/usr/bin/wol-xguard rules full`): each file is read without following a link into a folder of root's own, its publisher's signature checked there, and a file older than the one in use (a replay of a once-signed file) is refused, as is anything else, with the previous build kept. Each outcome goes to root's `guard/rules-status.json` (`ok`, `reason`, `time`, the versions in use); Settings reads it and says "Signature update refused: <reason>" (`malware::refusal`, `copy::malware_full`). The guard loads `rules/` and `local/full.xgdb` only when they and their folder are no link, root's, and writable by nobody else (`xguard::trusted`; tests in `crates/xguard/tests/full_rules.rs`). A program that runs `wol` can raise an install sheet; the sheet says "A program using wol wants to install “name”", never "wol" as the requester (P6-4). A package file copy (`wol` install of a local file) is refused when it would leave the staging disk under a reserve of 10 % (at least 1 GiB) free (P6-5; `packages::staging_reserve`). `wolos-build` drops a `.sig` that arrives with a package and signs it with the release key; a signature already beside a package is kept only if `gpg --verify` accepts it (P6-3).

## What runs as root
| Program | Does | Takes input from |
|---|---|---|
| wold | starts and restarts system jobs; serves the control/portal socket, the QoS socket and the Memory Saver socket (all 0666, identity from the kernel, ADR 0033 hello first); runs Allow-sheet package changes (`wol pkg`, system update, Undo Update) after the person said yes on screen, each stopped after 30 minutes. Runs with 4096 descriptors (`LimitNOFILE=4096:524288`); every job it starts goes back to soft 1024, so the session inherits nothing. A peer whose executable changed since it connected may not publish or subscribe either (audit S4-26) | its job files; the three sockets: peer uid, pid and executable from the kernel; QoS acts only on processes of the peer's own account (audit S4-1) |
| wol-admind | runs a fixed allow-list of actions (`AdminAction`), each validated, audited, with a password unless it is an everyday change by the account at the computer | a Unix socket (ADR 0033 hello first, each connection on its own thread with a 2 s request deadline, a password check bounded to 10 s; audit S4-7), peer uid from the kernel |
| wol-login | checks passwords (PAM), starts sessions, opens encrypted homes | the login window's socket |
| wol-xguard-clam (runs as `_xguard`, not root) | the full signature engine, on demand, no network | open files over a Unix socket |
| wol-xguard guard | answers first-use checks, reads the files it is asked about | kernel events, file content, a Unix socket with file descriptors, peer uid from the kernel |
| systemd units and timers | `wol power apply`, `update-check` | none |

No namespace sandbox around the daemons yet beyond their systemd hardening. Each daemon runs argv lists it builds itself (never a shell with user text). Each parses only its own small wire format.

## Needs hardware
- nftables firewall at boot on a real install (`nftables.service`).
- Compositor permission prompts on a real GPU (only `--verify-config` checked).
- `wol update apply` against the real mirror on a real install (the slot switch is tested in a VM).
- Encrypted homes on a real install.
