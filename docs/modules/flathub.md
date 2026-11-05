# Flathub in the App Store: curation and the Flatpak runtime (ADR 0021)

Decision records: ADR 0021 (wopkg: Flathub is one of the sources), the hub's records (hub repo, ADR 0046 (author certificates) and ADR 0049 (the supply-chain policy), not in this tree yet: who made it, and what WolOS did about it). Code: `crates/core/src/flathub.rs` (curation), `crates/core/src/flatpak.rs` (argv, permissions, consent grants), `crates/core/src/appstream.rs` (`verified_ids`), the Store tiles in `crates/shell/src/ui/store.rs`. Tests: `crates/core/tests/flathub.rs`, `crates/realtime/src/memsaver.rs` (players), `crates/api/src/memsaver/engine/tests.rs` (adoption); in wolos-pkgbuilds `checks/tests/recipe_flatpak_runtime.rs`.

## What the Store knew, and what it was missing

Flathub was already a second source: `flatpak search` under the catalog ("From Flathub"), a "Flathub" badge on the tile, install for the account (`flatpak install --user flathub`), the consent sheet before an app gets its permissions (`wol-flatpak-consent`, the `flatpak-wolos` fork), updates with the system's. It had no opinion: 30 hits in Flathub's order, all badged the same, and nothing to open on Discover.

## Three standings

| Standing | Badge under the name | Meaning | Source |
|---|---|---|---|
| Picked | `Flathub · Picked by WolOS` | in `flathub::CURATED`: WolOS chose it for the list. A choice, not a code audit | the list, 22 apps on 2026-11-04 |
| Verified | `Flathub · Verified by Flathub` | Flathub verified that the developer owns the app id (a website or a forge account) | `flathub::verification::verified` in the local AppStream (`appstream::verified_ids`, read once off the UI thread by `flathub::warm`) |
| Community | `Flathub` (unchanged) | neither: packaged by whoever submitted it | everything else |

Consistent with the hub's supply-chain policy (hub repo, ADR 0049): the person always sees what was done and never the word "safe"; the badge says who did what. The hub's Trusted level means a reviewer read every release against the manifest, file list and provenance; nothing of that exists for Flathub, so a pick is deliberately not called reviewed. Verified corresponds to the hub's Origin level (an automatic proof of who published). A pick may be unverified (Spotify, Slack, Zoom and Chrome are packaged by the community on Flathub, not by their vendors); the details page says so: "Picked by WolOS, but the community packages it, not the developer."

None of the three widens what an app may do. Permissions stay with the consent helper at install and at every update; a pick is not pre-approved.

## What the curation does

1. **Ranking.** `flathub::rank` puts picks first, verified apps next, the rest last, each group in Flathub's order (`show_more`).
2. **Badges.** `copy::store_source_label(standing)` (the words live in the shell's `copy.rs`, store section) on the tile and in the details "Source" row, and `copy::store_source_about` in the "About this source" row; a plain community app keeps the exact "Flathub" the goldens have.
3. **A shelf.** Discover ends with "Picked from Flathub" (nine tiles of `flathub::curated_hits`), shown where the machine is searched (`env.pkg_search`).
4. **One app, one tile.** The list may not repeat a native catalog app (test), and `covered_natively` still drops a Flathub hit that a native package covers: ours, then Cachy, then Arch, win.

The list: apps people ask for that the repositories do not carry, each checked against Flathub's API on 2026-11-04. To add one: the id, a one-line summary (sentence case, no full stop), a category; `cargo test -p core --test flathub` checks the id's shape, repeats, the summary rules and the native overlap.

## The runtime around the sandbox

| Part | What the image does | Proof |
|---|---|---|
| Portals | `hyprland-portals.conf`: ScreenCast and Screenshot from the compositor; FileChooser, Settings (colour scheme, accent), AppChooser and Print from `gtk`; Secret (passwords of browsers and chat apps) from `gnome-keyring`. Before IMAGE-F Secret had no backend named, and an app asking got no answer | `recipe_flatpak_runtime.rs` |
| Fonts | `/usr/share/fonts` is mounted in every sandbox at `/run/host/fonts`: Inter, JetBrains Mono, Noto (Latin, emoji, CJK). The runtime's own fontconfig does the fallback; our `70-wolos-cjk-arabic.conf` is a host file and does not reach the sandbox | packages in `recipe_flatpak_runtime.rs` |
| Colour scheme and accent | the Settings portal reads gsettings (`wol theme` sets them); libadwaita apps follow live | portal line above |
| Theme files | **not solved.** The WolOS GTK/Kvantum files in `~/.config` are symlinks into `~/.config/wolos/toolkits`; a sandbox cannot follow them without `--filesystem=xdg-config/gtk-4.0:ro`, `xdg-config/gtk-3.0:ro` and `xdg-config/wolos/toolkits:ro`. That widens every app's view of the home folder, so it is the consent helper's decision, not a silent global override. Open item, needs the fork and a VM | none |
| Memory Saver | a Flatpak app is adopted by its window like any app: the window's process and its children go into `app-wol-<id>-<pid>.scope`; `flatpak run`, `bwrap` and the D-Bus proxy above it stay in the app's own scope. Proof: `a_flatpak_app_is_adopted_by_its_window_and_its_supervisors_stay` (already behaved) | engine tests |
| Memory Saver, sound | **fixed.** A Flatpak's window class is its app id (`com.spotify.Client`) and its MPRIS name the short one (`spotify`), so a playing Flatpak was not found and could be frozen with the window hidden. `players()` now matches a word of a reverse-DNS class after its first (not a vendor word such as `gnome` or `mozilla`). The sound stream cannot stand in: a sandboxed client reports its pid inside the sandbox | `a_flatpak_player_is_found_by_the_short_name_of_its_app_id` |

Not verified in a running Flatpak (no Flatpak on the build box): the portal answers, the sandbox pid of a stream, the cgroup moves. The VM journeys of the image cover them.
