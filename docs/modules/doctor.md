# Doctor

Decision record: [0048](../adr/0048-system-web-apps.md) (system web apps). Code: `crates/controller/src/doctor.rs`; the app: `webapps/doctor/`.

One list of checks for the whole computer, each worded for a person and each with the fix it can run. `wol doctor` is the command, the Doctor web app is the same list with buttons.

## Checks

**Concept.** A check is `{id, area, title, status, detail, fix?}`. `id` is `<area>.<name>` and stays the same between releases; `status` is `ok`, `warn`, `fail` or `skipped` (not checked on this computer); `detail` is plain words and carries no command; `fix` is `{title, command, needs_admin}` and is absent when WolOS has nothing safe to run (the detail then says what to do).

**Why.** Sound, games, updates, security, storage, network, power and displays each had their own doctor or status verb with their own words. A person with a problem does not know which verb to ask. One list lets the app show counts per area and one "Fix" per row, and lets `wol doctor fix <id>` run it.

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

A fix is one of two things: wol-admind requests (`needs_admin`: installing a package, turning on the firewall; the password is asked for like for every admin verb) or one command run as the person (`systemctl --user start ...`, `wpctl set-volume ...`). `action_of` only turns the recognised shapes of the existing doctors' fix lines into actions; anything else (`enable [multilib]`, "plug in a microphone") stays advice and gets no button.

## The commands

```
wol doctor [--json] [area]       every check, or one area; exit 1 when any check is `fail`
wol doctor fix [--json] <id>     run that check's fix, then check its area again
```

`--json` is implied when stdout is not a terminal. `wol doctor --json` prints `{checked, ready, checks}` (`checked` in seconds since 1970). `wol doctor fix --json` prints `{id, ran, ok, check}`: `ran` is false when the check is fine or has no fix, `ok` is true when the check is no longer `fail` afterwards, `check` is the check as it is now. Unknown ids exit with "not found", a fix that cannot reach wol-admind exits with "unavailable".

The verbs are in the catalog as `doctor` (read) and in the command tree as `doctor.check` and `doctor.fix`.

## The web app

`webapps/doctor/` (`org.wolos.webapp.doctor`, permissions `doctor.read` and `doctor.fix`). Two bridge methods:

| Method | Permission | Answers with |
|---|---|---|
| `doctor.check` | `doctor.read` | the output of `wol doctor --json` |
| `doctor.fix` `{id}` | `doctor.fix` | the output of `wol doctor fix --json <id>` |

The sidebar lists the areas with the number of checks that need a look (red when one has failed, orange otherwise). The page lists each area's checks, worst first; a row opens to its detail, a Fix button when there is a fix, and "Show the command", the only place a command appears. Fixing shows progress, then "Fixed." or what is still wrong. "Check again" runs everything again; when nothing needs a look the page says "Everything is fine" with the time of the last run. Keyboard: Up, Down, Home and End move between rows, Enter or Space opens one, Escape closes it.

For `needs_admin` fixes the approval sheet is wold's, not the page's; the page just waits for the answer.

### Launching it

The launcher entry is generated from the manifest (`Manifest::desktop_entry`): `org.wolos.webapp.doctor.desktop`, `Exec=/usr/bin/wol-webhost /usr/share/wolos/webapps/doctor/app.toml`, `Icon=/usr/share/wolos/webapps/doctor/icon.svg` (a green rounded square with a heart and a pulse line), `StartupWMClass=org.wolos.webapp.doctor`. The shell's Settings pages that now say "check" can open the app by that id.

## Development and tests

Open `webapps/doctor/index.html?dev` in any browser: `kit/mock-bridge.js` answers from `fixtures.js`. The fixtures are `wol doctor --json` recorded on a build box (`fixtures/doctor-check.json`); the answers of `doctor.fix` are written by hand in the shape `wol doctor fix --json` prints (`fixtures/doctor-fix.json`), and `fixtures/doctor-fix-no-admind.json` is the real answer when wol-admind is not there. The recording has at least one check of every status, and the firewall is shown off so a fix can be tried.

| Test | Covers |
|---|---|
| `crates/controller/tests/doctor.rs` | argument parsing, every area with stable ids, one area, fixes through wol-admind (password on stdin) and as the person, nothing to run, unknown ids, no commands in words shown on screen |
| `doctor.rs` unit tests | which fix lines become actions and which stay advice, the text report |
| `webapps/doctor/test/doctor.test.mjs` | headless Chromium (`CHROME`, `--password-store=basic --use-mock-keychain`): counts, keyboard, Fix with progress and result, "Show the command", empty and error states, light and dark at 1280 and 800 wide with pictures in `OUT` |
