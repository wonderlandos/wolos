# Wi-Fi and the network (ADR 0016)

NetworkManager runs the network and iwd does Wi-Fi (`files/etc/NetworkManager/conf.d/wolos-iwd.conf`,
`wifi.backend=iwd`; both enabled in `50-wolos.preset`). Everything below goes through `nmcli`; the
Wi-Fi interface is whatever NetworkManager lists (never a guessed `wlan0`). The old
wpa_supplicant.conf writer is gone (QA-NET-20).

## Who runs what

| What | Where | As |
|---|---|---|
| Radio on/off, scan, join, Disconnect, hotspot | `wol_core::controls` (`set_wifi`, `wifi_networks`, `wifi_join_hidden`, `wifi_disconnect`, `set_hotspot`) | the person at the computer: NetworkManager's polkit rules allow it, no administrator password |
| Join for a helper app, Forget | wol-admind `wifi-join`, `wifi-forget` (`wol_core::admin`) | root, after the Authenticate sheet |
| Setup Assistant join | `wol-login` `SETUP_WIFI` | root |

Surfaces: Settings > Wi-Fi (`shell ui/settings/wifi.rs`), Control Center's Wi-Fi page (`ui/control_center/pages.rs`),
Settings > Network (`ui/settings/network.rs`), Settings > General > Sharing (`ui/settings/sharing.rs`),
`wol wifi …` and `wol network` (`controller/src/verbs.rs`), wold's `network.info` for apps (`api/src/facts.rs`).

## Joining

A password join is two steps, the password on stdin, never in argv (NET2-1):

1. `nmcli connection add type wifi con-name <ssid> ifname * ssid <ssid> wifi-sec.key-mgmt wpa-psk|sae`
   (`connection modify id <ssid> …` when a profile of that name is saved; `802-11-wireless.hidden yes`
   for a hidden network, QA-NET-10). `sae` only when every access point of that name is WPA3-only.
2. `nmcli connection up id <ssid> passwd-file /dev/stdin` with `802-11-wireless-security.psk:<password>`.
   A failed bring-up deletes a profile the join made.

An open network: `nmcli dev wifi connect <ssid> [hidden yes]`. A WPA passphrase is 8 to 63
characters, or the key itself as 64 hex digits (QA-NET-18).

Why a join failed comes from nmcli's stderr (`wol_core::wifi::classify_join_error`, QA-NET-8):
"Secrets were required" is a wrong password, "No Wi-Fi device" or NetworkManager not running is
no Wi-Fi, "The Wi-Fi network could not be found" is out of range, a DHCP or 90 s timeout is no
address. `wol wifi join` and `wol wifi on` exit non-zero with that cause (QA-PWR-21). The runner
waits 100 s (nmcli's own activation wait is 90 s).

802.1X (Enterprise) networks are listed with their kind (`wol_core::wifi::Security`); WolOS does not
join them yet and says so instead of asking for a password (QA-NET-9).

Lists use `--rescan auto` (NetworkManager scans when its list is older than 30 s) and Settings has
Look Again (`nmcli dev wifi rescan`, QA-NET-16). Settings lists every network in range; Control
Center lists 8 and a row with how many more, which opens Settings (QA-NET-15). nmcli's terse
escapes (`\:` and `\\`) are undone (`wol_core::wifi::unescape`, QA-NET-19).

Saved networks are read by SSID (`netinfo::known_ssids`: one profile UUID at a time), so "Home 1"
is the network Home; Forget deletes every profile of that SSID by UUID (QA-NET-17, QA-SET-16).
Disconnect is `nmcli dev disconnect <iface>`; the profile stays.

## Online, limited, portal, metered

One answer for everyone (QA-NET-12): `wol_core::netinfo::NetInfo` reads the default route (IPv4,
else IPv6), NetworkManager's connectivity (`nmcli -t -g CONNECTIVITY general status`: full, limited,
portal, none) and its Metered property (yes or guess-yes). wold gives apps
`{online, limited, portal, metered, interfaces}`; `wol network` adds `vpn`; the tray shows a
no-internet glyph for limited and portal, and the shell's `System::connectivity()` feeds the menu
bar's "Sign In to This Network" notice, posted once per portal (NET2-3). The probe is configured in
`files/usr/lib/NetworkManager/conf.d/wolos-connectivity.conf`: a plain-http GET of
`http://ping.archlinux.org/nm-check.txt` every 5 minutes (WolOS has no endpoint of its own yet).

A tunnel (WireGuard, OpenVPN tun: `link_type none`) is a VPN; it is up while it carries traffic,
and each link shows its own DNS servers (`nmcli … device show`, QA-NET-28).
`nmcli connection import type wireguard file x.conf` works with the shipped kernel; there is no
import UI yet (lead call, QA-NET-21).

## Names and the firewall

`.local` names: nss-mdns before `resolve` in /etc/nsswitch.conf (wolos-system's
`/usr/lib/wolos/nsswitch-mdns`); avahi owns mDNS, so systemd-resolved keeps `MulticastDNS=no` and
every other name goes through it (DNS-over-TLS and DNSSEC set there apply to all programs). No
compiled-in fallback resolvers (`FallbackDNS=`; QA-NET-4).

Settings that open a port write a rule file in `/etc/wolos/nftables.d/` and load the ruleset again
with `nft -f /etc/wolos/nftables.conf` while the firewall is on: Remote Login (`ssh.conf`, port 22)
and AirPlay Receiver (`airplay.conf`: tcp 5000 and 7000, udp 319-320 and 6000-6009, local network
only; QA-NET-5).

## Internet Sharing (hotspot)

Settings > General > Sharing > Internet Sharing and `wol wifi hotspot on [name]` (password on
stdin) / `off` / `wol wifi hotspot`: a profile "WolOS Hotspot" (access point, WPA2-PSK,
`ipv4.method shared`: NetworkManager runs its own dnsmasq for addresses and shares the link that
has the internet), brought up with the password on stdin; it never connects by itself (NET2-2).

## On the live medium

The live account has no password, so Sharing greys the administrator rows and says why, and
Remote Login is not offered (QA-NET-6, NET2-8). Joining, Disconnect and the hotspot work there.

Tests: `crates/core/src/{wifi,controls,netinfo,sharing,admin}.rs`, `crates/core/tests/admin_plans.rs`,
`crates/api/src/facts.rs`, `crates/controller/tests/{dispatch,image_network}.rs`,
`crates/shell/src/ui/settings/{wifi,network,sharing,machine}.rs`, `ui/control_center/pages.rs`;
wolvm journeys `qa100-net-*`, `qa100-net2-*`, `qa100-b7-wifi-hotspot-hidden`.
