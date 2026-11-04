# Doctor (ADR 0048)

Decision record: [0048](../adr/0048-system-web-apps.md) (system web apps). Code: `crates/controller/src/doctor.rs`; the app: `webapps/doctor/`.

One list of checks for the whole computer, each worded for a person and each with the fix it can run. `wol doctor` is the command, the Doctor web app is the same list with buttons.

## Checks

**Concept.** A check is `{id, area, title, status, detail, fix?}`. `id` is `<area>.<name>` and stays the same between releases; `status` is `ok`, `warn`, `fail` or `skipped` (not checked on this computer); `detail` is plain words and carries no command; `fix` is `{title, command, needs_admin}` and is absent when WolOS has nothing safe to run (the detail then says what to do).

**Why.** Sound, games, updates, security, storage, network, power, displays and hardware each had their own doctor or status verb with their own words. A person with a problem does not know which verb to ask. One list lets the app show counts per area and one "Fix" per row, and lets `wol doctor fix <id>` run it.

**How.** The facts and the rules stay where they live; `doctor.rs` words them and maps the existing fix line to something it can run.

| Area | Checks (`id` after the area) | Facts from |
|---|---|---|
| sound | `pipewire`, `wireplumber`, `realtime`, `config`, `input-level`, `clock`, `xruns`, `rnnoise`, `obs`, `portal`, `devices` | `crates/controller/src/audio_doctor.rs` |
| games | `base`, `multilib`, `steam`, `drm`, `lib32-gl`, `vulkan-icd`, `vulkan`, `futex-waitv`, `ntsync`, `wine` | `wol_core::games::doctor` |
| updates | `checked`, `waiting` | the update snapshot (`run::update_snapshot`): a look older than three days is `warn` |
| security | `firewall`, `remote-login`, `system-log`, `protections`, `malware`, `stopped` | `wol_core::security` (signatures older than 14 days are `warn`) |
| storage | `space`, `encryption`, `restore-points` | `wol_core::storage` |
| network | `connection`, `names`, `devices` | `wol_core::netinfo`, the hardware snapshot |
| power | `battery`, `sleep` | `wol_core::power` |
| display | `screens`, `devices` | `shared::hardware` |
| hardware | `video`, `vulkan`, `firmware`, `fingerprint`, `thunderbolt`, `cpu-power`, `cooling`, `power-modes`, `cpu-level` | `crates/controller/src/doctor/hw.rs` (see below) |

A fix is one of two things: wol-admind requests (`needs_admin`: installing a package, turning on the firewall; the password is asked for like for every admin verb) or one command run as the person (`systemctl --user start ...`, `wpctl set-volume ...`). `action_of` only turns the recognised shapes of the existing doctors' fix lines into actions; anything else (`enable [multilib]`, "plug in a microphone") stays advice and gets no button.

### The hardware area

The `display` and `network` areas ask whether a device has a driver. `hardware` asks whether the part works as well as the hardware can: a laptop of 2023 with a driver for everything can still decode video on the processor, run games on llvmpipe, miss a firmware blob or sit on the old speed driver. Every row reads the machine through `Env` and is `skipped` when the part is absent (no fingerprint reader, no Thunderbolt port) or the tool cannot run, never a failure. A virtual machine skips `video`, `vulkan` and `cooling`: software graphics and no thermal chip are normal there.

| Row | Reads | `ok` | `warn` / `fail` |
|---|---|---|---|
| `video` | `vainfo`: profiles with a decode entry point | "The graphics card decodes H.264, H.265, VP9 and AV1 video" | `warn`: nothing decodes, or the VA-API driver did not start (Intel Broadwell and newer use `intel-media-driver`, AMD uses `mesa`) |
| `vulkan` | `vulkaninfo --summary` | the GPU's name | `warn`: only llvmpipe (software), or the loader failed |
| `firmware` | the kernel log of this boot: final verdicts only ("no suitable firmware found", "failed to load firmware"), not the first tries of a fallback list (iwlwifi asks for `-89.ucode` and loads `-77`) | none failed | `warn`: the drivers named; a system update brings the firmware (`linux-firmware` is the whole set) |
| `fingerprint` | `fprintd-list` for the signed-in account | "A fingerprint reader works", with the number of fingers | `skipped` without a reader. No button: enrolling needs the sensor touched several times in a window of its own (`fprintd-enroll` in Terminal) |
| `thunderbolt` | `/sys/bus/thunderbolt` domain, then `boltctl list` | devices connected | `fail`: a port but no `bolt` (fix installs it); `warn`: a device is plugged in and not allowed (fix: `boltctl enroll`) |
| `cpu-power` | the cpufreq scaling driver | `intel_pstate`, `amd-pstate(-epp)` | `warn`: `acpi-cpufreq` and the other old controls |
| `cooling` | `thermald.service` (Intel, not in a VM) | running, or ended with status 2 (a processor it has no table for; the unit counts that as success) | `warn`: not running (fix: the `cooling-helper` admin action, `systemctl enable --now thermald.service`) |
| `power-modes` | `powerprofilesctl list` | the modes Energy Mode can use | `warn`: the daemon is not answering |
| `cpu-level` | `/proc/cpuinfo` flags and the repo sections of `pacman.conf` (ADR 0030) | the processor runs the optimized (x86-64-v3) builds, or lacks AVX2 and uses the standard ones | `fail`: optimized builds on and no AVX2 (a disk moved to an older computer); fix: the `mirror-gate` admin action, `wol mirror gate --yes` as root, turns them off |

There is one power daemon: `power-profiles-daemon` (Energy Mode, `wol power apply`). `tlp` and the others fight it over the same sysfs files and are not on the image (`wolos-iso/checks/tests/image_modern_hardware.rs` fails if one is added).

## The commands

```
wol doctor [--json] [area]       every check, or one area; exit 1 when any check is `fail`
wol doctor fix [--json] <id>     run that check's fix, then check its area again
```

`--json` is implied when stdout is not a terminal. `wol doctor --json` prints `{checked, ready, checks}` (`checked` in seconds since 1970). `wol doctor fix --json` prints `{id, ran, ok, check}`: `ran` is false when the check is fine or has no fix, `ok` is true when the check is no longer `fail` afterwards, `check` is the check as it is now. Unknown ids exit with "not found", a fix that cannot reach wol-admind exits with "unavailable".

The verbs are in the catalog as `doctor` (read) and `doctor fix` (administrator, capability `maintenance`, asked every time) and in the command tree as `doctor.check` and `doctor.fix`. For a helper (`wol mcp`, a `WOL_HELPER` script) or a web app, a fix that needs the administrator is `ApprovalRequest::DoctorFix` (`doctor::approval`): the check's wol-admind requests, at most 8 and only `shared::shell::DOCTOR_ACTIONS`, shown step by step on the password sheet and run by the shell; a fix of the person's own services is a consent sheet, then `wol doctor fix` runs it. `wol doctor fix` run for anyone but a person at `wol` never sends a wol-admind request itself.

## The web app

`webapps/doctor/` (`org.wolos.webapp.doctor`, permissions `doctor.read` and `doctor.fix`). Two bridge methods:

| Method | Permission | Answers with |
|---|---|---|
| `doctor.check` | `doctor.read` | the output of `wol doctor --json` |
| `doctor.fix` `{id}` | `doctor.fix` | the output of `wol doctor fix --json <id>` |

The sidebar lists the areas with the number of checks that need a look (red when one has failed, orange otherwise). The page lists each area's checks, worst first; a row opens to its detail, a Fix button when there is a fix, and "Show the command", the only place a command appears. Fixing shows progress, then "Fixed." or what is still wrong. "Check again" runs everything again; when nothing needs a look the page says "Everything is fine" with the time of the last run. Keyboard: Up, Down, Home and End move between rows, Enter or Space opens one, Escape closes it.

For `needs_admin` fixes the approval sheet is wold's, not the page's; the page just waits for the answer, which is `{id, ran, ok, check}` like the command's.

### Launching it

The launcher entry is generated from the manifest (`Manifest::desktop_entry`): `org.wolos.webapp.doctor.desktop`, `Exec=/usr/bin/wol-webhost /usr/share/wolos/webapps/doctor/app.toml`, `Icon=/usr/share/wolos/webapps/doctor/icon.svg` (a green rounded square with a heart and a pulse line), `StartupWMClass=org.wolos.webapp.doctor`. The shell's Settings pages that now say "check" can open the app by that id.

## Development and tests

Open `webapps/doctor/index.html?dev` in any browser: `kit/mock-bridge.js` answers from `fixtures.js`. The fixtures are `wol doctor --json` recorded on a build box (`fixtures/doctor-check.json`); the answers of `doctor.fix` are written by hand in the shape `wol doctor fix --json` prints (`fixtures/doctor-fix.json`), and `fixtures/doctor-fix-no-admind.json` is the real answer when wol-admind is not there. The recording has at least one check of every status, and the firewall is shown off so a fix can be tried.

| Test | Covers |
|---|---|
| `crates/controller/tests/doctor_hardware.rs`, `doctor/hw.rs` unit tests | the `hardware` area on fake machines: codecs the GPU decodes, software Vulkan, firmware the kernel could not load, fingerprint reader states, Thunderbolt (port, helper missing, device waiting), the processor power driver, Intel's cooling helper, power modes, x86-64 level against the builds installed |
| `crates/controller/tests/doctor.rs` | argument parsing, every area with stable ids, one area, fixes through wol-admind (password on stdin) and as the person, nothing to run, unknown ids, no commands in words shown on screen |
| `doctor.rs` unit tests | which fix lines become actions and which stay advice, the text report |
| `webapps/doctor/test/doctor.test.mjs` | headless Chromium (`CHROME`, `--password-store=basic --use-mock-keychain`): counts, keyboard, Fix with progress and result, "Show the command", empty and error states, light and dark at 1280 and 800 wide with pictures in `OUT` |
