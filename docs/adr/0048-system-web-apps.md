# 0048 - System web apps: folder, bridge, kit
Status: proposed (lead order 2026-11-03 23:18Z and 23:21Z: move tools like Doctor into graphical apps and build a suite of system apps, the way the big platforms ship their core apps). Builds on 0042 (the WolOS web runtime), 0021 (wopkg and the package grants), 0040 (security defaults), 0033 (wold's caller identity) and 0031 (web apps as apps).

## Context
WolOS's tools (Doctor, the sound and games checks, updates, security, storage, network) are `wol` verbs with `--json` output and a few shell panels. The lead wants each to be a real app with the system's look, and many more apps after them, written fast by small teams. ADR 0042 gives a runtime (one host process per app id, the Rust browser engine, an origin gate, the Memory Saver's scope) but no way for a page to reach the system and no shared look.

## Decision
**A system web app is a folder**, `webapps/<slug>/` in this repository, installed to `/usr/share/wolos/webapps/<slug>/`:
- `app.toml` (`webengine::bridge::Manifest`, unknown keys refused): `id` = `org.wolos.webapp.<slug>` (`process::APP_ID_PREFIX`, the slug is the folder's name, never `kit`), `name` (one line, at most 40 characters), `version` (`major.minor.patch`), `entry` (default `index.html`), `icon` (`.svg` or `.png` inside the folder), `summary`, `permissions` (names from the catalog below).
- Static HTML, CSS and JavaScript, no bundler, no npm, no build step, nothing fetched from the network (test-checked). Every visible word is in one table per app, `strings.js` (`window.WOLKIT_STRINGS`), read through `wolkit.t(key)` and `data-t`/`data-t-<attr>` markup; plain English until the OS locale system lands, then one table per language with the same keys.
- Its policy (ADR 0042) is `Manifest::policy`: origins `file:///usr/share/wolos/webapps/<slug>/` and `file:///usr/share/wolos/webapps/kit/`, nothing else, no optional web feature.

**The kit** is `webapps/kit/` (under 60 kB, test-checked; it is about 43 kB):
- `wolkit.css`: the golden tokens of `design/tokens/colors.json` (test-checked), light and dark from `<html data-theme>`, the eight accents of `wol_core::theme::accent::ACCENTS` from `<html data-accent>` (test-checked), one type scale on the 13 px UI size, one spacing scale, the radii, and the Settings layout (sidebar and grouped rows).
- `wolkit.js` (a classic script, loaded with `defer`: pages on `file:` cannot load modules in every engine): light-DOM custom elements `wol-app` (sidebar and content, skip link), `wol-sidebar` (links as items, hash routing, arrow keys, `select` event), `wol-list` (grouped card with heading), `wol-row` (label, detail, end content), `wol-switch` (role switch), `wol-button` (a real button inside; `primary`, `quiet`, `danger`, `busy`), `wol-sheet` (modal confirm, focus in and back, Escape is Cancel; `confirmSheet()`), `wol-badge` (`ok|warn|fail|info`: colour, shape and words, never colour alone), `wol-progress` (role progressbar, indeterminate without a value), `wol-empty`, `wol-search` (`/` focuses it, Escape clears it). Helpers on `window.wolkit`: `t`, `localize`, `explain` (an error as one line for a person), `confirmSheet`.
- `bridge.js`: builds `window.wolos` over the runtime's channel.
- `mock-bridge.js` and `fixtures.js`: development mode (below).

**The bridge, frozen contract (v1):**
- `wolos.call(method, params = {}) -> Promise<result>`. A method is a `wol` verb's action name in the controller's command tree (`audio.doctor`, `update.status`), `params` its named arguments as a JSON object (`{"topic": "disks"}` for `sys`), the result its `--json` output. An error rejects with an `Error` carrying `code`, `message` and `hint`; `code` is a `shared::errors::ErrorCode` name (`failed usage not-found denied unavailable invalid unsupported`); `message` is for the person; a `hint` that starts with a capital letter is for the person too (`Turn it on in Settings > Privacy`), one in lower case is for the developer.
- `wolos.subscribe(topic, handler) -> unsubscribe`. Topics v1: `app` (the record below changed), `permissions` (the granted list changed), `progress` (`{method, done, total, label}` of a long call).
- `wolos.app`: `{id, version, locale, theme: "light"|"dark", accent}`; `wolos.permissions`: the granted names; `wolos.ready`: a promise; `wolos.dev`: true in development mode.
- Wire format, one JSON text per message. Page to host: `{"id":n,"call":"<method>","params":{..}}`, `{"sub":"<topic>"}`, `{"unsub":"<topic>"}`. Host to page: `{"id":n,"ok":<result>}`, `{"id":n,"err":{code,message,hint}}`, `{"topic":"<topic>","data":<value>}`. At most 64 KiB per page message (`bridge::MAX_MESSAGE`).

**Permissions** are named capabilities in one catalog, `crates/webengine/src/bridge/catalog.rs` `PERMISSIONS` (the mock's copy is test-checked against it). Each opens a fixed set of methods; each method belongs to exactly one permission. v1: `system.read` (about, sys, hw.status, power.status, battery, boot.status, modules.status), `update.read` (update.status, update.check), `packages.update` (update.apply; consent `packages-update`, administrator), `security.read` (security.status, firewall.status), `network.read` (network, wifi.get, bluetooth.get; consent `network-info`), `audio.read` (audio.get, audio.doctor; consent `audio-info`), `audio.control` (audio.volume, audio.mute, audio.unmute, audio.reset; consent `audio-control`), `games.read` (games.doctor), `doctor.read` (doctor.check), `doctor.fix` (doctor.fix, administrator), `notifications` (notify; consent `notifications`). Every method is test-checked to be a verb of the command tree (doctor.check and doctor.fix once the Doctor web app's branch lands).

**Who grants.** The app never grants itself anything; declaring a permission only makes it askable. wold decides, through the machinery that exists:
- A system app (its folder is part of the sealed image, owned by the `wolos-webapps` package) holds the permissions it declares, like the shell's own layers come from the system defaults.
- Any other web app holds a permission only when it has a consent capability and `/etc/wolos/consent.json` grants it to the app, the app key being the app's folder (a canonical path, `wol_core::consent::is_valid_grant`); Settings > Privacy lists and revokes it like any grant. A permission without a consent capability (`system.read`, `security.read`, `games.read`, `doctor.*`, `update.read`) is for system apps only.
- An administrator method (`packages.update`, `doctor.fix`) still goes through wold's approval sheet and the administrator password for every call (the sheet is wold's and the shell's; `wol-sheet` is only for an app's own confirmations).

**The host's end.** `webengine::bridge::Bridge` (one per page) reads page messages, refuses a method not in the catalog (`not-found`), a permission the manifest does not declare or wold did not grant (`denied`), and hands every other call as a `Call` to a `Wold` (on a worker thread; replies are posted when they come); events reach the page only for topics it subscribed to (`bridge.js` always subscribes to `app` and `permissions` to follow the theme and the grants). This check is early and cheap; wold checks again and is the authority.

**wold's end** (built, WEBHOST 2026-11-04; `crates/api/src/webapp.rs`, `crates/shared/src/jobs.rs` `JobRequest::WebappCall`): one request on the existing connection, `webapp-call <method> <params JSON>`. wold takes the caller's identity from the handshake of ADR 0033 and accepts it only when the executable is `/usr/bin/wol-webhost`; the app id is the caller's cgroup scope (`app-wol-<app id>-<pid>-<n>.scope`, `process::scope_unit`), never a word in the request; wold reads that app's `app.toml` itself, checks the method is in the catalog and its permission declared and held, then runs the verb through the same parser and handlers as `wol mcp` (`controller::catalog`), so no new privileged surface exists. The catalog and the little of `app.toml` wold reads live in `wol_core::sysapp` (wold may not depend on the host crate); `webengine::bridge::catalog` re-exports it. wold starts `/usr/bin/wol webapp-call <app id> <method> <params>` (hidden verb, `crates/controller/src/webapp.rs`) as the caller's account with a cleared environment, a 120 s deadline and a 1 MiB reply cap; an app of others is found under `/opt/wolos/webapps/<slug>/` and holds a permission only with a consent grant for its canonical folder. An administrator method goes through the helper gate's approval sheet and the password, and a verb that gate has no request for (today `update.apply`, `doctor.fix`) is refused with `unavailable` and nothing runs. The host re-reads the consent file every two seconds for an app of others and posts a `permissions` event when its grants change; a system app's list never changes while it runs.

**The engine's end** (C API addition, ABI 1, symbol version `WWE_1.1` in `wolwebengine.map`; in the shim since WEBHOST, `crates/webengine/src/engine/{lib,channel}.rs`):
```c
typedef void (*wwe_message_fn)(void *ctx, const uint8_t *text, size_t len);
/* For views made after this call: in each top-level document whose URL is inside `origin` (a
 * file:///dir/ folder of the policy), before any page script, the engine defines
 * window.__wolosHost = {init: <init_json parsed>, post(text), onmessage: null}, the property
 * non-writable and non-configurable. post(text) calls on_message on the engine thread (texts over
 * 64 KiB are dropped). Frames and other documents get nothing. 0 ok, -1 error. */
int wwe_engine_set_host_channel(wwe_engine *engine, const char *origin, const char *init_json,
                                wwe_message_fn on_message, void *ctx, char *err, size_t errlen);
/* Calls window.__wolosHost.onmessage(text) in the view's top-level document if it has the channel;
 * 0 delivered, -1 not (no channel, or the document is elsewhere). */
int wwe_view_post_message(wwe_view *view, const uint8_t *text, size_t len);
```
The shim implements them with a user script and the page's console: the engine has no page-to-embedder message call at the pin, so `post(text)` writes one `console.debug` line (`wolos-host:` + a per-engine random secret + the text) that the delegate's console callback reads; the secret lives only in the injected script's closure, so a frame or another document cannot forge a message, and the line is never logged. `wwe_view_post_message` evaluates `window.__wolosHost.onmessage(text)` with the text as a JavaScript string literal. A library of the 1.0 build has neither symbol: the loader (`ffi::Lib::set_host_channel`) sees `None`, the host opens the page with `?dev` and says so on stderr (`WebView::has_channel` is false).

**Development mode.** `index.html?dev` in any desktop browser: with no runtime channel, `bridge.js` uses `mock-bridge.js`, which answers from fixture files (`webapps/kit/fixtures.js`, then the app's own `fixtures.js`; one `wolkitFixtures({...})` call of strict JSON: `app`, `permissions` = what `app.toml` declares (test-checked), `calls` keyed by method, a result or `{"$error": {...}}`). The same files feed `bridge::mock::Fixtures`, a `Wold` for Rust tests. Address options: `theme`, `accent`, `locale`, `deny=<perm,..>`, `fail=<method,..>`, `delay=<ms>`. Without `?dev` and without the runtime, every call fails with `unavailable`.

**Packaging and launch.**
- System apps: one package, `wolos-webapps` in wolos-pkgbuilds (new recipe; the wolos repository is its source, like `wolos-system`): `package()` copies `webapps/kit` and every `webapps/<slug>` to `/usr/share/wolos/webapps/`, leaving out the development files (`mock-bridge.js`, every `fixtures.js`), and installs each app's launcher entry, `Manifest::desktop_entry`, as `/usr/share/applications/org.wolos.webapp.<slug>.desktop`. The entries are to be generated into `files/usr/share/applications/` and test-checked against the manifests (regenerated with `WOL_REGEN=1`, like the web-runtime flag files), so the recipe only copies; that test lands with the package. It depends on `wolos-system` (wol, wold) and on the runtime's package (`webengine-wolos` and the host).
- The launcher entry: `Exec=/usr/bin/wol-webhost /usr/share/wolos/webapps/<slug>/app.toml`, `Icon=` the app's icon, `StartupWMClass=` and window class = the app id, so the shell matches the window to its launcher and the Memory Saver its scope. The host loads the manifest, builds the policy, starts its scope (ADR 0042), opens a Wolkit window titled with `name`, sets the channel with the `init` record and loads `entry`.
- Apps of others as a `.wol`: a `.wol` payload is a pacman package (ADR 0011, 0021), so a third-party web app is the same folder in a package that installs to `/opt/wolos/webapps/<slug>/` (never under `/usr/share/wolos/webapps`; a wopkg rule to add refuses that path in a non-system package), with the same launcher entry; its permissions need consent grants.

## Consequences
- No new privileged surface: every method is an existing verb, wold checks it again, and grants are the consent file's.
- A new tool app is one folder and a few catalog rows; a new method is a verb first.
- The look is one stylesheet, so every system app changes with the system's tokens.
- Until the shim has the channel and the host exists, apps run only in development mode; the bridge's Rust side and the contract are tested without the engine.
- The locale system later replaces `strings.js` per language; the keys are the contract.
- Wonderland Doctrine: app text says what the app does; no upstream names.

## Alternatives rejected
- A bundler and npm packages per app: fast to start, but a build per change, a dependency tree in the image and supply-chain risk for system apps.
- Shadow-DOM components: style isolation the kit does not need, harder theming through one stylesheet, and more bytes.
- Permissions as raw verb names: an app would list dozens of verbs, and Settings could not explain them in a sentence.
- A local HTTP server per app: a port, a process and an origin to defend, for what `file:` and the origin gate already give.
- Granting by the manifest alone: an app could grant itself anything by editing a file; only wold grants.
