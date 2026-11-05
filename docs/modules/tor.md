# Tor: the OS layer, `wol-tor` (ADR 0056)

Concept: Tor is a layer of the system. You choose where it applies, Atlas tabs, one app, or this whole computer, and where it applies the program has no way to the open internet except through Tor. Why this way: a setting inside a program can be bypassed by one bug (browsers have leaked DNS, DevTools and WebTransport traffic around their own Tor mode); a rule in the kernel cannot. How: the sections below. Decisions: [ADR 0053](../adr/0053-atlas-private-and-tor-tabs.md) (Atlas tabs) and [ADR 0056](../adr/0056-wol-tor-system-tor-layer.md) (apps and machine).

## What you can do

| Command | Does |
|---|---|
| `wol-tor open [URL]` | Open a Tor tab (starts the group's tor and Atlas in the isolated namespace) |
| `wol-tor switches show` | Show the switches: `atlas` (on), `system` (off), `lan` (off), apps, `keep-ready` (off) |
| `wol-tor switches set system on` / `off` (as administrator) | Tor for the whole computer; writes the one root-owned switch file and prints the plan |
| `wol-tor switches app add NAME` / `remove NAME` | Tor for one app |
| `wol-tor apply [--dry-run]` | Runs the plan from what is applied to the switch file; run at boot by `wol-tor-rules.service`, after the firewall and before the network |
| `wol-tor keep-ready on` / `off` | Keep Tor's directory documents fresh so a Tor tab starts quickly (see below) |
| `wol-tor bridges set LINE` / `show` / `clear` | Bridges (obfs4, webtunnel, snowflake) when Tor is blocked |
| `wol-tor status` | Bootstrap progress and the country of the exit |
| `wol-tor new-identity` | New circuits for new connections |
| `wol-tor doctor` | Bootstrap, leak test over Tor, DNS leak, clock, kill switch; "not checked" for what could not be measured |
| `wol-tor gateway rules system\|apps\|ns [--lan]`, `gateway steps ...`, `gateway join ...`, `gateway unit`, `gateway torrc` | Dry runs: print exactly what the privileged daemon would load or run |

## How it keeps you safe

- Atlas tabs: the browser runs in a namespace that has only loopback. It reaches Tor through a relay on a Unix socket and nothing else. If Tor dies, connections fail; there is no code path to the open internet.
- One app: the app runs in its own namespace on a private bridge. The host redirects its TCP to Tor, answers its DNS from Tor (names never go to a normal resolver), drops UDP, ICMP and IPv6, and never forwards. The namespace has its own drop rules as a second layer.
- Whole computer (Atlas tabs then use that same Tor instead of starting a second one): output rules redirect TCP and DNS to Tor and drop the rest; the filter chain's default is `drop`, so if the redirect rules vanish the machine goes offline instead of going open.
- Each site (Atlas) and each app gets its own circuits.
- Nothing is kept: Tor's data directory and Atlas's profile live on memory-backed storage and go when the group stops.
- The OS firewall and these rules never delete each other's tables (the firewall file no longer flushes the whole ruleset).
- Limits: root, or a program given `CAP_NET_RAW`, can bypass the rules; Tor carries TCP only (no VoIP, no QUIC-only services); IPv6-only sites do not load; IP-address URLs do not load in Tor tabs; a website still sees your browser's fingerprint, and Atlas Tor is not Tor Browser.

## Keep Tor ready

Off by default. When on, Tor's directory documents and its first relay are refreshed every few hours in the background, so a Tor tab opens in seconds. The cost: without bridges that refresh connects to the Tor network directly, and your network provider can see that this computer uses Tor. Turn it on if that does not matter to you, or set bridges first.

## Where things are

| What | Where |
|---|---|
| Crate | `crates/wol-tor` (GPL-2.0-or-later, no `unsafe`) |
| Switch state | `/etc/wol-tor/switches` (root-owned, the only scope switches), `/etc/wol-tor/bridges` (machine); per user: `~/.config/wol-tor/keep-ready` and `bridges` |
| Atlas group runtime | `$XDG_RUNTIME_DIR/wol-tor/<group>` (tor), `wol-tor-browser/<group>` (browser), `wol-tor-profile/<group>` (policies, fonts) |
| Benchmark plan | [bench/atlas-tor-plan.md](../bench/atlas-tor-plan.md) |

## Status

Built and unit-tested: torrc builder, relay and isolation, control port, namespace check, profile, rulesets and plans, doctor judging. Not built here: the `wol-admind` verbs that run the plans, the Settings pane, the `wol doctor tor` registration, packaging. Not run anywhere yet: the rulesets in the kernel (only parsed with `nft -c`), the units under systemd, tor with `Sandbox 1` and `TransPort`.
