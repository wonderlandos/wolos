# The post-change audit (second pass)

**Concept.** After a change lands in an area and its tests are green, one person who did not write
it tries to break it on a real machine, then writes a numbered report. The area is done when the
report's high and medium findings are fixed with tests (`docs/STANDARDS.md` §8).

**Why.** Tests prove what the author thought of. The bugs that become CVEs and the bugs that ruin an
install are the ones nobody thought of: the slow peer, the full disk, the second instance, the
reboot halfway. Finding them costs an hour on a box; finding them after a release costs a release.

**How.** Take this page as your brief. Fill in the component section that applies, run the general
list on everything, report in the format at the end.

## Getting a box

- The forge (`wolos-build/README.md` "Forge", `wolos-build/BUILD.md` "Forge"): `aws ec2
  start-instances --instance-ids <forge id from the README>`, `ssh arch@<public IP>`, then
  `wolos-build cache pull`. It stops itself after 30 minutes idle; `touch /run/wolos-forge.hold`
  while you work. Use your own directory under `/scratch/work/<you>/`; never touch another worker's
  pacman cache (two pacmans on one cache corrupt packages).
- Rust suites: `cd wolos && cargo test --workspace --locked`; one area:
  `cargo test -p controller -- install`. Clippy as CI runs it: see `.github/workflows/ci.yml`.
- An image to break: `wolos-iso/tests/run.sh` (live desktop in QEMU), `tests/test-install.sh`
  (unattended install with a bystander disk), `tests/run-installed.sh` (boot the result),
  `tests/test-secureboot.sh`. VirtualBox on the Windows machine for a second opinion.
- Never request an AWS quota increase and never build packages on a GitHub runner.

## General list (every area)

1. Read the diff first, then the tests; write down what the tests do **not** cover before you run anything.
2. Wrong input at every edge: empty, one byte, maximum, maximum plus one, `..`, absolute path,
   NUL, invalid UTF-8, a control character, a negative number, a number as a string.
3. Slow or missing peer: stop the daemon the change talks to (`systemctl stop wold`), pause it
   (`kill -STOP`), fill its socket buffer, answer with garbage, answer late.
4. Resources: full disk (`fallocate`), full tmpfs, no network, 1 GB of RAM, an ESP with 1 MB free.
5. Two at once: run the command twice in parallel; start a second shell or a second installer.
6. Interrupt: `kill -9` in the middle, reboot in the middle, unplug the disk, close the lid.
7. Privilege: run as the other account, as root where a user was expected and the reverse; read the
   files it wrote (modes, owners, where they are).
8. Logs: every failure you provoked must leave one actionable line in `journalctl` and show the user
   one sentence (`shell::copy` rules), not raw errno.
9. Upgrade: apply the change to a system installed from the previous release, not only a fresh one.
10. Read what the standards ask (`docs/STANDARDS.md` §1-§7) and check the diff against each item
    that applies; note the misses even when nothing breaks.

## By component

### Shell IPC (`$XDG_RUNTIME_DIR/wol-shell.sock`, wold `/var/run/wold.sock`, Kit host `wol-kit.sock`)
- Wire: `crates/shell/src/protocol.rs`, `crates/shared/src/shell.rs`, `shared::portal`, `shared::events`.
- Send: a line over `MAX_LINE` (60 KiB), 10 000 lines without newline, a JSON number as `1e999`,
  nested arrays 10 000 deep, a valid call for a capability this executable has no grant for.
- Identity: call as a different uid; replace the executable while a connection is open; ask through a symlink.
- Session asks (clipboard, panels): wold waits a few seconds for the shell; kill the shell during one.
- Watch the GTK main thread: `WAYLAND_DEBUG=1`, frame times; anything over 8 ms while a peer stalls is a finding.
- Fuzz: `cargo +nightly fuzz run ipc_decode` for at least 10 minutes on the changed decoder (`fuzz/`).

### Installer and disks (`files/usr/lib/wolos/install-*`, `repart.d*`, `crates/controller/src/install.rs`, `wol_core::install`)
- Disks: one disk; two identical disks; the live medium as a USB stick; a disk with the previous
  WolOS on it; a 4 Kn disk; a disk with a mounted partition; a read-only disk; a disk that vanishes.
- Bystander: the other disk must come out byte-identical (`tests/test-install.sh` does this; repeat
  with the disk order swapped).
- Plans (ADR 0023): hostname of 64 bytes, a user name with a space, a password of one character,
  `WOLOS_*` env unset and set to nonsense, a keymap that does not exist.
- Interrupt after repart, after the copy, before `esp-sync`, then boot: what does the person see?
- Encryption: TPM absent, TPM present but PCRs changed, wrong password three times, recovery key path.

### Boot (`esp-sync`, `boot-ok`, `boot-setup`, `boot-keys`, `boot-enroll`, ADR 0025, ADR 0012)
- A/B: break slot A's kernel (`mv` the `.efi`), boot three times, confirm the fall-back and the
  try counter (`wolos-a+3.conf` -> `wolos-a.conf`); then fix it and confirm the return.
- ESP: 1 MB free, FAT with a damaged directory, two ESPs, no ESP.
- Secure Boot: on with our key enrolled, on with it absent (must say so, not hang), setup mode, a
  firmware that ignores `BootNext`.
- Signing keys: file modes, owner, where they go when the TPM refuses; nothing of a key in a log.
- Seamless boot: the splash never shows the stock compositor page, also on a crash (`CHANGELOG.md` 0.11.5).

### Audio (`wol-volume`, PipeWire, the shell's volume OSD, Bluetooth in `crates/core/src/bluetooth.rs`)
- No sink; the default sink removed while adjusting; a Bluetooth headset connecting during playback;
  `wpctl` missing; 50 key presses in a second (OSD must not queue).
- Output switch while a call is open; sleep and wake with the headset still paired.

### Network (`update-check`, `update-download`, `wol mirror gate`, `crates/core/src/wifi.rs`, `netinfo.rs`, `hardware/online.rs`)
- Captive portal (DNS answers, HTTP lies), IPv6 only, a mirror that answers 200 with an HTML page, a
  mirror that drips one byte a second (the read timeout must fire), a redirect loop, a 5 GB body.
- Signatures: a package with a wrong signature, an old database replayed (the `revision` must refuse to go backwards).
- Wi-Fi: a network name with an emoji and one with a quote, a password of 63 characters, the AP
  leaving during the join (`JOIN_WAIT`, `crates/core/src/controls.rs`).
- Nothing leaves the machine the person did not ask for: `tcpdump` on a fresh install for 10 minutes.

### Packages and the recipes (`wolos-pkgbuilds`, `wopkg`, ADR 0021, ADR 0024)
- A `.wol` with a path containing `..`, with 100 000 files, with one 10 GB file, with a wrong
  signature, with a valid signature from a revoked key.
- `tools/lint.sh` and `checks/` pass; a recipe that reads `../` is refused.
- Install, remove, reinstall, interrupt with `kill -9` after extraction; the database must be whole.

### Privilege boundaries (`wold`, `wol-admind`, root helpers, plugins, MCP; `docs/STANDARDS.md` §7.1)
- Identity: send a request, then `exec` another program in the same process before the reply; the
  daemon must still act on the identity captured at accept (`PeerIdentity`).
- Quotas: a client that sends one line per second forever, one that never reads its replies, 10 000
  connections at once; each must be dropped without stalling other clients (`BoundedLineReader`).
- Fresh password: every request kind in the fresh-password table asked once with a cached session;
  each must prompt again.
- Commands: every `Command` the change adds in a root process goes through `privileged_command`;
  run it with `PATH=/tmp/evil:$PATH` and a poisoned environment.
- Paths: every name from outside is joined with `safe_join`; try `..`, an absolute path, a symlink
  to `/etc`.
- Units and polkit: `systemd-analyze security <unit>` for each unit the change touches (50 or
  lower); `pkcheck` an action WolOS does not list (must be refused).
- FFI and decoders: the change adds no `unsafe` (the ratchet test) and every new decoder has a fuzz
  target; run it 10 minutes.
- xguard: hand the check verb `/dev/zero`, a FIFO and a 2 GB file; it must answer, not hang.

## The report

Post it in `#reports` and attach it to the pull request. Facts, with `file:line`; no "seems" without
a reproduction. Format:

```
Audit: <area>, <commit>, <box>, <date>, <hours spent>
Covered: <what you tried, one line per list item above>
Not covered: <what you could not do and why>

1. [high|medium|low] <one sentence of the problem>
   Where: <path:line>
   Repro: <exact commands or clicks>
   Fix: <what you would change>, test: <the test to add>
   Confidence: <high|medium|low>, <why>
2. ...

Standards misses (no breakage): <STANDARDS.md § and file:line, one per line>
Verdict: done | not done (<which findings block>)
```

Severity: **high** = data loss, privilege, a machine that does not boot, a secret exposed;
**medium** = a feature fails for a real configuration or the user sees raw error text;
**low** = cosmetics, a missing log line, a standards miss. A high finding goes to `#plan` at once.
