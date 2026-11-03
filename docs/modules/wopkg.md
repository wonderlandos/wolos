# wopkg (ADR 0021)

One tool for every source. `wopkg` (Crystal CLI, `wopkg/`) drives `libwopkg` (`crates/wopkg`, `libwopkg.so.1`, header `include/wopkg.h`, symbols in `abi/wopkg.map`). The App Store links the Rust crate directly.

The CLI's `lib LibWopkg` (`wopkg/src/wopkg/lib_wopkg.cr`) is generated from `wopkg.h` by the same generator as libwol's (`crates/wol-abi/tests/common/bindgen.rs`); `cargo test -p wol-abi --test bindings` fails when it drifts. Crystal apps reach the same calls through `require "cryswol/pkg"` (`Wol::Pkg`, [cryswol.md](cryswol.md)).

## Sources and ids

| `kind` | Where | `id` | Installs as | Update |
|---|---|---|---|---|
| `pacman` | `[wolos]`, `[wolos-cachy]`, Arch repos | package name | system change through wold | `wopkg update` (pacman, sealed live layer) |
| `wol` | `.wol` records | app id | system change (pacman package) | with the system |
| `flatpak` | Flathub and other signed remotes | app id | `flatpak --user` | `flatpak update` |
| `appimage` | `gh:owner/repo` releases, files | URL | `~/Applications` | zsync delta, else full download; key pinned at first install |
| `recipe` | `wopkg/recipes/*/recipe.toml` | recipe name | `~/.local` | rebuild when the version changes |
| `aur` | community package collection metadata dump | package name | sandboxed build, then `package.install-file` | rebuild; always `unreviewed` |

Flathub is added for the account the first time an app is installed from it. On an installed slot the package database is the one `/etc/pacman.conf` names (`/usr/lib/pacman`).

`catalog` is Arch's AppStream (`archlinux-appstream-data`). It installs nothing: it ties a package name to an AppStream id, so search can find a package by its app's name and one app shows once. Sources join only when their ids agree. In the October 2026 data, 604 of Arch's 1401 apps share an id with Flathub; the rest stay separate entries, and a package without metadata never joins by name. `wopkg pin <name>` keeps a package out of `wopkg update` (`package.update-holding` passes `--ignore`); `wopkg update <name>` updates one app.

`wopkg audit <pkg>` checks a community-collection package out and reads it with `wopkg-aurlaws` without building it (see [aurlaws](aurlaws.md)). The same analyser runs after the sandboxed build, on the files, the built package and the trace.

Search ranks exact name, prefix, word, summary; ties by the source order above. One result per app (`key`), each listing every source.

## JSON documents

Config (`wopkg_context_create`, all optional): `home`, `cache_dir`, `state_dir`, `data_dir`, `apps_dir`, `pacman_local`, `flatpak_dirs`, `wol_records`, `keys_dir`, `wold_socket`, `release_file`, `allow_loopback_http`, `jobs`. Unknown keys are refused.

Search result (`wopkg_search`):

```json
{"query":"firefox","took_us":210,"results":[
 {"key":"firefox","name":"firefox","summary":"...","score":1000,
  "sources":[{"kind":"pacman","repo":"extra","name":"firefox","id":"firefox","version":"150.0-1","summary":"...","installed":false}]}]}
```

`wopkg_info` returns the same hit for one name. `wopkg_installed` returns `{"pacman":[names],"flatpak":[ids],"wol":[ids],"recipe":[names],"appimage":[{"name","path"}]}`.

Index update request: `{"sources":[{"name","kind","url"|"path","format"?}]}` (the CLI builds it from `/etc/pacman.conf`, Flatpak's appstream copy and the local feeds), result `{"sources":[{"name","records","rebuilt","error"?}]}`. A source is rebuilt only when its input's size, mtime or ETag changed.

Downloads are bounded: an index input at most 512 MiB (`fetch::MAX_INPUT`), the release feed `<channel>.json` 4 MiB, the plugin index `index.json` 4 MiB, a `.sig` 64 bytes. A longer body is refused and its part file deleted, so the last good copy stays.

The release feed lives at `wopkg::FEEDS`, the assets of the latest wolos-packages release: `https://github.com/wonderlandos/wolos-packages/releases/latest/download/<channel>.json` and `<channel>.json.sig` (GitHub redirects to the asset; the download follows it). Every publication uploads the feeds of all channels again, so `latest` always carries each one. In v0.12 the feeds are published with `expires` 0: the serial floor alone stops a replayed older feed.

Signed lists (release feed and plugin index) carry `serial` (increasing with each publication) and `expires` (Unix seconds, 0 = none) next to `format`, both covered by the signature. A copy is checked (signature, format, channel, serial not below the highest seen, not expired) before it replaces the cached copy and its ETag; the highest feed serial per channel lives in the state dir. When a new copy fails, the cached copy is used while it still checks. The Store downloads the plugin index again once half of its remaining lifetime or a day has passed, and an expired cached list is not shown.

Progress events (callback, valid during the call, JSON with an `event`): `bytes` (`url`, `done`, `total`), `done` (`url`, `outcome`), `source` (`report`, from an index update), `appimage` (`name`, `result`). New events and fields may appear; ignore what you do not know.

History entry (`wopkg_history_record`, one JSON line each in `history.jsonl`): `{"source","action","name","from","to","undo"}`; `undo` is a document tagged by `type` (`restore` with `from`/`to`, `remove` with `path`, `run` with `argv`, `system` with `note`). `wopkg_rollback(id)` runs the undo.

## System changes (wold)

`wopkg_system_call` sends one line `call <json>` to the wold socket and reads one answer line. Calls: `package.change` (`verb` install|remove|update, `names`), `package.install-file` (`path`), `system.update` (`manifest`, `signature` hex), `system.channel` (`channel`), `system.rollback`. wold takes the caller's executable from the kernel and checks the consent grant: `packages-install`, `packages-remove`, `packages-update`, and `packages-unreviewed` for files built locally. Denials say where to allow it (Settings > Privacy & Security > Software). Nothing else is trusted from the request. Every call except `system.update` (a signed, newer release) is an administrator sheet the shell runs through wol-admind after the password; `install-file` installs wold's own staged copy (ADR 0034).

## Recipes

`recipe.toml`: `name`, `version`, `summary`, `license`, `homepage`, `needs` (tools the sandbox must have), `[[source]]` (`url`, `sha256`, `file`), optional `[fetch]` (lockfile step, with network), `[build]` and `[install]` (`run` = shell lines, offline sandbox; `$TARGET_PREFIX` is the final prefix, `/work/out` the staging one). Built output is cached by the recipe's hash. Everything under the staging prefix is copied into `~/.local` as it is, so a recipe may also write `share/applications/*.desktop` (a launcher entry that exists only while the recipe is installed) or files its program reads, such as `lib/<name>/profile.ps1`.

## Exit codes (CLI)

0 done, 1 usage or bad input, 3 an installer failed, 5 refused for trust (unsigned remote), 6 not found or not available from that source.
