# Browser privacy harness (plan AT-7 and AT-16, reel scenes 5 and 6)

Python 3 standard library only; run every tool with `python3 -I`. Each tool has a `--selftest`
that uses a stub browser (`stub_browser.py`) and needs no browser build: it proves that the tool
passes a clean stub and fails, naming the culprit, a stub that misbehaves.

| Tool | What it proves | Reel claim |
|---|---|---|
| `egress.py` | nothing but loopback leaves the machine and no DNS query is made: 10 local tabs, typing in the address bar, then idle (default 30 min); local canary page tries `fetch`, WebRTC STUN, WebTransport, DNS prefetch and preconnect (the ADR 0053 leak canaries, and the positive control: a run that does not see its own canaries is blind and fails) | "It talks to the sites you open. Nothing else." |
| `headers.py` | `DNT: 1`, `Sec-GPC: 1` and `navigator.globalPrivacyControl === true`, in a document and a worker, on a fresh profile and on one made by another browser build | Do Not Track and Global Privacy Control are on |
| `blocklist.py` | six known tracker URLs on a local page are blocked from the first load on a fresh profile with no network; with the blocker switched off the same page loads them (so "blocked" means blocked by the list); `manifest` prints the pinned list manifest; `coverage` names the bundled rule that covers each tracker | tracker blocking from a list built in |
| `page_asis.py` | text, element sequence and layout hash of a corpus page with the blocker off equal the page fetched raw; canvas readback noise is measured and documented | every page as the site made it |
| `defaults.py` | the effective defaults, read from the browser's own internals pages over a DevTools pipe: suggestions off, prediction never, DNT, GPC, services off, usage statistics off, crash reports kept | the privacy defaults |
| `policies.py` | every policy key WolOS writes exists in the engine's policy definitions with the right type and value (and recommended ones may be recommended) | policies are not silently ignored |

## Isolation (egress, headers, blocklist, page_asis, defaults)

`privlib.Isolation` re-executes the tool in a new network and mount namespace (`unshare`, as root or
with a user namespace) and fails closed (exit 2) if the namespace shows any interface but loopback.
Inside, a dummy interface with a default route makes a connection to any outside address leave a
packet we can see; DNS goes to our sink, which answers every name with its own loopback address and
records the TLS server name or HTTP host of what arrives. Where AppArmor refuses user namespaces
(the Ubuntu forge, ADR 0053 D2) run in a wolvm guest as root.

Exit codes: 0 pass, 1 a check failed, 2 the harness could not run (isolation missing, bad input).

## Running against a browser

```
export XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1      # the live session
B='/path/to/browser --ozone-platform=wayland'
python3 -I egress.py    run --browser "$B" --user 1000 --idle 1800 --omnibox-handshake /tmp/omni --out OUT
python3 -I headers.py   run --browser "$B" --user 1000 [--old-browser "$OLD"] --out OUT
python3 -I blocklist.py run --browser "$B" --user 1000 --out OUT
python3 -I page_asis.py run --browser "$B" --user 1000 --out OUT
python3 -I defaults.py  run --browser "$B" --user 1000 --out OUT
python3 -I policies.py  check --source crates/core/src/browser.rs --defs POLICY_DEFINITIONS_DIR_OR_TSV
```

`--omnibox-handshake PATH`: the tool creates `PATH.ready` when the tabs are loaded and waits for
`PATH.done`; a driver outside the guest types into the address bar (`wolvm key`, `wolvm type`)
and creates `PATH.done`. `--omnibox-cmd` runs a typing command inside instead. A headless browser
cannot open the ten tabs from the command line; use the windowed session.

## Known limits

- The "blocker off" side of `blocklist.py` is a stand-in until the first-run switch exists (pref
  `atlas.tracker_blocking`, AT-20): it disables the blocker's component extension in the profile
  (state 0, disable reason 1). Replace it with the switch (`--off-prefs`).
- Tracker hosts must not be on the HSTS preload list (the page uses plain http): the browser
  upgrades such a host to https and the local server never sees the request.
- `defaults.py` sets one Local State key (`internal_only_uis_enabled`) so the internals pages show.
- Fingerprint noise is on by default (decision D4) and perturbs canvas and audio readbacks only;
  `page_asis.py` reports it, it does not judge it.
