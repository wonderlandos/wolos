# Games

Windows games run on wine-proton, a native FreeBSD build of Valve's Wine. The Steam client is a Linux program (Valve builds no FreeBSD client), so it runs on the Linux ABI through linux-steam-utils. The WolOS Proton compatibility tool hands each game from Steam to native wine-proton. Games never run on the Linux ABI.

## Pieces

| Piece | Code | Does |
|---|---|---|
| `wol games steam [url]` | `crates/controller/src/games.rs` | checks what the client needs; first start: `lsu-bootstrap` with a notification; refreshes WolOS Proton; starts Steam |
| `wol games steam-setup` | same | writes WolOS Proton into the Steam root; prints Installed, Updated or up to date |
| `wol games doctor [--json]` | `crates/core/src/games/doctor.rs` | every check with the exact fix; exits 1 when one fails; JSON when not a TTY |
| WolOS Proton | `crates/core/src/games/steam.rs` | `compatibilitytool.vdf`, `toolmanifest.vdf` (v2, no `require_tool_appid`: no pressure-vessel), `wolos-proton` (POSIX sh) |
| `wol-run --steam <verb> ...` | `crates/proxy/src/steam.rs` | runs Steam's verbs on wine-proton |
| `wol-run --exe <file> ...` | `crates/proxy/src/exe.rs` | opens a `.exe` (`.msi` through `msiexec /i`) with one prefix per program |
| `steam.desktop` | `desktop/overlay/.../applications` | Launchpad, Spotlight, App Store "Open", `steam://` links: `wol games steam %u` |
| `org.wolos.windows-app.desktop` | same | default app for `.exe` and `.msi` (`mimeapps.list`) |
| Notifications | `crates/core/src/games/notice.rs` | what a Dock or Launchpad start says; bodies pass `shell::copy::lint` |

## The image turns on what the client checks

| Check (linux-steam-utils `steam`) | Shipped by |
|---|---|
| Linux ABI 32 + 64 bit, `/compat/linux` mounts | `linux_enable=YES` (`image/rcconf.sh`), rc.d `linux` |
| Rocky Linux 9 base, linux-steam-utils, wine-proton | `image/packages-base.txt` |
| `security.bsd.unprivileged_chroot=1`, `vfs.usermount=1` | `/etc/sysctl.conf.local` |
| `compat.linux.use_real_ifnames=1` | `/etc/sysctl.kld.d/linux.conf` (applied when rc.d loads `linux.ko`) |
| nullfs loaded | `/boot/loader.conf.d/wolos-games.conf` |
| `/etc/machine-id` | FreeBSD rc.d `hostid_save` |
| NVIDIA: `linux-nvidia-libs` matching the driver | not yet: `wol games doctor` gives the fix |

`tests/image_games.rs` ties each sysctl in `doctor::SYSCTLS` to a shipped line.

## First start (`wol games steam`)

| Step | User sees |
|---|---|
| A client check fails | notification: restart (settings) or `wol games doctor` (missing package); nothing is written |
| `~/.steam/steam/ubuntu12_32` missing | "Getting Steam ready", then `lsu-bootstrap` (adds `--allow-stealing-my-passwords,...` for accounts in wheel: the WolOS account is the administrator) |
| Download fails | "Steam couldn't download. Check that you're online, then open Steam again." |
| WolOS Proton missing or outdated | rewritten (atomic, launcher 0755) |
| Steam starts | Steam's own updater window, then sign-in |

## Steam to wine-proton

| Step | What happens |
|---|---|
| Steam runs `wolos-proton <verb> <game.exe>` | a Linux process: `/bin/sh` is the Linux root's bash, so the script is POSIX sh |
| `swap` | restores the FreeBSD `LD_PRELOAD`, `LD_LIBRARY_PATH`, `LIBGL_DRIVERS_PATH`, `PATH` that linux-steam-utils kept in `LSU_FBSD_*` (FreeBSD rtld stops on a Linux `LD_PRELOAD`) |
| `exec /usr/local/bin/wol-run --steam` | the kernel picks the ABI from the ELF brand: FreeBSD |
| Environment | drops `STEAM_RUNTIME*`, `SYSTEM_*`, `LD_AUDIT`, `PRESSURE_VESSEL_*`, `SRT_*`, `LSU_LINUX_*`; Linux entries leave `PATH`, `LD_*`, `VK_*` lists; Steam's Vulkan layers off; `WINEPREFIX=$STEAM_COMPAT_DATA_PATH/pfx` |
| Loader | wine-proton 10: `bin/wine64`, 32-bit games via `~/.i386-wine-pkg` or `bin-wow64`; 11: `bin/wine` (WoW64) |
| Sync | 10: esync (`WINEESYNC=1`); 11: wineserver (its fsync and ntsync are Linux-only); `futex_waitv` only helps Linux programs |
| DXVK, vkd3d-proton, dxvk-nvapi | from an installed Valve Proton of the same version (`files/lib/wine/{dxvk,vkd3d-proton,nvapi}`) or `WOL_{DXVK,VKD3D,NVAPI}_DIR`; copied into `system32`/`syswow64`, `WINEDLLOVERRIDES=...=n,b` |
| Vulkan | FreeBSD ICDs in `/usr/local/share/vulkan/icd.d` (Mesa anv/radv, NVIDIA) |
| GPU preset | `WOL_GPU_PRESET` or the hardware snapshot: all `WINEDEBUG=-all`; Mesa `MESA_VK_WSI_PRESENT_MODE=mailbox`; NVIDIA shader cache kept |

| Verb | Does |
|---|---|
| `waitforexitandrun` | new prefix: `wineboot -u`; copy DLLs; wait for the last `wineserver`; run (`PROTON_LOG=1`: `~/steam-<SteamGameId>.log`) |
| `run` | the same without the wait |
| `runinprefix` | run in the prefix, no DLL copies |
| `getcompatpath`, `getnativepath` | `winepath -w` / `-u`: prints only the path |
| `destroyprefix` | remove `pfx` |

Proton switches read: `PROTON_USE_WINED3D`, `PROTON_NO_D3D12`, `PROTON_ENABLE_NVAPI`, `PROTON_NO_ESYNC`, `PROTON_LOG`. Others belong to Valve's Python script and do nothing here.

## Limits

| Gap | Today |
|---|---|
| Games that need the Steam client API (Steamworks) | pick linux-steam-utils' "FreeBSD Wine (emulators/wine-proton)"; the doctor lists what it needs |
| Anti-cheat, VR, Steam Input | not supported by linux-steam-utils |
| NVIDIA Linux libraries | the doctor's fix; not installed by `wol hw` |

## Tests

| Where | Proves |
|---|---|
| `core` `games::*` | vdf and launcher text, install/repair, doctor parsers on real `vulkaninfo` output, every fix, client blockers, notices |
| `proxy` unit + `tests/steam_tool.rs` | verb dispatch, env table, DLL copies; the generated launcher runs `wol-run` against a fake wine with a Steam-like env |
| `controller` `tests/games.rs`, `tests/image_games.rs` | `wol games` in a temp HOME; the image ships every client setting, the launcher and the `.exe` handler |
| `image/vm/test/games.sh` | in the FreeBSD VM: doctor, a Windows program through `wolos-proton` from a Linux shell, headless Vulkan |
