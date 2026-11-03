# Web apps

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

## Not verified

- No row is measured yet; switch and policy names in the table are not yet checked against the
  shipped binaries.
- The hook and the flags on an installed system (no VM run yet).
