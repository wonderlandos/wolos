# Wi-Fi

One interface: `wlan0`. Shell, Control Center (`wol_core::controls`), Setup Assistant (`wol-login` `SETUP_WIFI`) and admind (`wifi-join`, `wifi-power`) all use it.

The Arch image ships NetworkManager (enabled in `50-wolos.preset`) and iwd (`image/arch/packages.x86_64`); the kernel and `linux-firmware` bring the drivers and firmware (ADR 0016).

## Open migration

`wol_core::wifi` and admind still drive wpa_supplicant (`/etc/wpa_supplicant.conf` upsert, `wpa_cli -i wlan0 reconfigure`, `scan_results`, `status`), which is what the FreeBSD base used. NetworkManager owns the interface on the image, so join, scan and power must move to `nmcli` (or iwd's D-Bus API) with the same `wifi-join` / `wifi-power` admind verbs. Until then these paths do not apply on a booted Arch image.

Tests: `crates/core/src/wifi.rs`, `crates/core/src/controls.rs`, `crates/core/src/admin.rs`.
