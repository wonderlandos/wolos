# WolOS architecture map
Base: Arch Linux snapshot (ADR 0016). Compositor: Hyprland (ADR 0001).
Layers: `kernel/` -> `image/` (installs packages + `desktop/` overlay + `design/` assets).
Rust userland: one Cargo workspace in `crates/` (ADR 0002). Rules enforced by `crates/shared/tests/architecture.rs`.
Rule: prefer upstream ports + config over new code.

| Path | Content |
|---|---|
| `adr/` | decisions |
| `modules/system-api.md` | System API topics + invariants |
| `modules/abi.md` | libwol: the app API (`wol.h`, `libwol.so.1`), ABI rules, planned families, Lua widgets (ADR 0020) |
| `modules/intel-graphics.md` | i915 firmware, tunables, VA-API/Vulkan stack |
| `modules/hardware.md` | hardware catalogue, `wol hw`, `wolos-hw.service`, shipped firmware, 15.1 gaps |
| `modules/ux-journeys.md` | 13 user journeys with verdicts and owners; error copy rules (`shell::copy`, lint) |
| `modules/automation.md` | helpers (`wol mcp`, scripts): policy, approval sheet wire, audit log |
| `modules/userland.md` | Dock apps and app-ids, default apps, zsh, sudo, keyring, printing, USB sticks, size |
| `modules/memory-bandwidth.md` | iGPU bandwidth paradigms, top 5, measurement (ADR 0007) |
| `adr/0009-live-iso.md` | live ISO: desktop on cd9660, memory overlays |
| `modules/widgets.md` | desktop widgets, Edit Widgets, building a widget package, `wol widgets` (ADR 0010) |
| `modules/packages.md` | `.wol` format, keys and trust, `wol install/remove/list/search`, index, factory (ADR 0011) |
| `modules/games.md` | native Steam and Proton, `wol games`, `.exe` double-click on wine or umu |
| `modules/power.md` | power policy per power source and mode, sleep and lock, battery alerts, `wol power` (ADR 0014) |
| `modules/security.md` | firewall, hardening, daily update check, malware check, encrypted homes, app permissions, what runs as root (ADR 0015, 0024) |
| `modules/boot.md` | quiet boot, ESP layout and firmware entry, GRUB/rEFInd, Secure Boot, install safety checks |
