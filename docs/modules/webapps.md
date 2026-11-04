# Web apps (ADR 0042)

Two tracks (ADR 0042). Track 1: apps that bundle their own web engine (packaged desktop apps, the
code editor, the browser) stay as packaged and keep the Memory Saver (ADR 0026); WolOS tunes them
through runtime profiles and per-app policies. Track 2: WolOS's own web apps and site-as-app
entries run on WolOS's runtime on the Rust browser engine, which reads the same policies.

## Per-app policy

`/usr/share/wolos/webpolicy/<app>.json` (repo: `files/usr/share/wolos/webpolicy/`), decoded by
`wol_core::webpolicy::parse` (`crates/core/src/webpolicy.rs`): unknown keys are refused, the file is at
most 64 KiB, the app id is the file stem.

| Field | Meaning | Track 1 | Track 2 |
|---|---|---|---|
| `telemetry_off.hosts`, `.beacon_paths` | analytics hosts and beacon paths to drop | through `patches` | engine adaptor |
| `jank.busy_timers`, `.hidden_animations` | clamp busy timers, stop animations of hidden windows | through `patches` | engine adaptor |
| `heap_mb` | ceiling for the heap-cap flag rows (128 to 65536); absent: no cap | flag files | engine adaptor |
| `background_throttle` | allows the throttling and timer-alignment rows | flag files | engine adaptor |
| `features` | web features the app needs | not used | the probe vocabulary |
| `patches` | byte edits: `file`, `sha256` of the whole original file, `op` `replace` or `remove`, `offset`, `find`, `with` | pacman hook | never (no file edits) |

`offset` is part of the patch schema: the byte position of `find` in the original file. The helper
edits bytes at a known position and never searches, so an edit cannot land anywhere else.

A patch is data and never runs code. It edits one regular file under `/usr/lib/` or `/opt/`, only
while the whole file has the `sha256` it was written for and the bytes at `offset` are `find`;
anything else is refused and the file stays as packaged.

Shipped role policies: `electron.json` (desktop apps from pacman on the shared engine), `code.json`
(the code editor), `helium.json` (the browser). None ships a patch yet: a patch needs the hash of
the exact packaged file and a measured gain.

### How track 1 applies it

- Flag files: `wol_core::webruntime::flags_with` with `webpolicy::allows` decides which rows of the
  flag table reach `/usr/share/wolos/webruntime/<profile>-<soft|gpu>/<role file>` (below).
- Patches: `wolos-webpolicy.hook` (`files/usr/share/libalpm/hooks/`) runs
  `/usr/lib/wolos/webpolicy` after every install or upgrade of a plan or a patched file. The
  helper is POSIX sh and does not read JSON: `webpolicy::plan` compiles each policy into
  `<app>.plan` next to it (one line per patch, five one-word fields, grouped by file, offsets
  descending). It checks the hash and the bytes, writes a sibling file, keeps the packaged mode,
  renames it into place and records the patched hash in `/var/lib/wolos/webpolicy/<app>.state`,
  so a second run is a no-op, a reinstall of the same version is patched again and a new
  version is refused until the policy is updated. The hook never fails a package transaction.
- State, one line per file (`webpolicy::parse_state` reads it, Settings shows "policy not
  applied" from it): `patched <file> <sha256>` or
  `refused <file> <unix time> <expected sha256> <found sha256 or -> <reason>`. A refusal is also
  printed on stderr; a later successful run replaces it.
- AppImages (`wopkg::appimage::policy_args`): the image is signed and compressed and is never
  edited, so `patches` are not applied; the launcher's `Exec=` gets the flag rows the app's
  policy (`<launcher slug>.json`) allows, written at install and again after every update.

## Runtime profiles (track 1 flag rows)

One profile per account: Balanced (default), Light or Full, one word in
`$XDG_CONFIG_HOME/wolos/web-runtime` (`webruntime::PROFILE_FILE`); it applies at the next session
start. Full adds no caps and no throttling; Light never rasterises on the GPU.

- The table is `webruntime::ROWS` (`crates/core/src/webruntime.rs`): each row has a role, the
  profiles it is in, whether it needs a real GPU, a reason and a status. Only `Baseline` rows
  (shipped for correctness before the table) and `Measured` rows (a gain from the web-app
  benchmark, with the path of its record) reach a file; `Candidate` rows are tested and never
  emitted. Today every performance row is a candidate, so each profile writes the platform hint
  and nothing else. Promoting a row is one status change plus
  `WOL_REGEN=1 cargo test -p core --lib -- webruntime:: webpolicy::`.
- Rows: platform hint (Baseline); heap caps per role and profile; wakeup throttling and timer
  alignment; browser policies for background-tab throttling and tab discard (machine-wide, one
  value for every profile; only the rows `helium.json` allows are merged into the browser's
  managed policy file by `theme::helium`);
  GPU rasterisation and zero-copy upload (GPU tier only).
- File rules, tested: the engine keeps only the last occurrence of a switch, so list switches are
  merged; launchers read the files line by line or word-split, so every line is one word; the
  browser never gets a feature list (it would replace its launcher's video-decode features); no
  row may move or disable the code cache or snapshot data in the app's profile directory
  (`webruntime::FORBIDDEN`).
- Linking: wol-session picks the GPU tier (a render node, not a VM adapter the compositor
  rejects, and `wol hw compute` naming a backend other than `cpu`) and runs
  `/usr/share/wolos/webruntime/link.sh`, which links each of `electron-flags.conf`,
  `code-flags.conf` and `helium-flags.conf` in the config home that is missing or already a link
  into `/usr/share/wolos`. A file of your own is never touched.

## Memory Saver

`memsaver::app::Facts::runtime_discarded`: when the app's own runtime has already given its
background content back, the saver goes to Saving without its own reclaim pass; App Nap and
Restore are unchanged. wold passes `false` until a runtime reports its discards.

## System web apps (ADR 0048)

WolOS's own tool apps (Doctor first) are folders under `webapps/<slug>/`: `app.toml`, `index.html`,
`app.js`, `strings.js`, an icon and, for development, `fixtures.js`. They share the kit in
`webapps/kit/` (`wolkit.css`, `wolkit.js`, `bridge.js`, `mock-bridge.js`, `fixtures.js`) and reach
the system only through `window.wolos`:

```js
const r = await wolos.call("audio.doctor");          // a wol verb's --json output
const off = wolos.subscribe("permissions", redraw);  // app | permissions | progress
wolos.app.theme;                                     // "light" | "dark"
```

Start a new app by copying `webapps/hello/`: change `app.toml` (id `org.wolos.webapp.<slug>`, the
folder named after the slug), list the permissions it needs from
`crates/webengine/src/bridge/catalog.rs`, put every word in `strings.js`. Open
`webapps/<slug>/index.html?dev` in a desktop browser to run it against the recorded answers
(`&theme=dark`, `&deny=audio.read`, `&fail=audio.doctor`, `&delay=0`). A method the catalog lacks
is a `wol` verb first, then one catalog row in both `catalog.rs` and `mock-bridge.js`, then a kit
fixture.

Checks: `cargo test -p webengine --test webapps` validates every manifest and permission, the
mock's catalog copy, the fixtures, the tokens and the strings tables; with
`CHROME=<headless browser>` it also runs `webapps/hello` in development mode and saves light and
dark screenshots to `WOLKIT_SHOTS`.

The kit's elements are tested without a browser: `webapps/kit/test/elements.test.mjs` runs the
eleven elements of `wolkit.js` (what each builds, what its attributes, keys and clicks do, what it
announces) on `design/tools/dom-stub.mjs`, a DOM made for this (no dependency): elements, text,
attributes, selectors, bubbling events, the custom-element lifecycle, focus, `<dialog>`, hash
navigation, `dom.key`, `dom.type` and `dom.click` with the default actions that matter. It has no
layout, no style cascade and no shadow DOM; `dom-stub.test.mjs` holds it to the browser behaviour the
kit tests lean on. Run them with `node --test webapps/kit/test/*.test.mjs` (a directory argument
fails on Node 22, so name the files or use the glob); `cargo test -p webengine --test kit_js` runs
every `webapps/<app>/test/*.test.mjs` the same way.

## Not verified

- No row is measured yet; switch and policy names in the table are not yet checked against the
  shipped binaries.
- The hook and the flags on an installed system (no VM run yet).
- System web apps run in development mode only: the engine has no host channel yet
  (`wwe_engine_set_host_channel`, ADR 0048), `wol-webhost` and wold's `webapp-call` are not built.
