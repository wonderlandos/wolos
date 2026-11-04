# 0053 - Atlas private and Tor tabs: the kernel is the kill switch
Status: proposed. The OS-wide layer (per-app and whole-machine Tor, the switches, `wol doctor tor`) is [ADR 0056](0056-wol-tor-system-tor-layer.md); this record is the Atlas tab part. (lead order 2026-11-04 20:12Z: "a private tab and also a Tor tab like Brave's, only much more secure and faster"). Draft by ATLAS-TOR; the CTO accepts it when the `wol-tor` crate, the Tier 0 Tor profile and journey J-T1 are green on the VM. Builds on ADR 0043 (Atlas: patch series, cheapest mechanism first, no engine rewrite) and ADR 0040 (security defaults).

## Context

What people mean by "a Tor tab like Brave's" (read in brave-core master on 2026-11-04: `components/tor/`, `components/services/tor/tor_launcher_impl.cc`, `net/proxy_resolution/proxy_config_service_tor.cc`, `chromium_src/net/socket/socks5_client_socket.cc`):

- **One tor per browser.** Every Tor window shares one tor process. It is started from a utility process with `base::LaunchProcess` (`kill_on_parent_death`), with no sandbox of its own. The tor binary and the pluggable transports arrive through the browser's component updater.
- **Persistent tor data.** `DataDirectory` is a folder in the user data directory, so the guard, the consensus and, in older releases, `tor.log` stay on disk. This is why a second start is quicker.
- **A Tor window is an off-the-record profile** whose proxy config is `socks5://127.0.0.1:<port>`. Stream isolation uses SOCKS5 username/password (the browser patches SOCKS5 auth into the engine): the username is the `NetworkAnonymizationKey` of the top-frame site, the password is 16 random bytes, and an entry expires after 10 idle minutes. "New Tor circuit for this site" replaces the password.
- **Fingerprinting:** the browser's own randomisation ("farbling"). It does not try to look like Tor Browser, and the vendor says plainly that a Tor window is not Tor Browser.
- **Weak spot:** every guarantee is a setting inside the shared network service of one browser process, so any engine path that ignores the proxy leaks. The public record has examples: brave-browser #13527 (2021, HackerOne, "Tor DNS issue": `.onion` names reached the system resolver through the CNAME ad-block check), #23579 (2022, HackerOne, the DevTools console leaked in a Tor window), #59393 (2026-09, WebTransport sessions resolved `.onion` names through normal DNS). Other weak spots: bootstrap from cold is 5 to 30 s [E]; a crash dump of the browser process can hold Tor-window memory; tor shares the user's account with no extra confinement.

Atlas is Chromium with a patch series (ADR 0043). The tab model has a limit that decides the UX: a tab belongs to its window's profile, so an off-the-record tab, or a tab that runs in another process tree, cannot sit inside a normal window without rewriting the engine. ADR 0043 D8 forbids that rewrite.

## Decision

### D1. Two new kinds of tab, both reached from the normal window
- The new-tab menu (and the File menu in the compositor strip) gains **New Private Tab** (Shift+Ctrl+N) and **New Tor Tab** (Alt+Shift+Ctrl+N).
- New Private Tab opens a tab in the private window, and creates that window if there is none. The private window is the engine's off-the-record profile: nothing goes to disk, and it closes with its last tab.
- New Tor Tab opens a tab in the window of the current **Tor group**, and starts the group if none is running.
- A group is a separate Atlas instance with its own tor (D2). "New Tor Identity" in that window's menu ends the group and starts a fresh one: new tor process, new circuits, empty storage.
- The words say "tab" because the person asked for tabs; the tab opens in its own window because the engine needs that (stated in the Context). The window title and the tab strip make the kind of window visible (D6).

### D2. A Tor group is two sandboxed user units, and only one of them has a network
- `wol-tor-<g>.service` runs tor.
  - Its DataDirectory is in a `RuntimeDirectory` (tmpfs, removed on stop), seeded from the bootstrap cache (D4).
  - `SocksPort` and `ControlPort` are **Unix sockets only** (`unix:` paths in `$XDG_RUNTIME_DIR/wol-tor/<g>/`). Control uses cookie authentication.
  - Logging: `StandardOutput=null`, `SafeLogging 1`, no log file. Health is read over the control port.
  - systemd sandbox: `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp`, `PrivateDevices`, `RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6`, `RestrictNamespaces`, `LockPersonality`, `MemoryDenyWriteExecute`, `SystemCallFilter=@system-service`, `CapabilityBoundingSet=`, `LimitCORE=0`, `MemorySwapMax=0`.
  - tor's own `Sandbox 1` is on when no pluggable transport is configured [V: the PT exec path and `Sandbox 1` on 0.4.8/0.4.9].
- `atlas-tor-<g>.service` runs `wol-tor host`, which starts Atlas with `--user-data-dir` under `PrivateTmp` (tmpfs).
  - Unit settings: `PrivateNetwork=yes` (only `lo` exists), `PrivateUsers=yes`, `MemorySwapMax=0`, `LimitCORE=0`, `Environment=TZ=UTC LANG=en_US.UTF-8`, and the Tor policy directory bind-mounted read-only over the normal one (D5).
  - `wol-tor host` listens on `127.0.0.1:9150` inside that namespace and relays SOCKS5 to tor's Unix socket. Unix sockets are file paths, so they cross the namespace boundary; nothing else does.
  - **The profile is written before the unit exists.** `ProtectSystem=strict` makes the whole file system read-only for the unit, so nothing can be installed from inside it. `wol-tor open` (the launcher, outside the unit) writes the policy file and `fonts.conf` into `$XDG_RUNTIME_DIR/wol-tor-profile/<g>/` and the unit binds `policies/` read-only over `/etc/atlas/policies` (and sets `FONTCONFIG_FILE` to the `fonts.conf` beside it). The bind has no `-` prefix, so a missing source fails the unit, and `wol-tor host` itself re-reads the files (`profile::verify`: proxy, forced private profile, no QUIC) and refuses to start Atlas if they are absent or altered. The Atlas package must ship the empty `/etc/atlas/policies` directory (the unit cannot create a mount point on a read-only tree) [V on the VM].
  - **The unit's own tmpfs.** The unit has `RuntimeDirectory=wol-tor-browser/<g>` (real tmpfs, mode 0700, removed on stop): Atlas's `--user-data-dir` and the host's command socket are there, not in `PrivateTmp` (which can be disk-backed).
- **Kill switch = the kernel.** The browser's namespace has no route anywhere. If tor dies, or an engine path ignores the proxy (WebRTC, WebTransport, QUIC, DNS prefetch, a future bug like the three above), the connect fails with `ENETUNREACH`. Nothing falls back to the clear net, because no code path for it exists.
- **Fail closed at start.** systemd applies `PrivateNetwork=` in user units only where unprivileged user namespaces work, and silently skips it otherwise. This was measured on the build host on 2026-11-04: Ubuntu 24.04 with the AppArmor userns restriction showed the host's interfaces and a connect to 1.1.1.1:443 succeeded. So `wol-tor host` checks first: interfaces = `lo` only, no IPv4 or IPv6 route except loopback. If the check fails, it refuses to start Atlas with "Tor tabs need an isolated network and this system did not provide one". No Tor tab is ever opened without the kill switch.
- **The relay** (`wol_tor::relay`):
  - It accepts SOCKS5 `CONNECT` only. `BIND` and `UDP ASSOCIATE` are refused (Tor carries TCP only; UDP is the classic WebRTC leak).
  - SOCKS4/4a are refused, and so are IP-literal targets in private, loopback and link-local ranges.
  - It never connects anywhere but the tor socket.
  - It maps each stream to a tor isolation credential (D3).
  - It holds the control connection with `TAKEOWNERSHIP`, so tor exits when the group ends, even after a crash of the host.
- **Why two units and not one process tree:** tor needs the network and the browser must not have it. One unit cannot be both.
- **Why one tor per group and not one per user (Brave):** a compromise or misconfiguration of one group cannot reach another; each group starts with empty circuits; the cost is 25-40 MB RSS of tor per group [E, measured by J-T3]. The guard stays shared (D4), so more processes do not mean more guards.

### D3. Stream isolation: per site, per group, renewable, with no help from the engine needed
- With engine patch 0007 (SOCKS5 auth, the mechanism Brave uses), Atlas sends username = top-frame site (from the request's `NetworkAnonymizationKey`) and password = a per-site epoch. The relay forwards it as the tor credential `<group>/<site>/<epoch>`. Tor's `IsolateSOCKSAuth` puts different credentials on different circuits.
- One function maps a site name to its key (`Isolator::key_of_site`): the longest key the browser offered that is the name or a parent of it (so `www.example.github.io` is `example.github.io`, private registries included, as patch 0007 sends), else the destination rule below. Streams and "New circuit for this site" both use it, so the toolbar item moves the circuit the browser is actually using.
- Without the patch (the repackaged binary, Tier 0), the relay derives the key from the **destination host**: an IP literal stays as it is, a name becomes its last two labels (or three when the second-level label is short, such as `co.uk`). Every stream then gets `<group>/<dest-key>/<epoch>`.
  - This is coarser than first-party isolation: a tracker embedded on two sites shares one circuit. But it never leaves two sites on one circuit by accident, and it works today.
  - J-T2 measures both modes.
- "New circuit for this site" bumps the epoch of the key (`wol-tor newcircuit <g> <host>` on the control socket of the host, or the toolbar item of patch 0008). "New Tor Identity" ends the group (D1). There is no global `NEWNYM`, which would reset every site at once.

### D4. Faster: a warm cache, a persistent guard, conflux, and no work on the critical path
- **The bootstrap cache.** `wol-tor-cache.service` (user unit, timer every 6 h, `Nice=19`, `IOSchedulingClass=idle`) runs tor briefly with the same bridges configuration and keeps `cached-certs`, `cached-microdesc-consensus` and `cached-microdescs` in `$XDG_STATE_HOME/wol-tor/cache/`. A group copies them into its tmpfs DataDirectory before tor starts. A consensus is used only while it is valid; `wol_tor::cache` reads `valid-until` and refuses a stale one.
  - **Opt-in, default off.** A periodic fetch is a Tor bootstrap from this machine. Unless bridges are configured it connects to the Tor network directly (clear net), and tells the local network and the ISP "this machine uses Tor". So nothing is installed, enabled or run until the person turns on "Keep Tor ready" (Settings, or `wol-tor keep-ready on`), which writes the two unit files and enables the timer; `off` removes them. With the switch off no unit file exists (tested), and `cache-run` refuses to run outside the unit. With bridges set (`wol-tor bridges set`), the cache fetches through them. The switch can be turned on from the Tor tab's first page, with this sentence next to it.
- **Persistent guard.** (Only with "Keep Tor ready" on.) The cache unit owns `state` (guards and the learned circuit-build timeout); groups get a read-only copy and never write it back. One guard per machine, as with Tor Browser, instead of a new guard per group. A new guard per start would expose the user to more guards (guard discovery) and would bootstrap slower.
- **torrc for speed:**
  - `ConfluxEnabled 1` and `ConfluxClientUX latency`: two legs per exit circuit, lower tail latency (tor >= 0.4.8).
  - `CircuitPadding 1`.
  - `LearnCircuitBuildTimeout 1` (its learned state is seeded).
  - `ClientUseIPv6 1`.
  - `ClientPreferIPv6ORPort auto`.
- **The critical path** is: copy about 3 MB into tmpfs, tor start, `BOOTSTRAP PROGRESS=100`, Atlas start. The Atlas instance starts in parallel with tor: the relay holds SOCKS connects until tor reports `done` (the upper bound is 30 s, then the error page), so the window shows while tor finishes.
- **Targets** [E, replaced by the benchmark in D7]: a Tor tab is usable in 3 s or less with a warm cache, against Brave's 5-30 s cold; page load median no worse than Brave's Tor window on the same corpus at the same hour.

### D5. The Tor profile (Tier 0: policies and unit settings, works on the repackaged binary)
- Bind-mounted policy file (`wol_tor::profile::policies()`):
  - `ProxySettings` = fixed `socks5://127.0.0.1:9150`; `IncognitoModeAvailability` = 2 (forced).
  - `QuicAllowed` false; `WebRtcIPHandlingPolicy` = `disable_non_proxied_udp`; `HttpsOnlyMode` = `force_enabled`; `DnsOverHttpsMode` = `off` (names are resolved by the exit through SOCKS5).
  - **IP-literal addresses do not load in a Tor tab.** `SafeSocks 1` makes tor refuse a SOCKS request that carries a bare IP address (the sign of a local DNS lookup), and the relay refuses private and loopback literals too. So `http://93.184.216.34/` shows an error page; use the name. This is intended; `.onion` and ordinary names are unaffected.
  - `NetworkPredictionOptions` = 2; `SearchSuggestEnabled` false; `DefaultJavaScriptJitSetting` = 2 (JIT off: the "Safer" level of Tor Browser; costs JS speed, measured by J-T4); `DefaultWebUsbGuardSetting`, `DefaultWebHidGuardSetting`, `DefaultSerialGuardSetting`, `DefaultWebBluetoothGuardSetting`, `DefaultGeolocationSetting`, `DefaultSensorsSetting`, `DefaultNotificationsSetting` = 2.
  - `BrowserThemeColor` = `#6A4DF4` (the Tor window's colour, see D6; the normal profile keeps "no theme colour").
  - `ExtensionInstallBlocklist` = `*`.
  - `AudioCaptureAllowed`, `VideoCaptureAllowed`, `ScreenCaptureAllowed`, `PasswordManagerEnabled`, `AutofillAddressEnabled`, `AutofillCreditCardEnabled` false; `SavingBrowserHistoryDisabled` true; `BrowserSignin` 0; `SyncDisabled` true; `MetricsReportingEnabled` false.
  - [V: every key against the policy definitions of the tree, AT-8 validator.]
- WebRTC: `WebRtcIPHandlingPolicy` = `disable_non_proxied_udp` and the kernel (no UDP route). `WebRtcHideLocalIpsWithMdns` is **not** disabled: turning it off would expose local ICE addresses, and a test asserts the flag list never names it.
- Unit environment and flags: `TZ=UTC`, `LANG=en_US.UTF-8`, `--lang=en-US`, `--accept-lang=en-US,en`; a fixed font set through `FONTCONFIG_FILE` (bundled Noto and Inter, no user fonts); `--disable-features=WebGPU,...` [V names]; crash reporting off; no extensions. The reduced Linux user agent of the engine is the same for every Atlas Tor user.
- Honest limit, written in the Tor window's first page: Atlas Tor hides the address and splits sites across circuits. It is **not** Tor Browser: a Chromium fingerprint is rarer than a Tor Browser one. For the strongest anonymity, use Tor Browser.

### D6. What the engine patches add (Tier 4, written to the patches-0 format, `atlas/patches/`)
| # | Patch | Release |
|---|---|---|
| 0006 | `ui/private-and-tor-tab-menu`: the two menu items and accelerators; New Tor Tab asks the bridge (`atlas-bridge`, ADR 0043 AT-9a) to open the URL in the current group | 0.14 |
| 0007 | `privacy/tor-socks-isolation`: SOCKS5 username/password (RFC 1929) for the Tor instance's proxy; username = top-frame site of the `NetworkAnonymizationKey`, password = epoch | 0.14 |
| 0008 | `ui/tor-window-chrome`: tab strip and sidebar colour, onion glyph, "Tor" title suffix, New Tor Identity and New Circuit for This Site items, letterboxing (content size rounded down to 200 x 100 steps, at most 1400 x 900) | 0.14 |
| 0009 | `privacy/tor-onion-location`: honour `Onion-Location` only in the Tor instance (offer, never auto-redirect); `.onion` names are never resolved outside the Tor instance (the #59393 class) | 0.14 |

**Status of the patches: DRAFT, not applied.** They were written without a prepared engine tree and are in the Atlas plan directory (`tor/patches`, with a `series.add`). Symbols they use that are defined nowhere yet and must be written in the fork: `AtlasTorEpochFor` (0007), `AtlasTorBumpEpoch` (0008), `ShowOnionAvailableInfobar` (0009), `kAtlasNewTorTabMenuItem` (0006). Until they land, the Tier 0 profile and the relay's destination isolation are what ships.

Colour: no new token (the brand rule in the chrome tokens). The private window keeps the engine's dark off-the-record look. The Tor window is coloured at Tier 0 by the `BrowserThemeColor` policy of the Tor instance: the design palette's violet `#6A4DF4` (`icon.categories.wolos.body`) seeds the engine's palette, so the tab strip and frame are violet even on the repackaged binary. Patch 0008 adds the onion glyph in `accent` and the title suffix. Patches 0007 and 0008 act only when the instance runs with the switches `--atlas-tor-isolation` and `--atlas-tor-window`, which `wol-tor host` passes (the repackaged engine ignores unknown switches). Colour is never the only signal: the title says "Private" or "Tor".

### D7. The benchmark (owner: AT-2 harness, `tools/bench/atlas/`; plan in `docs/bench/atlas-tor-plan.md`)
- **A/B:** Atlas Tor group vs the Brave Tor window, same VM image (4 vCPU, 8 GiB), same network, same hour.
  - Rounds alternate A, B, A, B; 20 rounds per cell; we report the median, p90 and IQR.
  - Tor-network variance is the biggest noise, so each round also records the circuit's guard and exit RTT through the control port (`GETINFO circuit-status`, `ns/id/...`).
- **Metrics:**
  - (M1) cold bootstrap: process start to `BOOTSTRAP PROGRESS=100`, both with no cache.
  - (M2) warm open: menu click (`atlas-bridge` timestamp) to first contentful paint of a local test page served through an onion service we run. For Brave: the window open to the same paint.
  - (M3) page load: `loadEventEnd` of 10 fixed sites, our mirrored corpus on a host we control, reached via clearnet exit and via its onion address.
  - (M4) relay overhead: on a Chutney test network on loopback (deterministic), Atlas through the relay against a direct SOCKS connection to the same tor. Target under 0.5 ms p50 per connect.
  - (M5) RSS of the group (tor + Atlas instance) at idle and with 5 tabs.
  - (M6) leak checks, pass/fail rather than a number: the J-T1 suite also run on Brave, so the comparison is "these leak there, not here".
- **Exit:** we say "faster" only where M1-M3 medians are lower with non-overlapping IQRs. Otherwise the release notes give the numbers and do not use the word.

### D8. Not in this ADR
- Arti (the Rust Tor client) instead of C tor: a candidate once it has pluggable transports and conflux at parity [V]. `wol_tor::torrc` is the only file that would change.
- A Tor-only system mode.
- Tor for other apps.
- Onion services hosting.
- A VPN-over-Tor mode.

## Journeys
| Id | What | Proves |
|---|---|---|
| J-T1 | In the Tor group, a page tries: plain fetch, WebRTC STUN, WebTransport to an IP, DNS prefetch of a canary name, QUIC, `.onion` via a normal window. Only the SOCKS path reaches the network (packet capture on the VM host bridge: no DNS for the canaries, no UDP from the guest except tor's own) | kill switch, no DNS leak |
| J-T2 | Two sites embedding the same third party: circuits differ (control port `circuit-status` by `SOCKS_USERNAME`); "New circuit for this site" changes only that site's circuit | isolation |
| J-T3 | Kill tor (`systemctl --user kill wol-tor-<g>`): the open tab loads nothing, the error page says Tor stopped, nothing reaches the clear net; tor restarts and the tab reloads | kill switch on crash |
| J-T4 | Close the group: no file of the group remains under `$HOME`, `$XDG_STATE_HOME` holds only the cache; swap and core files never got Tor pages (`MemorySwapMax=0`, `LimitCORE=0` read back from the units) | no persistence |
| J-T5 | Bridges: with direct Tor blocked on the VM's bridge (iptables drop to the consensus' guards), obfs4 then Snowflake bootstrap succeeds | censorship |
| J-T6 | Private tab: open, browse, close: nothing in the profile dir changed except what the engine documents for incognito | private tab |
| J-T7 | Host without user namespaces (the Ubuntu forge as is): New Tor Tab refuses with the sentence from D2, and no Atlas instance starts | fail closed |

## Consequences
- The Tor tab is safer than Brave's for a structural reason, not because of more settings: the browser that renders pages has no network.
- The cost is a process tree per group (tor 25-40 MB plus an Atlas instance [E]), so the Memory Saver rules of ADR 0026 apply to the Tor instance like any other.
- Things that need the network inside the Tor instance (spell-check download, component updates, Safe Browsing) do not work there. This is intended.
- The repackaged browser already gets the Tor group with destination isolation (Tier 0). The engine patches only refine it.
- New packages: `tor` (repackaged from Arch, plus our tmpfiles and units), `lyrebird` (obfs4, webtunnel), `snowflake-client`. The recipe plan is in the Atlas plan directory (`tor/tor-wolos-recipe.md`), and the pkgbuilds owner writes it.
- New crate `crates/wol-tor` (GPL-2.0-or-later, std only, `forbid(unsafe_code)`).
