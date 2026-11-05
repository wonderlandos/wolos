# Atlas privacy proof (AT-7 and AT-16)

Concept: the browser's privacy claims (reel scenes 5 and 6: "It talks to the sites you open. Nothing else.", Do Not Track and Global Privacy Control on, tracker blocking from a built-in list that works offline, every page as the site made it) as tests that fail loudly and name the culprit. Why: a claim nobody can break is not a claim, and a policy key the engine does not know is silently ignored. How: `tools/bench/atlas/privacy/` (README there), Python 3 standard library only; each tool has a `--selftest` on a stub browser and a `run` against a real one in a network namespace.

## Identity of the numbers

| | |
|---|---|
| Date | 2026-11-04, one pass, in one wolvm guest `atlas-priv` on the forge (4 vCPU, 8 GiB, KVM, kernel `7.3.1-1-wolos`) booted from `wolos-2026.11.04-x86_64.iso` (sha256 `167ac1020a878116d1f57061c54837e46d1eb85dd1089c812ffde358c6101fac`) |
| Repack (before) | the browser package the ISO ships today (version 0.20.3.1-1, engine 156.0.8078.79) |
| Atlas (after) | overlay artifact `ov1`, `atlas-0.20.3.1-x86_64_linux.tar.xz`, sha256 `754b886a210c9110cd28a575329978d751a5bf3635aa45916c0eb15897005951`, fork commit `e976fe8f` (inherited series 346, platform 7, overlay 5: privacy 0001-0005) |
| Session | the live Wayland session of the guest, browser started as the live user (uid 1000) by the harness inside a new network namespace (loopback and one dummy interface; DNS answered by the harness) |
| Harness | the tools of this commit; the guest got a copy through a read-only disk |

## Results

| Check | Repack | Atlas ov1 |
|---|---|---|
| `egress.py`, 10 local tabs, typing in the address bar, idle 1800 s | PASS: 801 packets, 764 loopback, 37 to the canary page's own addresses, 9 DNS queries all from the canary page, 0 violations | PASS: 800 packets, 763 loopback, 37 to the canary page's own addresses, 9 DNS queries all from the canary page, 0 violations |
| `egress.py` with a profile where suggestions and the services are switched on, idle 150 s | FAIL (the harness catches real traffic): the inherited services host (50 connects, TLS server name seen) and `duckduckgo.com` (87, address bar suggestions); 17 DNS queries not from the canary page | not run (the switches are off in Atlas) |
| `headers.py`, fresh and existing profile, document and worker | FAIL: no `DNT`, no `Sec-GPC`, `navigator.globalPrivacyControl` undefined, 20 failures | PASS: `DNT: 1` and `Sec-GPC: 1` on all four requests, `navigator.globalPrivacyControl === true` in document and worker, fresh profile and one made by the repack |
| `defaults.py`, effective values from the internals pages | FAIL: 4 ok, 9 wrong (DNT, GPC, services enabled, bangs, extension proxy, spellcheck files, browser updates, list downloads, crash reporting mode 0 instead of -1) | PASS: 13 ok, 0 wrong |
| `blocklist.py`, 6 tracker URLs, no network, fresh profile | PASS: blocked from the first load (0 of 6 loaded, neutral control loaded); with the blocker extension disabled 6 of 6 load | PASS (same numbers) |
| `page_asis.py` | PASS: text 823 characters and 55 elements equal the raw page; layout hash `2b866de32bde` with the blocker off and on | PASS (same) |
| `policies.py` on `crates/core/src/browser.rs` | 19 of 19 keys defined with the right type and value, against 1421 engine definitions of 156.0.8078.79; 5 are deprecated, see below | same engine |

What the numbers say:

1. The repack already sends nothing in 30 minutes: its services switch reads `true`, but its consent flag (`services.user_consented`) is `false`, so no call is made until the person consents. The expectation in the brief ("it flags the repack's services traffic") did not hold for the default state; the harness does flag it as soon as the switches are on (second row). The zero-egress claim is therefore proven for the default state of both builds; what Atlas changes in this area is that the switches themselves are off (13 of 13 defaults) and DNT and GPC are on.
2. The two decision D2 defaults (address bar suggestions off, network prediction never) read `False` and `2` in both builds: the inherited series already registers them that way, so no overlay patch (0090, 0091) is needed. WolOS also ships both as recommended policies (`recommended_policies()`), which the person can switch on.
3. Packets to the canary page's own addresses (192.0.2.10 to 192.0.2.12, documentation ranges, the 37 "not loopback" packets) and the DNS queries for `*.egress.test` are the harness's own positive controls (WebRTC STUN, WebTransport and fetch attempts); a run that does not see them fails as blind. `--canaries-must-not-leak` counts them as violations (ADR 0053 leak canaries).
4. DNS prefetch and preconnect hints in the canary page were not acted on in either build (prediction is off in both).

## Built-in list manifest (AT-16)

Pinned from `third_party/ublock/assets` of the prepared tree (engine 156.0.8078.79, blocker 1.66.4, GPL-3.0), listed with `python3 -I tools/bench/atlas/privacy/blocklist.py manifest ASSETS`. The remote list URLs are stripped from `assets.json`, so only the bundled files are used and nothing is downloaded (list downloads are off: the `services.ublock_assets` pref reads `false` in Atlas).

| List | On by default | File | Size | Rules | Version or date | Licence |
|---|---|---|---|---|---|---|
| asset index | yes | `assets/assets.json` | 33 KiB | n/a | none in the header | GPL-3.0 (blocker) |
| Public Suffix List | yes | `assets/thirdparties/publicsuffix.org/list/effective_tld_names.dat` | 326 KiB | n/a | 2026-09-15_10-18-26_UTC | MPL-2.0 |
| uBlock bad-lists index | yes | `assets/ublock/badlists.txt` | 5 KiB | n/a | none in the header | GPL-3.0 (uAssets) |
| inherited list (annoyances.txt) | yes | `assets/helium/annoyances.txt` | 0 KiB | 1 | %timestamp% | see the list header |
| inherited list (unbreak.txt) | yes | `assets/helium/unbreak.txt` | 1 KiB | 3 | %timestamp% | see the list header |
| uBlock filters: Ads | yes | `assets/ublock/filters.min.txt` | 1848 KiB | 23320 | Thu, 17 Sep 2026 14:41:39 +0000 | GPL-3.0 (uAssets) |
| uBlock filters: Badware risks | yes | `assets/ublock/badware.min.txt` | 144 KiB | 4220 | Thu, 17 Sep 2026 10:56:03 +0000 | GPL-3.0 (uAssets) |
| uBlock filters: Privacy | yes | `assets/ublock/privacy.min.txt` | 136 KiB | 1832 | Thu, 17 Sep 2026 05:14:48 +0000 | GPL-3.0 (uAssets) |
| uBlock filters: Unbreak | yes | `assets/ublock/unbreak.min.txt` | 173 KiB | 2550 | Thu, 17 Sep 2026 16:19:33 +0000 | GPL-3.0 (uAssets) |
| uBlock filters: Quick fixes | yes | `assets/ublock/quick-fixes.min.txt` | 70 KiB | 352 | Thu, 17 Sep 2026 11:24:11 +0000 | GPL-3.0 (uAssets) |
| AdGuard – Ads | yes | not bundled | 0 | 0 | | selected by default but not bundled: empty offline (downloads are off) |
| EasyList | yes | `assets/thirdparties/easylist/easylist.txt` | 2098 KiB | 67825 | Thu, 17 Sep 2026 12:07:02 +0000 | GPL-3.0-or-later or CC BY-SA 3.0 (dual) |
| AdGuard/uBO – URL Tracking Protection | yes | not bundled | 0 | 0 | | selected by default but not bundled: empty offline (downloads are off) |
| EasyPrivacy | yes | `assets/thirdparties/easylist/easyprivacy.txt` | 1457 KiB | 56074 | Thu, 17 Sep 2026 05:25:58 +0000 | GPL-3.0-or-later or CC BY-SA 3.0 (dual) |
| Online Malicious URL Blocklist | yes | `assets/thirdparties/urlhaus-filter/urlhaus-filter-online.txt` | 810 KiB | 9318 | 2026-09-17T00:06:42Z | CC0-1.0 (LICENSE.md in the bundle) |
| AdGuard – Cookie Notices | yes | not bundled | 0 | 0 | | selected by default but not bundled: empty offline (downloads are off) |
| EasyList – Cookie Notices | yes | not bundled | 0 | 0 | | selected by default but not bundled: empty offline (downloads are off) |
| uBlock filters – Cookie Notices | yes | not bundled | 0 | 0 | | selected by default but not bundled: empty offline (downloads are off) |
| EasyList – Newsletter Notices | yes | not bundled | 0 | 0 | | selected by default but not bundled: empty offline (downloads are off) |
| EasyList – Notifications | yes | not bundled | 0 | 0 | | selected by default but not bundled: empty offline (downloads are off) |
| uBlock filters – Annoyances | yes | not bundled | 0 | 0 | | selected by default but not bundled: empty offline (downloads are off) |
| Peter Lowe's ad and tracking server list | yes | `assets/thirdparties/pgl.yoyo.org/as/serverlist` | 93 KiB | 3560 | none in the header | McRae GPL 4.r53: list owner asks for non-commercial use (README.md in the bundle) |

11 filter lists on by default and bundled, 169055 rules, 6.7 MiB; 8 selected by default with nothing bundled

Reading the table:

- On with no network and bundled: 11 filter lists, 169055 rules, 6.7 MiB. Eight lists are selected by default in `assets.json` but have no bundled file (AdGuard lists, the cookie and newsletter notice lists, the annoyances list): offline they are empty, and the blocker does not fetch them. The reel promise "a list built into Atlas" holds for the 11 above; the cookie-notice and annoyance blocking the person may expect from the blocker's own page is not there offline.
- The tracker test page covers, with rules the matcher models (`python3 -I tools/bench/atlas/privacy/blocklist.py coverage ASSETS`): `static.criteo.net` (EasyPrivacy `||criteo.net^$third-party`), `match.adsrvr.org` (EasyList `||adsrvr.org^$third-party`), `googleads.g.doubleclick.net` (EasyList), `static.hotjar.com` (EasyPrivacy), `pixel.quantserve.com` (EasyPrivacy), `sb.scorecardresearch.com` (EasyPrivacy).
- Licences: the uBlock lists are GPL-3.0 (uAssets), EasyList and EasyPrivacy are GPL-3.0-or-later or CC BY-SA 3.0 (dual), the malicious URL list is CC0-1.0, the public suffix list is MPL-2.0. One needs a decision: the Peter Lowe server list is published under the "McRae General Public License 4.r53", whose text forbids any use that makes anybody money (its README in the bundle says "noncommercial uses only"). It is bundled and on by default, and EasyList and EasyPrivacy already cover the same hosts; removing it from `assets.json` and the package (about 93 KiB, 3560 entries) is the safe choice for a distributed OS image.
- The two inherited helper lists (`annoyances.txt`, `unbreak.txt`) carry 1 and 3 rules and a `%timestamp%` placeholder as their version.
- Tracker hosts in the test page must not be on the HSTS preload list because the page uses plain http: `www.google-analytics.com` and `bat.bing.com` are, the browser upgrades them to https and the local server never sees the request (first attempt of this harness: 2 of 6 never loaded even with the blocker off). They were replaced.

## Policies (item 5)

`python3 -I tools/bench/atlas/privacy/policies.py check --source crates/core/src/browser.rs --defs DIR` reads the keys of `policies()` (16 mandatory) and `recommended_policies()` (3) and checks them against the engine's `policy_definitions` (1421 definitions, YAML, read with a small reader; or the fixture table `crates/core/tests/fixtures/atlas-policies.tsv`, regenerate it with `policies.py dump --defs DIR`). Result: all 19 keys exist with the right type; the enum values are allowed; the three recommended keys (`SearchSuggestEnabled`, `NetworkPredictionOptions`, `TranslateEnabled`) are marked recommendable by the engine. Five keys are deprecated in 156 and still honoured: `PromotionalTabsEnabled` (replaced by `PromotionsEnabled`), `PrivacySandboxPromptEnabled`, `PrivacySandboxAdTopicsEnabled`, `PrivacySandboxSiteEnabledAdsEnabled`, `PrivacySandboxAdMeasurementEnabled`; they will go in a later engine and the check will then name them as unknown.

## Reproduce

In a wolvm guest as root (the forge refuses user namespaces, ADR 0053 D2); see `tools/bench/atlas/privacy/README.md`. The 30 minute run needs a driver outside the guest to type into the address bar (`wolvm key NAME ctrl-l`, `wolvm type NAME "weather in lisbon tomorrow"`) between `PATH.ready` and `PATH.done`.
