# 0056 - wol-tor: the Tor layer of the OS (Atlas tabs, chosen apps, the whole machine)
Status: proposed (lead order 2026-11-04 22:05Z: "wol-tor need to be the best layer of tor possible in security and etc (and people can activate on machine too in general and use anywhere and the Atlas will use it)"). Draft by ATLAS-TOR. The CTO accepts it when the rulesets load on the VM and journeys J-T8 to J-T11 pass. Number 0056 because 0054 (post-quantum keyring) and 0055 (system TLS policy) are taken by other branches. [ADR 0053](0053-atlas-private-and-tor-tabs.md) is the Atlas tab part and stays valid.

## Context
ADR 0053 made `wol-tor` the thing that starts one tor per Tor-tab group. The order is wider: Tor is a layer of the operating system. A person turns it on for Atlas tabs, for one app, or for the whole machine, and any program can use it ("use anywhere"). What must stay true in all three: the program that talks to the network has no way to the clear net except through tor, and that is a property of the kernel's rules, not of a setting inside the program.

What exists to build on (read in the tree, 2026-11-04): the crate `crates/wol-tor` (torrc builder, SOCKS5 relay with per-site isolation, control port, bootstrap cache, the namespace check), the unit helpers, and `wol-admind` as the place privileged work already goes. Tor's own transparent proxy (`TransPort`, `DNSPort`, `AutomapHostsOnResolve`) does the original-destination lookup itself, so no code of ours needs `unsafe` or raw sockets.

## Decision

### D1. Three scopes, three switches, one tor design
| Scope | Switch (default) | Privilege | How traffic is forced | Kill switch |
|---|---|---|---|---|
| Atlas tabs | `atlas` (on: it only enables the menu item) | none (user units) | ADR 0053: browser in a namespace with only loopback, relay to tor's Unix socket | the namespace has no route |
| One app | `app NAME` (off) | `wol-admind` | the app runs in its own namespace on the `wol-tor0` bridge; the host redirects what arrives there to tor | the namespace's own drop rules, then the host's redirect-or-drop, then no forwarding |
| Whole machine | `system` (off) | `wol-admind` | nftables `output` hook: TCP to `TransPort`, DNS to `DNSPort`, everything else dropped | the output filter chain's policy is `drop`; with the redirect rules gone nothing leaves either |

Atlas uses the layer by default: when the machine-wide Tor is on, an Atlas Tor tab group does not start a tor of its own (that tor, running as the user, would be redirected into the machine's tor: Tor over Tor, a second hop with no benefit). Its relay connects to the machine tor's SOCKS socket (`wol-tor open` passes `--tor-socks`), the browser stays in its namespace with no route, and isolation per site works as before because that SocksPort has `IsolateSOCKSAuth`. The group owns nothing of that tor (no `TAKEOWNERSHIP`, no watcher). When the machine scope is off, the group has its own tor as in ADR 0053. `cache-run` does nothing while the machine scope is on. "Keep Tor ready" ([ADR 0053](0053-atlas-private-and-tor-tabs.md) D4) is the fourth switch, default off, and is not a boot-time scope: it is a per-user timer.

The state is **one root-owned file**, `/etc/wol-tor/switches` (see D8); there is no per-user copy. `wol-tor switches show|set|app` reads and (as root) changes it, and parsing is such that junk can only turn a scope off. `wol_tor::mode::plan(applied, wanted)` returns the ordered actions `wol-tor apply` runs; the order is the safety property and is tested:
- turning on: system drop rules first; then the app bridge (tor listens on its address, so it must exist before tor starts); then tor; then namespaces. A redirect to a tor that is not up yet fails closed; the other order leaks while it boots;
- turning off: namespaces, then tor, then the bridge, then the rules;
- tor dying never unloads rules: the rules stay and the machine is offline until tor is back or the person turns the scope off.

### D2. The rulesets are data and are tested as such
`wol_tor::transparent::Ruleset` is a list of chains and rules. `render()` writes the `nft -f` text and `verdict(packet)` runs a packet through the same rules, so the kill-switch tests exercise what is loaded:
- over a grid of users, protocols, destinations and ports, no packet of a user other than tor leaves the machine unredirected (system mode);
- with the redirect (`nat`) chains removed, every such packet is dropped (fail closed);
- UDP other than DNS, ICMP and all IPv6 to the internet are dropped (Tor carries TCP; IPv4 TCP is what the `TransPort` serves);
- app traffic is never forwarded, even with the redirect gone and `ip_forward` on;
- the text is checked with `nft -c` (the real parser) where the machine allows it. It ran green on the build host with `sudo`; the unprivileged form is skipped [V on a clean runner].

System mode details: tor's own user is exempt (looked up as user `tor`); `127.0.0.53` DNS (the systemd resolver stub) and every UDP/TCP port 53 go to the `DNSPort`, so there is no clear-net DNS fallback; DoT/DoH ride the `TransPort`; loopback is left alone; a LAN allow list (empty by default; `switches set lan on` adds the private ranges, with the system rules) stays direct, except tor's virtual range 10.192.0.0/10 (inside 10/8), which is redirected before the LAN rule so a `.onion` address is never sent direct, with the cost stated in Settings ("devices on your network can see this machine is not using Tor for them"). The update mirror goes over Tor like everything else; there is no exception to make it faster.

### D3. Per-app: a namespace per app, a bridge, two independent drop layers
- `wol-admind` runs `transparent::gateway_up_steps()` once (host rules loaded first, then the `wol-tor0` bridge at 10.191.0.1/24, forwarding off, IPv6 off) and `netns_steps(app)` per app: namespace, resolv.conf pointing at the gateway, IPv6 off, the namespace's own output rules (`policy drop`, TCP and DNS to the gateway only) **before** the veth exists, then veth, address, and a single default route via the gateway.
- Each app has its own address (the bridge is 10.191.0.0/24, chosen outside tor's virtual range 10.192.0.0/10 so no `.onion` mapping can ever land on the gateway or an app), and tor's `TransPort ... IsolateClientAddr` gives each app its own circuits. The address index is stored in the switch file and kept, so a running app never changes address when others come and go. Bridge ports are **isolated** (`bridge link set dev wtvN isolated on`), so one app cannot reach another app's address.
- **Names never leave through the host's resolver.** `ip netns exec` bind-mounts `/etc/netns/<ns>/*` over `/etc`, and the namespace gets its own `resolv.conf` (the gateway only) and `nsswitch.conf` with `hosts: files dns`. The system file says `resolve [!UNAVAIL=return]`, and a unix socket is not bound by a network namespace: without this file the app would ask systemd-resolved over `/run/systemd/resolve`, whose upstream queries leave outside tor in app mode. `.onion` names reach the DNSPort the same way (gateway UDP 53, redirected by the host). `app-exec` also empties `/run/systemd/resolve` and `/run/avahi-daemon` in that mount namespace. Known gap: an app that calls `org.freedesktop.resolve1` over the system D-Bus still reaches the host resolver; blocking the system bus would break portals and power management, so it is documented, not blocked [V].
- The app starts with `transparent::join_argv`: `ip netns exec`, then `wol-tor app-exec` (inside the namespace's private mount namespace it puts empty tmpfs over the host resolvers' socket directories), then `setpriv` with the user's ids, `--no-new-privs` and every capability dropped, then the command. The systemd scope that `spawn_app` already creates for the memory budget (ADR 0026) wraps this argv; that code belongs to MEMSAVER-X and was not touched. Question posted in #plan.
- The host side (`app_host_ruleset`): redirect on `prerouting` for arrivals on `wol-tor0`; an `input` chain that accepts only the `TransPort` and the `DNSPort` from the bridge and drops the rest (an app cannot reach sshd or any other host service); a `forward` chain that drops everything from the bridge, so a host with `ip_forward=1` still routes nothing.
- The gate chain accepts the TransPort and DNSPort only at the gateway address, so a port number alone never opens another service of the host (the mDNS responder on 5353, for one).
- Limits: a program that needs UDP (VoIP, games, QUIC without TCP fallback) does not work in a Tor scope; that is the point. IPv6-only destinations are unreachable.

### D4. The gateway tor
- One system unit `wol-tor-gateway.service` (`wol-tor gateway unit` prints it): tor as its own user, the sandbox of every tor unit (no new privileges, `ProtectSystem=strict`, `MemoryDenyWriteExecute`, seccomp `@system-service`, no capabilities, no core files, no swap, `StandardOutput=null`), `RuntimeDirectory` on tmpfs. It listens on `127.0.0.1` (system rules) and on the bridge address (app namespaces), both when both are on; changing that is `ReloadGateway`.
- torrc additions (`wol-tor gateway torrc`): `SocksPort unix:... IsolateSOCKSAuth WorldWritable` while the machine scope is on (Atlas groups use it, D1), `TransPort ... IsolateClientAddr IsolateClientProtocol`, `DNSPort`, `AutomapHostsOnResolve 1`, `VirtualAddrNetworkIPv4 10.192.0.0/10` (so `.onion` names resolve to a virtual address and connect through the `TransPort`), plus everything of ADR 0053 D2 and D4: `ClientOnly`, `SafeLogging`, `Sandbox 1` when no pluggable transport is configured [V: `Sandbox 1` together with `TransPort` on 0.4.8/0.4.9], conflux, circuit padding, learned circuit-build timeout, bridges (obfs4, webtunnel, Snowflake) from `/etc/wol-tor/bridges`, checked line by line before they reach tor.
- **State is kept.** The gateway's DataDirectory is `/var/lib/wol-tor/gateway` (`StateDirectory=`), not the runtime tmpfs: guards, the consensus and the circuit-build timeout survive restarts and reboots (a machine-wide tor with a new guard at each boot is worse), while the sockets and the cookie stay in the runtime directory. An Atlas group still has its tmpfs data (nothing kept, ADR 0053).
- **The binary is ours and checked.** tor comes from our own package (recipe note in the Atlas plan directory), pinned by hash and signed with the repository's key like every package. At start `wol-tor` refuses a tor that is not owned by root or is writable by group or others (`tor_binary_trusted`); `WOL_TOR_BIN` is a developer override and skips only that check.
- Control is a Unix socket with cookie authentication; the gateway's cookie is readable by its user only.

### D5. Onion services and Atlas
`.onion` works in all three scopes (Atlas tabs: by name through the relay; apps and machine: through `AutomapHostsOnResolve`). Atlas's Onion-Location hook is patch 0009 of ADR 0053 (offer, never redirect), still a draft.

### D6. Settings, status and `wol doctor tor`
- **Settings > Privacy & Security > Tor**, copy posted in #plan for the shell lane (it is not in this crate): "Tor for Atlas tabs" (on), "Tor for chosen apps" (list with Add app), "Tor for this whole computer" (off), "Keep Tor ready" (off), "Bridges". Each active scope has a status row: "Connected to Tor, 100%, exit in <country>" or "Starting, 45%" or "Not connected: your traffic is blocked" (the honest state when tor is down in a Tor scope).
- `wol-tor status` prints bootstrap and the exit country (the exit of the first built general circuit, `ip-to-country` from tor's geoip). `wol-tor new-identity` sends `NEWNYM`.
- `wol-tor doctor` (to be reachable as `wol doctor tor`) measures and judges with `doctor::evaluate`:
  - bootstrap percentage;
  - leak test: asks the Tor Project's check service **over tor's SOCKS socket only** (never a direct connection) and reads `IsTor`;
  - DNS leak: a canary `.onion` name is queried to an address nothing routes; only a tor `DNSPort` can answer, and the answer must be in `10.192.0.0/10`; run only when the machine scope is on;
  - clock skew against the consensus (tor refuses a consensus more than an hour off);
  - the drop rules are loaded (needs administrator rights; otherwise "not checked");
  - anything not measured is "not checked", never a pass; exit code 1 only for a FAIL.

**Why not a `tor-group` uid exempted in the rules** (the other way to avoid Tor over Tor): the group tors are systemd *user* units and run as the person; a different uid would need system-level group units and a socket the browser can reach across users. Sharing the machine tor removes the second tor altogether, keeps per-site isolation (SOCKS credentials), and needs no exemption beyond `tor` itself.

### D8. Boot and persistence (one switch file, one unit)
- `/etc/wol-tor/switches` (root-owned, mode 0644) is the only switch store for the scopes. `wol-tor-rules.service` (`wol-tor gateway rules-unit`) runs `wol-tor apply` at boot: the plan from nothing (`/run/wol-tor/applied` is on tmpfs, so absent after a boot) to the file's contents. It is `After=nftables.service` (the OS firewall's table exists first) and `Before=network-pre.target network-online.target` (no packet is sent between). `apply` stops at the first failing step, says which, and leaves the applied state unchanged.
- A change at run time is `wol-tor switches set ...` (root) then `wol-tor apply`; the daemon does both. `wol-tor apply --dry-run` prints every step.
- A scope that is on at boot is therefore on before the network is: system rules load before DHCP, so nothing leaves between boot and tor being up; the app namespaces and the gateway come up in the plan's order.

### D9. Coexistence with the OS firewall
The OS firewall (`/etc/wolos/nftables.conf`, `nftables.service`) and the Tor scopes share one kernel rule set and own different tables: `inet wolos` for the firewall, `inet wol_tor` and `inet wol_tor_apps` for Tor. Neither may delete the other's.
- The firewall file used `flush ruleset` as its first line, at boot and at every reload (`FIREWALL_RELOAD_ARGV`). That deleted the Tor tables and, in system mode, removed the kill switch and the redirect at once: fail-open. It now starts with `destroy table inet wolos`, which replaces only its own table in the same transaction. A test fails if the file, or any shipped nft file, contains `flush ruleset`, and if the firewall names a Tor table.
- Each Tor ruleset starts with `add table` + `delete table` of its own table only, so loading it twice replaces it.
- A second guard: `nftables.service` gets a drop-in (`wol-tor gateway nft-dropin`, file name `wolos.tor.conf`, sorting after the OS drop-in that resets `ExecReload`) that runs `wol-tor apply --reload-rules` after a start or a reload, so even an administrator's own `nft flush ruleset` followed by a firewall restart brings the Tor rules back. An administrator's `flush ruleset` alone is not defended against (root can do anything); the doctor reports the missing rules (J-T11).
- Limits stated: a process with `CAP_NET_RAW` (root, or one granted it) can send through `AF_PACKET`, which bypasses netfilter's output hooks. The threat model of the scopes is unprivileged apps, which have no such capability (the app start command drops them all); a root process can also simply remove the rules.

### D7. What is not in this ADR
Replacing the user's DNS settings for non-Tor traffic; Tor for other users' sessions separately from the machine (the machine switch is for all users); a captive-portal helper (the person turns the scope off to log in); Arti (see ADR 0053 D8).

## Journeys (VM)
| Id | What | Proves |
|---|---|---|
| J-T8 | Machine scope on: `curl` to a public IP and to a name, a UDP DNS query to 1.1.1.1, `ping`, an IPv6 connect; packet capture on the host bridge | only tor's own traffic leaves; DNS never reaches the clear net |
| J-T9 | Kill tor (`systemctl kill wol-tor-gateway`) with the scope on | every connection fails or hangs; capture shows no clear-net packet; tor restarts and traffic resumes |
| J-T10 | Two apps in Tor scopes fetch the same check page | different exits (`IsolateClientAddr`); an app cannot reach the host's sshd |
| J-T11 | `wol-tor doctor` in each scope; then `nft flush ruleset` (as root); then `systemctl reload nftables` | all ok; after the flush the kill-switch finding is FAIL; after the firewall reload the Tor rules are back (drop-in) and the finding is ok; and a plain firewall reload (no flush) never removed them |

## Consequences
- One crate, three scopes, the same tests: the rulesets and plans are unit-tested without privilege; the VM journeys prove the kernel side.
- A Tor scope is not Tor Browser: browser fingerprints are the app's own. The Settings text says so.
- A Tor scope that is on and has no tor (or no network) means no connectivity for what it covers. That is the intended failure.
- New work for others: `wol-admind` verbs that call `wol_tor::mode::plan` and `transparent::*` (and install `wol-tor-gateway.service`); the Settings pane; `wol doctor tor` registering `wol-tor doctor`; packages `tor`, `lyrebird`, `snowflake-client`, `nftables`, `iproute2`, `util-linux` (`setpriv`) in the Tor module.
