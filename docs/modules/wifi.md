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
Settings > Network (`ui/settings/network.rs`, its VPN section `ui/settings/network_vpn.rs`, its subpages
`ui/settings/ip_settings.rs`, `ui/settings/proxy.rs`, `ui/settings/dns_over_tls.rs`), Settings > General > Sharing
(`ui/settings/sharing.rs`), `wol wifi …` and `wol network` (`controller/src/verbs.rs`), `wol network ip`, `wol vpn`,
`wol proxy`, `wol dns`, `wol airplane` (`controller/src/netverbs.rs`), wold's `network.info` for apps (`api/src/facts.rs`).

| What (NET-3) | Where | As |
|---|---|---|
| A link's IPv4 address (Automatic or Manual), VPN import, connect, disconnect, forget, airplane mode | `wol_core::controls` (`controls/net.rs`; pure parts in `netconf.rs`, `vpn.rs`) | the person at the computer: NetworkManager's polkit rules, rfkill through the session's `uaccess` on /dev/rfkill |
| System proxy, DNS over TLS | wol-admind `proxy`, `dns-over-tls` (`wol_core::admin`) | root, after the Authenticate sheet with the password typed every time |

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

## IP Settings: a static address (QA-LIVE3-14)

Settings > Network > a link > IP settings > Configure… (the IP Settings page) and `wol network ip <link>
[auto | manual <address/prefix> [router] [dns…]]` change the IPv4 method of the profile active on that
link (`nmcli -g GENERAL.CON-UUID device show`), by UUID: `nmcli connection modify uuid <u> ipv4.method
manual ipv4.addresses … ipv4.gateway … ipv4.dns …` (Automatic clears the three), then `nmcli connection
up uuid <u>`. Checked before anything runs (`wol_core::netconf::validate_ipv4`): an address with its
prefix that is not the subnet's own number or broadcast (prefixes 31 and 32 have none), a router inside
that subnet, DNS servers that are IP addresses.

A Manual address then gets a 15-second trial: once a second the link must be connected (device state
100) and reach at least as far as before (`nmcli networking connectivity check`: full before and none
after is lost). When it does not, the profile goes back to what it was (its earlier Manual address, else Automatic) by itself and Settings says
so; `wol network ip` exits non-zero. Only IPv4 is typed in; IPv6 stays automatic.

## VPN

Settings > Network > VPN lists the WireGuard and VPN profiles (`nmcli -t -f NAME,UUID,TYPE,ACTIVE
connection show`; a `vpn` profile's plugin from `vpn.service-type`), each with Connect or Disconnect and
Forget; Add VPN picks a file, and `wol vpn import <file>` does the same. A WireGuard `.conf` is imported
with `nmcli connection import type wireguard file`, from a private copy named after the file as an
interface name (at most 15 letters, digits, `-`, `_`: NetworkManager names the tunnel after the file).
Its private key then moves into the login keyring (`secret-tool store`, libsecret) and out of the
profile (`wireguard.private-key-flags 2`, not saved; `connection.autoconnect no`); Connect looks it up
and hands it to `nmcli connection up uuid <u> passwd-file /dev/stdin`, never in argv. When no keyring
answers, the key stays in NetworkManager's root-only profile file and Connect works without it.
An OpenVPN `.ovpn` needs NetworkManager's OpenVPN plugin (networkmanager-openvpn, with openvpn): the image
ships both through wolos-system's depends (wolos-pkgbuilds `checks/tests/recipe_network_vpn.rs`); where the plugin
is missing, import says so. A password OpenVPN asks for is typed in Settings (or given
on stdin to `wol vpn connect`), sent the same way and remembered in the keyring once it worked. Forget
deletes the profile and its keyring entries.

## Proxy

Settings > Network > Proxy and `wol proxy [off | auto <url> [no-proxy=…] | manual http=host:port
https=… socks=… no-proxy=…]`: Off, Automatic (a PAC file) or Manual, with the hosts reached directly.
wol-admind `proxy` writes `/etc/wolos/proxy.conf` (what Settings reads back) and the login environment
`/etc/profile.d/wolos-proxy.sh` (`http_proxy`, `https_proxy`, `all_proxy` as `socks5h://`, `no_proxy`
with localhost added, both spellings; a PAC file has no variable). Then, as the person, the desktop's
keys (`org.gnome.system.proxy`: mode, autoconfig-url, ignore-hosts, http, https, socks) and
NetworkManager's `proxy.method auto` with the PAC address on the profile of the default route (`none`
otherwise). Every value is checked twice (Settings, then wol-admind) to hold no quote, `$` or space: it
is written into a shell file.

## DNS over TLS

Settings > Network > DNS over TLS and `wol dns tls <off|opportunistic|strict> [server…]`: wol-admind
`dns-over-tls` writes `/etc/systemd/resolved.conf.d/50-wolos-dns-over-tls.conf` (`DNSOverTLS=no`,
`opportunistic` or `yes`, and `DNS=` with the servers, `9.9.9.9#dns.quad9.net`: the name after `#` is
the one the server's certificate must have) and restarts systemd-resolved. Always (strict) is refused
without a server of the computer's own: a home router does not speak TLS and every lookup would fail
(network-r2: 347 tries on port 853, no answer). The shipped `/usr/lib/systemd/resolved.conf.d/wolos.conf`
sets neither key, so the drop-in wins (`controller/tests/image_network.rs`). Because every lookup goes
through resolved (see "Names and the firewall" below), the setting applies to every program.

## Airplane mode

Control Center's Airplane Mode tile (first in the grid on a computer with a radio), Settings > Network >
Airplane Mode and `wol airplane on|off`: on remembers which radio types were on
(`/var/lib/wolos/airplane/radios`, the computer's list that every account shares, one rfkill type per
line; kept when it was on already; without that folder, `$XDG_STATE_HOME/wolos/airplane`) and runs
`rfkill block all` (Wi-Fi, Bluetooth, mobile broadband); off unblocks only those types (every type
without the list) and removes the list. Airplane mode reads as on while the list exists and a radio is
still blocked, so `rfkill unblock all` in a terminal ends it. The menu bar shows the plane
(`airplane-mode-symbolic`) unless a wired link carries the traffic.

## Names and the firewall

`.local` names: nss-mdns before `resolve` in /etc/nsswitch.conf (wolos-system's
`/usr/lib/wolos/nsswitch-mdns`); avahi owns mDNS, so systemd-resolved keeps `MulticastDNS=no` and
every other name goes through it (DNS-over-TLS and DNSSEC set there apply to all programs). No
compiled-in fallback resolvers (`FallbackDNS=`; QA-NET-4).

Why not `MulticastDNS=resolve` in systemd-resolved instead (decided 2026-11-04): avahi already
answers and advertises on udp 5353 for printers, AirPlay and the Sharing name; resolved's mDNS on
top would be a second stack on the same port, and the two make each other unreliable. With
nss-mdns asked first only for `.local`, every other lookup still reaches resolved (`resolve
[!UNAVAIL=return]` stays in the hosts line), so DNS-over-TLS and DNSSEC settings keep applying to
every program (network-r2). Contract tests: `controller/tests/image_network.rs`
(`nsswitch_has_mdns_or_resolved_off`, `resolver_has_no_hidden_fallback`) and
`image_systemd.rs::one_mdns_stack`.

Settings that open a port write a rule file in `/etc/wolos/nftables.d/` and load the ruleset again
with `nft -f /etc/wolos/nftables.conf` while the firewall is on: Remote Login (`ssh.conf`, port 22)
and AirPlay Receiver (`airplay.conf`: tcp 5000 and 7000, udp 319-320 and 6000-6009, local network
only; QA-NET-5). Both ship closed and are in wolos-system's `backup=`, so an upgrade keeps what
the setting wrote (wolos-pkgbuilds `checks/tests/recipe_firewall_files.rs`).

## Internet Sharing (hotspot)

Settings > General > Sharing > Internet Sharing and `wol wifi hotspot on [name]` (password on
stdin) / `off` / `wol wifi hotspot`: a profile "WolOS Hotspot" (access point, WPA2-PSK,
`ipv4.method shared`: NetworkManager runs its own dnsmasq for addresses and shares the link that
has the internet), brought up with the password on stdin; it never connects by itself (NET2-2).

## On the live medium

The live account has no password, so Sharing greys the administrator rows and says why, and
Remote Login is not offered (QA-NET-6, NET2-8). Joining, Disconnect and the hotspot work there.

Tests: `crates/core/src/{wifi,controls,netinfo,sharing,admin,netconf,vpn}.rs`, `crates/core/src/controls/net.rs`,
`crates/controller/tests/net3.rs`, `crates/core/tests/admin_plans.rs`,
`crates/api/src/facts.rs`, `crates/controller/tests/{dispatch,image_network}.rs`,
`crates/shell/src/ui/settings/{wifi,network,network_vpn,ip_settings,proxy,dns_over_tls,sharing,machine}.rs`,
`ui/control_center/pages.rs`, `status.rs`; goldens `settings-network-all-wide`, `settings-ipsettings-*`, `settings-proxy-*`,
`settings-dnsovertls-*`, `controlcenter-airplane`; wolvm journeys `qa100-net-*`, `qa100-net2-*`,
`qa100-b7-wifi-hotspot-hidden`, `qa100-net3-*`.
