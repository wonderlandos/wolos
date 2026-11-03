# Packaging drafts (for wolos-pkgbuilds)

These files are not built here; they belong in `wolos-pkgbuilds` and are kept next to the app so the
change that adds an app option or a file also changes its package.

| Here | Goes to | Notes |
|---|---|---|
| `PKGBUILD` | `wolos-pkgbuilds/wolos-app-looks/PKGBUILD` | `_wolos_commit` is bumped by `tools/bump-wolos.sh` like the Gallery's; `license=(GPL-2.0-or-later MIT)` because the engine's four ported files are MIT |
| `app_looks.rs` | `wolos-pkgbuilds/checks/tests/app_looks.rs` | The contract test, shaped like `app_gallery.rs` |
| `wol-looks-schedule.service`, `.timer` | installed by the recipe into `/usr/lib/systemd/user/` | The app enables the timer itself (`systemctl --user enable --now wol-looks-schedule.timer`) when a schedule is set and disables it when it is off; no preset is needed |
| the `packages` line | `wolos-pkgbuilds/packages` | `wolos-app-looks wolos-app-looks libwolkit,libwol,wolos-system wolos-app-looks/` |
| the icon | `design/icons/apps/wolos-looks.svg` (brand) | The desktop entry uses `wolos-settings` until the brand adds it; then change `Icon=` |
