# Games

Steam is the native Arch package from `[multilib]` (ADR 0016). Windows games run on Proton, which Steam installs and updates itself. There is no custom compatibility tool and no Linux ABI shim. For Windows programs outside Steam, `wol-run --exe` uses the native `wine` package (`/usr/bin/wine`, WoW64) and falls back to umu-launcher (`/usr/bin/umu-run`, multilib) when wine is not installed.

## Pieces

| Piece | Code | Does |
|---|---|---|
| `wol games steam [url]` | `crates/controller/src/games.rs` | checks what the client needs; first start: a notification; then `/usr/bin/steam [url]` |
| `wol games doctor [--json]` | `crates/core/src/games/doctor.rs` | every check with the exact fix; exits 1 when one fails; JSON when not a TTY |
| `/usr/bin/wol-run --exe <file> ...` | `crates/proxy/src/exe.rs` | opens a `.exe` (`.msi` through `msiexec /i`) with one prefix per program; wine gets DXVK/vkd3d-proton copied from a Proton in the Steam library, umu brings its own |
| `steam.desktop` | `desktop/overlay/.../applications` | Apps, Search, App Store "Open", `steam://` links: `wol games steam %u` |
| Notifications | `crates/core/src/games/notice.rs` | what a Dock or Apps start says; bodies pass `shell::copy::lint` |

## Doctor checks

| Check | Fails when | Fix |
|---|---|---|
| `base` (info) | never: names the WolOS base (os-release) and whether the kernel is linux-wolos | another kernel is reported and kept |
| `multilib` | `[multilib]` off in `/etc/pacman.conf` | enable it, `pacman -Sy` |
| `steam` | no `/usr/bin/steam` | `pacman -S steam` |
| `drm` | no `/dev/dri` | `wol hw status` |
| `lib32-gl` | no 32-bit Vulkan loader or OpenGL (the client is 32-bit) | `lib32-vulkan-icd-loader`, `lib32-mesa`, vendor lib32 driver |
| `vulkan-icd`, `vulkan` | no ICD in `/usr/share/vulkan/icd.d`, or only llvmpipe | vendor Vulkan packages |
| `nvidia` | `lib32-nvidia-utils` missing or not the running driver's version | `pacman -S lib32-nvidia-utils` |
| `futex-waitv`, `ntsync` (warn/info) | kernel lacks them: Proton falls back to esync/fsync | none |
| `wine` (info) | not installed | only for programs outside Steam |

The client starts only when `multilib`, `steam`, `drm`, `lib32-gl` and `nvidia` pass (`doctor::STEAM_CLIENT`).

## Display and presentation (audit X1)

Steam, Wine and most native games are X11 clients and run through Xwayland. Xwayland renders them 1:1 (`xwayland:force_zero_scaling` in `hypr/wolos-perf.conf`). On a HiDPI screen they would be half size, so they are told the scale instead. `wol games steam` adds `STEAM_FORCE_DESKTOPUI_SCALING=<largest monitor scale>` when it is not 1 (`wol_core::games::steam::ui_scale`). `wol-x11-env` also exports it, with `GDK_SCALE`, to the systemd user environment, and Dock and Apps launches pass both on (`shell::kit::scale_env`).

Fullscreen games present immediately (tearing allowed: `general:allow_tearing`), not at the next vblank. Three window rules in `wolos-perf.conf` do this:

| Rule | Matches |
|---|---|
| `wol-game-immediate` | Steam and Proton games (`steam_app_<id>`) |
| `wol-wine-immediate` | Windows programs started with wine outside Steam (class ends in `.exe`) |
| `wol-xwayland-fullscreen-immediate` | any other fullscreen X11 window (native, GOG, itch). A fullscreen X11 video player tears too. |

`misc:vrr = 2` turns VRR on for fullscreen windows only. `xwayland:use_nearest_neighbor` scales a game's lower fullscreen mode sharp, not blurred.

The image ships `gamescope` for Steam's `gamescope %command%` launch option and for games that need a fixed resolution. Memory Saver treats it as a game process.
