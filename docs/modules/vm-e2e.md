# VM end to end: an agent drives the whole system (wolvm) (ADR 0029)

What a browser test runner gives a web app (open the page, click, wait for text, screenshot,
read the console), `wolvm` gives the whole OS: boot the ISO, go through the installer, reboot
into the installed disk, log in, open apps, collect the journal. Code and command reference:
`wolos-iso/tests/wolvm/wolvm` and `wolos-iso/tests/README-wolvm.md`. Decision: ADR 0029 (one
build path; an ISO is released only after its VM test passed). What it drives in this repository:
the installer (`crates/shell/src/ui/installer.rs`, `crates/controller/src/install.rs`), first boot
and the desktop as shipped.

## Why

- **One step at a time.** The fixed scripts (`tests/test-installer.sh`, `tests/test-install.sh`)
  pass or fail as a whole. To find out *why* a screen is black or a button does nothing, an agent
  needs to act, look, and decide the next action, the way a person sits at the VM.
- **Pixels and facts together.** A screenshot says what a person sees; the journal, `systemctl
  --failed` and `/proc/cmdline` say why. wolvm gives both, from the same VM, at any step.
- **Retry without starting over.** A whole-VM snapshot (RAM, disk, UEFI vars, TPM) taken before a
  risky step takes about 4 s to restore, so an agent tries another path instead of re-booting and
  re-installing (minutes).
- **No special image.** The released ISO is tested as it ships. Everything wolvm needs from the
  guest is already there: systemd-boot reads extra kernel words from SMBIOS, systemd reads
  credentials from SMBIOS, and `qemu-guest-agent` is in the package list.

## How it works

| Piece | Mechanism |
|---|---|
| VM | QEMU q35 + OVMF, KVM when `/dev/kvm` is usable, daemonized; one dir per VM in `$WOLVM_HOME/vms/NAME` (`vm.json` config, `run.json` state) |
| Disks | `disk.qcow2` and the UEFI vars as `vars.qcow2`: every writable image supports internal snapshots. Test disks (`--drive`, raw, never converted: their bytes are compared afterwards), the ISO as a USB-stick-like disk (`--iso-bus virtio`), offline (`--no-net`) |
| Secure Boot on/off | `set --secureboot on|off` flips `SecureBootEnable` in the vars with virt-fw-vars (keys stay): install with enforcement off, boot with it on |
| Screen | QMP `screendump` (PNG; PPM + a stdlib PNG writer as fallback). The pointer moves 1 px and back first, so a compositor that only flushes damage repaints |
| Keyboard, pointer | QMP `send-key` (qcodes, US layout for `type`), `input-send-event` absolute moves on a usb-tablet for `click` |
| Serial | `console=ttyS0` via SMBIOS type 11 `io.systemd.boot.kernel-cmdline-extra` (live and installed); a socket chardev with a log file; `wait-text` reads the log since the last start |
| Shell in the guest | the QEMU guest agent (`guest-exec`, root, output captured); with `--ssh`, sshd with a per-VM key passed as the credential `ssh.authorized_keys.root`; `ssh-open` starts sshd and opens port 22 for that boot (the image ships both closed) |
| Snapshots | running: HMP `savevm`/`loadvm` (whole VM, swtpm state included); stopped: `qemu-img snapshot` on disk and vars |
| Journeys | YAML subset or JSON list of steps; a screenshot after each; `journey.jsonl`; `report.md` + `index.html` |
| TPM, Secure Boot | swtpm per VM (`--tpm`); `--secureboot` = OVMF secboot code + Microsoft/Red Hat keys enrolled with virt-fw-vars (SMBIOS cmdline extras are ignored then: use the agent) |
| MOK screens | `mok-esp.sh` (signed shim + an unenrolled loader) and `journeys/mok-screen.yaml`: shim's Security Violation box, then MokManager; firmware text also lands on the serial console |
| Unattended | `--append "wolos.autoinstall=/dev/vda ..."` (`files/usr/lib/wolos/autoinstall`); with `--direct` the ISO's kernel boots with exactly our cmdline |

## The journey, boot to apps

1. **Boot the ISO.** `wolvm new w --iso wolos.iso --tpm && wolvm start w`.
   `wolvm wait-text w "Install WolOS: the first screen of the live image"` (the live system on
   the serial console; it prints no "Welcome to" greeting), `wolvm wait-agent w`,
   `wolvm wait-stable w`, `wolvm screenshot w now.png`.
2. **Drive the installer.** Read the screenshot, act (`key tab`, `key ret`, `click X Y`,
   `type "name"`), `wait-stable`, screenshot again. `snapshot save` before the disk page.
   Ask the installer what it did: `wolvm ssh w 'journalctl -b -t wol-installer -n 50'`.
   Or unattended: `--direct --append "wolos.autoinstall=/dev/vda"` and
   `wait-text "wolos-install: done"`.
   **Tester note (unattended installs):** an autoinstall without `wolos.plan=` creates no user
   account, so the installed disk boots to the login window with nobody to log in as.
   `wolos.autologin=yes` is a word for the live session only; the installed system ignores it.
   For a ready account, pass a plan the app wrote (`wolos.plan=WOLOSKEY:plan`, as
   `WOLOS_PLAN=auto tests/test-install.sh` does). A user made by hand in the installed system
   (`useradd`) must be in the group `seat`, or the desktop cannot open the seat and the session
   restarts in a loop.
3. **Reboot into the installed disk.** After the power-off: `wolvm set w --boot disk`
   (or `eject` + reboot from the guest), `wolvm start w`. The installed system boots from its own
   ESP; the SMBIOS console words still apply, so serial and agent work there too.
4. **Unlock and log in.** Screenshot: the unlock or login screen; `type` the passphrase or
   password and `key ret`. Check: `wolvm ssh w 'loginctl list-sessions'`.
5. **Open apps.** `key super` / the search bar, `type terminal`, `key ret`; screenshot; check with
   `wolvm ssh w 'pgrep -a wol-'`.
6. **Collect.** `wolvm logs w -n 1000 -o journal.txt`, `wolvm ssh w 'systemctl --failed'`,
   `wolvm report DIR` for a person.

A path that works goes into a journey file (`tests/wolvm/journeys/*.yaml`) so the next run is
`wolvm journey w FILE` and a report.

## How an agent reads a screen and picks the next key

- Take the screenshot after `wait-stable`; open it with the Read tool (the image is shown).
- Say what is there: the page title, which control has the focus ring, dialogs, error text.
- Pick the smallest action that moves forward: Enter on the focused default button; Tab to move
  focus; a click on a button centre read off the image (wolvm clicks in image pixels).
- Screenshot again and compare: if nothing changed, the action was wrong or the app is busy
  (check the journal before trying again). Restore the snapshot rather than stacking guesses.
- Text in a picture is not proof of state: confirm with a command in the guest when it matters
  (disk layout: `lsblk -f`; boot entries: `bootctl list`; session: `loginctl`).

## Limits

- Runs on Linux hosts with QEMU (the forge); from Windows go through `forge.sh run` and `scp`.
- `type` knows the US layout only; the guest keyboard layout must be US while typing.
- `virtio-gl` screendumps can fail ("no surface"); wolvm fails then rather than return an old frame.
- With Secure Boot on, no serial console unless the image's own entry has one.
- v0.11.5's ISO is not signed, so under Secure Boot the firmware refuses it before any WolOS code
  runs; the MOK path is exercised with the signed shim on a prepared disk instead.
