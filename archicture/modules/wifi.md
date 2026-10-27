# Wi-Fi

One interface: `wlan0`. Shell, Control Center (`wol_core::controls`), Setup Assistant (`wol-login` `SETUP_WIFI`) and admind (`wifi-join`, `wifi-power`) all use it.

## Boot (`rc.d/wolos_wifi`, REQUIRE FILESYSTEMS wolos_hw, BEFORE netif)

`libexec/wolos/wifi`, every boot (a disk moves between machines):

| Step | Action |
|---|---|
| driver | loaded before this by rc.d `wolos_hw` (`wol hw boot`, archicture/modules/hardware.md) |
| device | first word of `sysctl net.wlan.devices`, waits up to 10 s only if a PCI class 0x0280 device exists |
| iface | `ifconfig wlan0 create wlandev <dev>` unless wlan0 exists |
| conf | `/etc/wpa_supplicant.conf` created 0600 with the `wol_core::wifi` header if missing; never replaced |
| network | `ifconfig_wlan0="WPA DHCP"` via sysrc if unset; netif then starts wpa_supplicant + dhclient |

Join: admind/wol-login upsert `network={}` into the conf, then `wpa_cli -i wlan0 reconfigure`. Power: `ifconfig wlan0 up|down`.

## Firmware (offline, `desktop/packages.txt`)

Mapping from fwget(8) `pci_network_*` (releng/15.1); names from the 15.1 kmods repo.

| Family | Packages |
|---|---|
| Intel iwlwifi | `wifi-firmware-iwlwifi-kmod-{7000,8000,9000,22000,ax210,bz,sc}`; ADL-P CNVi 8086:51f0 -> ax210 |
| Realtek rtw88 | `rtw8821c rtw8822b rtw8822c rtw8723d` |
| Realtek rtw89 | `rtw8851b rtw8852a rtw8852b rtw8852bt rtw8852c rtw8922a` |
| MediaTek mt76 | `mt7921` (`if_mt7921` + `mt76_core` built by `kernel/build.sh`) |

The firmware packages carry the tunables (`/boot/loader.conf.d`): rtw88/rtw89 `compat.linuxkpi.skb.mem_limit=1` (32-bit DMA, needed above 4 GB RAM), iwlwifi `22000 ax210 bz` 802.11n/ac + hardware crypto.

## What each card gets on FreeBSD 15.1 (man pages, status report 2026Q2)

| Card | Driver | Link |
|---|---|---|
| Intel, firmware families `22000 ax210 bz` | iwlwifi | 802.11n/ac (Wi-Fi 5 rates); no 802.11ax/be, no 6 GHz yet |
| Intel, firmware families `7000 8000 9000 sc` | iwlwifi | 802.11a/b/g (54 Mbit/s) |
| Realtek RTL88xxCE/BE/DE, RTL885x/8922 | rtw88, rtw89 | 802.11a/b/g; 8821CE/8822BE may fail to associate (rtw88(4) BUGS) |
| MediaTek MT7921/MT7922 | if_mt7921 (WolKernel) | 802.11a/b/g |
| MediaTek MT7925 | - | unsupported: driver unstable on 15.1, not built |
| Qualcomm ath10k (QCA6174/9377/988x) | - | unsupported: not in the 15.1 build, port unfinished |
| Qualcomm ath11k/ath12k (QCA6390, WCN6855/7850) | - | unsupported: LinuxKPI has no MHI/QRTR transport |
| Broadcom | - | unsupported |

Unsupported cards get no firmware and say why in `wol hw` and Settings (no promise the image cannot keep).
Tests: `crates/controller/tests/image_wifi.rs`, `crates/core/src/hardware/tests.rs`. Unverified on hardware: see CHANGELOG 0.11.1.
