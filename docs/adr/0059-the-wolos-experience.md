# 0059 - The WolOS experience: the integration surfaces every app provides

Status: accepted (lead order 2026-11-04: "integrate every app, rewrite what the apps need to integrate the experience more; WE NEED TO CLOSE THE EXPERIENCE END TO END LIKE MAC, BUT ON LINUX; Atlas is important too"). Drafted by APPS-E2E, promoted to the repo by E2E-CONTRACT; accepted by the CTO 2026-11-04. The matrix it decides is data in `docs/modules/experience.md` and is enforced by `crates/controller/tests/experience.rs`. Builds on 0020 (libwol), 0026 (Memory Saver), 0031 (themes), 0032 (sessions), 0036 (Crystal userland), 0039 (libwolkit and wolkit), 0043 draft (Atlas), 0048 (system web apps), 0054 draft (keyring), 0057 (search by meaning). Wonderland Doctrine: this text names no upstream project except the stock GNOME apps the image ships today, which are listed in `docs/modules/apps.md` anyway; the public version should say "the stock apps" and keep the names in the table.

Provenance: the state of each surface and every number come from reading the code today (the experience map carries file:line); the lead orders and the Atlas, Crystal and web-app decisions come from memory (#920, #947, #1133, #1178) and the ADRs above. The options and the premortem are written for this draft.

## Context

- The image ships 18 things the lead calls apps. Five are ours in Rust/GTK or as a web app, two are ours in Crystal (one merged, one on a branch), and eleven are stock apps or a repackaged browser: the bundled set is mostly other people's GTK apps (`ISO/profile/packages.desktop:75-89`).
- The shell is ours, so every system surface (menu bar, Dock, Search, Share, notifications, clipboard, theme) is ours to complete. Today seven of 28 surfaces are complete, fourteen partial, seven absent (the experience map, section 1).
- The failure the lead named at 22:17Z is the pattern: a surface "integrated" once, at session start, and wrong afterwards. The compositor's title strip was coloured by `wol theme apply` at login; the Auto light/dark flip updated only the shell's own windows, so the App Store was dark under a white strip (root cause and fix in the experience map, section 8).
- The HIG (`docs/modules/hig.md`) binds our apps to one look and one shortcut table; it says nothing about which system services an app must offer or use, and it promises two things no code delivers (Space = Quick Look, windows remember their size).

## Options considered

1. **Layered: system services for every app, verbs and an AppKit for native apps, rewrites in Crystal by debt per hour (chosen).** The shell gives stock apps what a platform gives them; apps we write get the same through one library; we rewrite an app only where integration debt is highest and the toolkit can carry it.
2. **Rewrite every bundled app natively before anything else.** The truest result and the one the lead's words suggest; rejected as the plan: about 1,000 worker hours for Files, Mail, Calendar, Text Editor and Terminal alone, blocked on toolkit work that has not been done (multi-line text, drag and drop), and the first-hour experience would not change for weeks.
3. **Patch or fork the stock apps (menu export, share, search providers).** Every update then costs a rebase of apps we do not own, against ADR 0029's single build path and the licence story; the Mac advantage (one vendor owns the apps) is exactly what we do not have. Rejected, except for our own fork of the browser (ADR 0043), which is a separate decision.
4. **Make the apps web apps on the Rust engine (ADR 0048).** Fast to build, uniform look, bridge permissions fit the consent model; but rich editing, media and PDF rendering depend on the engine's maturity, and the lead's order of 2026-11-03 is Crystal on Cryswol/Wolkit for the bundled apps. Kept as the fallback for one app (Notes) and as the shape of system tools (Doctor).
5. **Do nothing beyond fixing bugs.** The stock apps stay; the bar stays without menus, Space does nothing, Search finds names. Rejected: it is the state the lead is unhappy with.

Premortem of option 1, written as if it failed in three months: (a) the synthesised menus claim actions the app does not honour, people stop trusting the bar (mitigation: rows are disabled where the hand-off table says `Nothing`, and a unit table pins six classes); (b) services that work only with Files (Quick Look through the previewer name) look like a hack next to Search and Atlas (mitigation: stated limits, follow-up hooks); (c) the Crystal apps arrive late because multi-line text and drag and drop are long poles (mitigation: those two start first, the apps that need neither go first, Notes has a web fallback); (d) DESIGN-SYSTEM and TILING change the same shell files and the packages collide (mitigation: hot-file owners in the gap plan, section 4, KIT and A11Y wait for DESIGN-SYSTEM); (e) the journeys cannot run on the build box and drift (mitigation: lane tests are the bar for done, the forge run for proved, one manifest); (f) the contract rots into prose (mitigation: D1's test).

## Decision

**D1. The Surfaces are a named, numbered list kept as data.** The 28 surfaces S01..S28 move to `docs/modules/experience.md` as a table (surface, tier, owner package, journey). Every shipped app's launcher entry carries `X-WolOS-Surfaces=` (ids it provides). A test in `crates/controller/tests/experience.rs` fails when a row has no journey, when an entry claims an id that is not in the table, or when the table claims a surface for an app whose entry does not list it. Adding a surface is a row, an owner and a journey.

**D2. Three tiers, one rule.** Tier 1: **system-provided**, works for any app including stock ones (standard menus synthesised from the `wol-app` keys, Quick Look, Services, Share targets, Default Apps, drop-on-Dock, Search through the provider interface, spell check, colour picker, clipboard, media keys, notifications, theme). Tier 2: **declared**, the app tells the system through standard desktop-entry keys, `org.wolos.Menus`, search-provider files or libwol verbs. Tier 3: **native**, an app on Wolkit or the kit gets its Tier 2 declarations from one library (`Wolkit::AppKit`, `kit::appkit`, `wolkit.js`) and writes only its content. **Rule: provide a surface in the system when the system can; in the app only when the content is the app's.**

**D3. Never patch a stock app.** If a surface needs the app, the app joins the rewrite list or the gap is documented in the table. Atlas is the exception because it is ours (ADR 0043).

**D4. Standard keys first.** Desktop-entry `MimeType`, `Actions`, `x-scheme-handler/*` and the GNOME search-provider interface are read before any WolOS key is invented. WolOS keys are prefixed `X-WolOS-` and listed in `docs/modules/experience.md`: `X-WolOS-Surfaces`, `X-WolOS-Share` (what the app accepts through Share: `text;url;file`), `X-WolOS-Service` (selection verbs the app offers).

**D5. Appearance is one event.** Every change of the effective theme, whoever causes it (the Look page, Settings, Control Center, Looks, an import, **the Auto switch at 07:00 and 19:00**), reaches the compositor strip, the toolkit files, gsettings, foot and the `theme` event through the same path, `wol theme apply` / `restyle_apps`. The main shell follows the clock for Auto and runs it when the variant changes (`crates/shell/src/appearance_sync.rs`, built in the worktree `wt/apps-e2e`); a test pins that the strip is the theme's `surface` for every theme and variant, and that an Auto sweep over 48 hours never leaves the strip on the other variant after a tick.

**D6. One title standard (proposal, needs DESIGN-SYSTEM and the lead).** A window has one title row. For windows with the compositor strip (Kit, Wolkit, server-side), the strip carries the lights and the title and is filled with the same token as the window's toolbar, so the strip and the toolbar read as one bar; today the strip is `surface` at 90 % and 28 px (`crates/core/src/theme/hypr.rs:27`) and the toolbar is `glass` at 40 px (`crates/shell/src/kit/kit.css:215`). Windows that draw their own frame carry the `wol-lights-none` tag (`crates/realtime/src/chrome.rs:91`). A window whose content can differ in variant from the system (a browser theme) must either follow the system variant or draw its own frame; the test of D5 plus a golden that takes a window screenshot with the strip pins it.

**D7. A bundled app is replaced only by a better one.** A stock app is replaced by a Crystal app when the replacement passes: the HIG checklist (`docs/modules/hig.md` section 5), every surface the old one had plus the ones in its app-table row, goldens at both sizes, one journey, and feature parity on the five things people do with it (named in the app's `DESIGN.md`). Until then the stock app stays and the system services serve it. Order: Gallery (finish), Notes, Music, Calculator, Documents, then the rest as the toolkit allows; Terminal stays foot, Files stays Nautilus and Mail stays Geary for 0.13 (decisions D1, D2, D5 for the lead in the gap plan).

**D8. The first day is one journey and a release gate.** `e2e-firstday` has twelve checkpoints (install, Setup in a non-English language, first sign-in, Look chosen and applied everywhere, Quick Look, Atlas first run, a link from another app, Store install, Update Now, restart with the session offered back), run on the forge with screenshots before each release; a red row blocks the release unless the lead waives it.

**D9. Journeys are the acceptance.** Each package ships `e2e-<name>.yaml` in `tests/wolvm/journeys/` and lists it in `tests/wolvm/e2e.manifest`; the lane (containers, no KVM) runs the unit and golden tests; the forge runs the journeys.

**D10. Out of scope, said so.** Phone clipboard, Handoff and Continuity, nearby sharing, Universal Control, cloud sync of files, settings or the keyring, a voice assistant, payments, Screen Time. They need services we do not run (ADR 0033).

## Consequences

- The shell gains seven system services and a library per toolkit; every app we ship gains a declaration file's worth of work, not a rewrite.
- Stock apps keep their quirks (hamburger menus, their own search) but sit under the same bar, the same Share, the same Search and the same Quick Look.
- The Crystal apps depend on four toolkit packages (multi-line text, drag and drop, the 1.16 verbs, AppKit); the plan schedules them first and says what is not possible in seven days.
- The HIG gains two enforced promises (windows remember size and Space is Quick Look) or loses them; D1's test makes the choice visible.
- The ISO grows by about 37 MiB compressed (measured on the 2026-11-02 package archive, whole dependency closure of the image list; Gallery's 6 MiB is an estimate): dictionaries and their engine 7.3 MiB, the Portuguese list 1.3, the colour picker 0.1, the screen reader with its speech voice and braille support 22.7, Gallery with its two libraries 6 (the PDF tools are already in the image through the document viewer's library). Nothing leaves the image for it: the image has no size limit (lead, 2026-11-04), `build.sh` only prints the size, and an ISO over GitHub's 2 GiB asset size is published on the CDN with a link from the GitHub release. The rows are in `docs/modules/experience.md`, section Image size.

## Decisions needed from the lead

1. Rewrite order: Gallery, Notes, Music, Calculator, Documents (recommended) or Gallery, Documents, Music, Notes (2026-11-03).
2. Mail: keep Geary (recommended) or fund a Rust mail library plus a Crystal client.
3. One title bar (D6): approve the direction so DESIGN-SYSTEM can fold it into the system.
4. Notes native on a new multi-line text engine (A) or as a web app (B) if the engine slips.
5. Terminal stays foot, Files stays Nautilus for 0.13.

## Not verified

No VM was run; the stock apps' behaviour (previewer name, search-provider files, MPRIS, DnD) is from knowledge of those apps and is marked [V] in the experience map; hours are estimates calibrated on Gallery and Looks.
