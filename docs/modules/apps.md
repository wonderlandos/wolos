# Bundled apps (ADR 0036, ADR 0039)

The apps that come with WolOS. The plan (wave 3, package CR-11) is that each of them is a Crystal app on Cryswol and Wolkit (ADR 0036, ADR 0039) with a new visual, built and tested headless on libwolkit (`crates/wolkit`), and drawn on screen once the native presentation lands. This page lists them and the rules they share; each app has its own page.

| App | Id | Program | Package | Source | Page | State |
|---|---|---|---|---|---|---|
| Gallery | `org.wolos.gallery` | `wol-gallery` | `wolos-app-gallery` | `wolos-crystal/apps/gallery` | [gallery.md](gallery.md) | headless build, specs and goldens; pictures draw when libwolkit paints images |

Next in line: documents, music, notes.

## What every bundled app has

- **An id** `org.wolos.<name>` (frozen once shipped), passed to `Wolkit.run ..., app_id:`, used for its permissions, its preferences and its data folder (`~/.local/share/org.wolos.<name>`).
- **A design note** `wolos-crystal/apps/<name>/DESIGN.md`: layout, motion on the named curves of the style language (`fast`, `normal`, `slow`, the theme ease; springs `gentle`, `snappy`, `bouncy`), the empty, loading and error states, the keyboard map. One page.
- **The kit frame** (`app_window`, `toolbar`, `sidebar`, `section`, `list_row`, `empty_state`, `sheet`, `menu`, `tabs`) and the HIG (`docs/modules/hig.md`): at most five toolbar actions, errors inline in one sentence, undo before confirm.
- **Its own look** only as classes in `src/styles/*.wolcss`, compiled after the kit sheet and committed as Crystal (`src/<name>/style/`), checked for drift by `make spec`. Theme tokens and the design scale only.
- **Specs**: the pure parts without an engine; the app in the headless engine with the pinned fonts, with a golden screenshot and accessibility tree for each state (900 x 600 and the narrow 480 x 360), including dark mode.
- **A package** `wolos-pkgbuilds/wolos-app-<name>` that builds with the app's Makefile against the system libwolkit and libwol, runs the app's `make spec` in `check()`, installs `/usr/bin/wol-<name>` and `/usr/share/applications/org.wolos.<name>.desktop`, and has a contract test in `checks/tests/app_<name>.rs`.
- **A crafted icon** from `design/icons/apps` named in the launcher entry.

## Lessons from the first app

- A region (`switch`, `show`, `each`) row must contain at least one element, and a component alone does not count as one: wrap each branch in a `column`.
- Give each screen of a `switch` its own column, so the regions inside it are parts of that column and are freed with it.
- Sheets for `wolkit-stylec` are read in sorted path order: put the kit sheet and the app's sheets in one folder with the kit's name first (`tools/styles.sh` of the gallery does this).
- An element macro's name cannot also be a method of the app (`menu`, `toolbar`, ...): the getter wins and the element call fails to compile.
