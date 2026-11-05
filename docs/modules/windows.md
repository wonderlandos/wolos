# Windows: tiling and snapping

Two ways to arrange windows. Tile windows (System Settings > Desktop & Dock > Windows) is the compositor's layout: every window fills the screen side by side, and dragging one rearranges the tiling. Snap (this page) is for floating windows, the default: drag a window by its title bar to a side, a corner or the top and it takes that part of the screen, with a preview while you drag, or use the keys. The model is the edge and corner snapping and arrow-key walk other desktops have. No decision record yet (`NO_ADR_YET` in `crates/controller/tests/adr_index.rs`).

## Behaviour

| Gesture | Result |
|---|---|
| Drag the title bar until the pointer touches the left or right edge of a display | the window becomes that half; a rounded accent-tinted box shows where it will go before you let go |
| Same, near the top or bottom of that edge (within 96 px), or along the top edge near a corner | that quarter |
| Drag until the pointer touches the top edge (the menu bar) | zoom (the compositor's maximise, the zoom button) |
| Drag a snapped window away | it gets the size it had before the first snap, under the pointer |
| Rest the pointer on the zoom button (0.6 s) | the snap layouts chooser opens under it: left and right halves, and the four quarters; a click snaps the window there; Esc, the pointer leaving, or 1.8 s unused close it |
| An edge shared with another display | never snaps: the pointer is on its way across |
| Bottom edge | never snaps (the Dock's) |

Keys: ⌘← and ⌘→ give a half (from the other half they restore the old size); ⌘↑ from a half gives its top quarter, from a bottom quarter the half, from a free window zoom; ⌘↓ from a top quarter gives the half, from a half its bottom quarter, from zoom the old size, from a free window or a bottom quarter it minimises. Left and right on a quarter move it across. ⌘↑ was a second key for Overview: ⌃↑ and the three-finger swipe up stay. On a tiled window ⌘← and ⌘→ move it through the tiling (`movewindow`), ⌘↑ zooms and ⌘↓ unzooms.

Zones, boxes and the key walk are `crates/realtime/src/snap.rs` (pure, cargo-tested). Boxes are frames (title bar included) in the usable area (below the menu bar, above the Dock) with `general:gaps_out` from the area's edges and `general:gaps_in` on each side of a split, the gaps tiled windows have (zoom has none). Fixed-size windows (dialogs, palettes) never snap or resize.

## Parts

| Part | Where | Does |
|---|---|---|
| Policy | `crates/realtime/src/snap.rs`, C ABI `wol_snap_*` in `ffi.rs` and `wol_realtime.h` | zone under the pointer, box of a zone, Super+arrow step, unsnap position |
| Compositor | `plugins/wol-hypr/main.cpp` | listens to the drag controller (`motion`, `ended`), asks the policy, applies the frame with `setTargetGeom`; remembers the frame before the first snap; hover timer on the zoom button; `hyprctl wolsnap` and `hl.plugin.wol.snap()` |
| Shell | `crates/shell/src/snap.rs`, `ui/snap.rs` | `snap-preview` and `snap-layouts` lines on the shell socket (`protocol::Verb::Snap`), the preview layer `wol-snap-preview` (full output, no input) and the chooser layer `wol-panel-snaplayouts` |
| Binds | `desktop/overlay/usr/share/wolos/hypr/hyprland.lua` (and `.conf`) | ⌘ and the four arrows run `hyprctl wolsnap left\|right\|up\|down`; named on the shortcut sheet (`shortcuts.rs`, Windows group) |
| Setting | `wol_core::hyprprefs::DesktopPrefs::snap`, key `plugin:wol:edge_snap` | Settings > Desktop & Dock > Windows > Snap windows to screen edges (on by default); sent live by `hyprctl keyword`; the line in `hypr/wolos/settings.lua` is guarded by `hl.plugin.wol` (the plugin's namespace, registered by wol-hypr) because the compositor reads the file once before the plugin loads. The keys work whether it is on or off |

The shell lines (one per change of zone, never per motion): `snap-preview off`; `snap-preview <output> <zone> <x> <y> <w> <h>` (the frame in the output's own coordinates); `snap-layouts <output> <0xaddress> <x> <y>` (the zoom button's centre). The chooser answers on the compositor socket with `wolsnap zone <zone> address:<0xaddress>`; `left` and `right` without `zone` are the keys.

## Tests

`cargo test -p realtime snap` (zones, boxes, key walk, unsnap, ABI), `cargo test -p core hyprprefs::tests::snap` and `shortcuts::tests::super_arrows`, `cargo test -p shell --lib snap` and `--test hypr_conf` (the plugin's lines are read by the shell; the binds), golden cases `snap-preview` and `snap-layouts` and the changed `settings-desktopdock-*`, `shortcut-sheet-*`. The journey `qa100-window-tiling` (wolvm, tests/wolvm/journeys) drives keys, real drags by QMP pointer events, the chooser and the switch on an installed disk. The plugin itself builds only against the compositor's headers (`plugins/wol-hypr/Makefile`).
