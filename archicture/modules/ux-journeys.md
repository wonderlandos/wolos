# UX journeys (audit 2026-10-27)

Acceptance: `site/src/pages/Thesis.jsx` (Calm, Instant, Obvious, One look, No homework, Yours, Safe to tinker, AI-friendly not forced).
Verdicts: OK, STUCK (no way forward without help), JARGON, TERMINAL, SLOW (> 1 s, no progress/cancel), NO-FEEDBACK (> 150 ms silent, or no confirmation).
Tests: `crates/shell/tests/journeys.rs` (J1 J2 J3 J4 J8 J11), `crates/shell/tests/copy_lint.rs`.

## Error copy: one place

| Rule | Where |
|---|---|
| Every error sentence comes from `shell::copy` (`action`, `wifi_join`, `install`, `update`, `undo`, `UPDATE_CHECK`); `ActionError::message` is `copy::action` | `crates/shell/src/copy.rs` |
| `copy::lint`: no `BANNED` fragment (failed, error, errno, os error, exit code, program names, pkg, wlan, root…), no paths/flags/backticks, capital start, full stop, no `!`, <= 140 chars, <= 2 sentences | `copy_lint.rs` |
| No `{e}` / `e.to_string()` outside `eprintln!` in `crates/shell/src` | `copy_lint.rs::no_raw_error_text_on_screen` |

## J1 First boot (USB -> desktop)

| Step | User sees / does | Feedback | Verdict | Owner |
|---|---|---|---|---|
| Live boot | black screen (`boot_mute`), up to 10 s Wi-Fi wait | none | NO-FEEDBACK | Image |
| Install WolOS (dock) | terminal window, bsddialog disk list (`ada0 465G`), "ALL DATA on ada0 will be erased" | text, counter | TERMINAL, JARGON | First-run |
| Install ends | `sysrc` lines, "Press Enter to close."; no restart, no "remove the stick" | none | STUCK | First-run |
| First boot | black; NVIDIA driver only online, before Wi-Fi exists | none | NO-FEEDBACK | Kernel/hardware |
| Setup: Language, Keyboard, Wi-Fi, Account, Appearance | 5 cards (11 answers incl. disk + erase) vs 4 promised | instant | OK (count over) | First-run |
| Keyboard card | check mark on U.S. after picking pt_BR; layout not applied while typing the password | - | STUCK risk | First-run |
| Wi-Fi card | list read once from an empty cache: "No networks found"; wrong password = success | text | NO-FEEDBACK | First-run |
| Create account fails | "Go back and check it" on a card without Back; "Setup is unavailable. Restart…" with no Restart | - | STUCK | First-run |
| Desktop | no Mail in the Dock | - | OK (gap) | Base apps |

## J2 Join Wi-Fi

| Step | User sees / does | Feedback | Verdict | Owner |
|---|---|---|---|---|
| Menu bar Wi-Fi icon | opens Control Center main page, then chevron | instant | OK | Control Center |
| List | `scan_results` cache, no rescan, no "Searching…" | none | NO-FEEDBACK | Control Center |
| Join (CC) | sheet closes, nothing until done (now up to 20 s) | none | NO-FEEDBACK | Control Center |
| Admin password | asked for every join and on/off (5 min cache); standard users cannot join | sheet | STUCK | Power/Security |
| wol-admind | not started by the image: every admin action = "This isn't available on this computer." | - | STUCK | Image |
| Wrong password | was silent success; now `WrongPassword` -> "“Home” didn't accept that password. Check it and try again." | sentence | OK (fixed, tested) | UX |
| Password `"`/`\` or < 8 | refused; now "Wi-Fi passwords have 8 to 63 characters…" | sentence | OK (fixed) / STUCK for `"` | UX / Settings |
| Hidden network, captive portal | not built | - | STUCK | Settings |
| No Wi-Fi card | "Not Connected", tile still clickable; hardware state only in `wol hw` | - | TERMINAL | Kernel/hardware |

## J3 Install an app

| Step | User sees / does | Feedback | Verdict | Owner |
|---|---|---|---|---|
| Find App Store | `wol-store` not built into the image, no Dock/menu/Spotlight entry | - | STUCK | Packages |
| Search | catalogue at once, `pkg search` later, no spinner, offline = "No results" | none | NO-FEEDBACK | Packages |
| Get | ring, no cancel, no % | ring | SLOW | Packages |
| Fails | was "X was not installed. The change did not work. Try again."; now `copy::install` ("…Check that you're online, then try again.") | sentence | OK (copy) / cause lost | UX / Packages |
| Cancel admin sheet | reported as wrong password | - | JARGON | Settings |
| Remove app | no Remove anywhere | - | STUCK | Packages |
| Double-click `.wol` | no MIME type, no handler, Files opens the folder | - | STUCK | Packages |

## J4 Change look, undo

| Step | User sees / does | Feedback | Verdict | Owner |
|---|---|---|---|---|
| Appearance Light/Dark/Auto + 8 accents | instant; Qt/libadwaita apps restyle on restart only | silent | OK / NO-FEEDBACK | App Look |
| 24 catalogue themes | `wol theme set` only; a catalogue theme shows as Auto/Violet | - | TERMINAL | Settings |
| CC Dark Mode | drops accent and Auto (writes `wol-dark`) | - | bug | Control Center |
| Wallpaper | `wol wallpaper set` and Settings > Wallpaper change the desktop at once (reload + file watch); Dynamic follows the clock | instant | OK | Shell |
| Preset ("rice") | not built | - | STUCK | App Look |
| Undo | journal + `System::undo` exist, no Undo button/toast/Restore page; `wol undo` only | - | TERMINAL | Settings |
| Undo errors | were raw (`wolos/theme changed since…`); now `copy::undo` | sentence | OK (fixed, tested) | UX |

## J5 Windows game via Steam

| Step | User sees / does | Feedback | Verdict | Owner |
|---|---|---|---|---|
| Linux compat | linux64 + linux_base + steam-utils + wine-proton, and every setting the client checks, in the image | - | OK | Games |
| Get Steam | Steam in Launchpad, Spotlight and the App Store (Open); `steam://` links open it | - | OK | Games |
| Steam first run | `wol games steam`: "Getting Steam ready" notification, `lsu-bootstrap`, then Steam's own updater window; offline: "Steam couldn't download…" | notification | OK (host-tested, not yet on hardware) | Games |
| Proton in Steam | WolOS Proton refreshed at every start; the user picks it once per game (Properties > Compatibility) | - | OK (one choice per game) | Games |
| Windows .exe | double-click runs it on wine-proton in its own prefix (`wol-run --exe`) | none while a new prefix is made | NO-FEEDBACK | Games |
| NVIDIA | `linux-nvidia-libs` not installed by `wol hw`; the notification points to `wol games doctor` | notification | TERMINAL | Kernel/hardware |

## J6 Plug in

| Device | What happens | Verdict | Owner |
|---|---|---|---|
| USB stick | autofs under `/media` on access; no notice, no sidebar entry, no Eject | NO-FEEDBACK | Base apps |
| Printer | CUPS + Avahi run; no Printers pane (`core::printers` unused) | STUCK | Settings |
| Bluetooth headphones | no scan/pair; Off may grey the tile for good; no A2DP | STUCK | Kernel/hardware |
| Second monitor | auto-extends; no arrange/mirror/keep-or-revert | OK / STUCK | Settings |
| Webcam | webcamd on plug-in | OK | - |

## J7 Update, roll back

| Step | User sees / does | Feedback | Verdict | Owner |
|---|---|---|---|---|
| Learn of updates | no check, no notification | - | NO-FEEDBACK | Packages |
| Check Now | spinner; was "Make sure pkg is installed…"; now `copy::UPDATE_CHECK`; `pkg version -U` never refreshes | spinner | OK (copy) / wrong result | UX / Packages |
| Available | "Updates install into a new boot environment." (jargon, untrue) | - | JARGON | Settings |
| Update Now | spinner, no cancel, 30 min cap; "Restart to finish updating." without Restart button | spinner | SLOW | Settings |
| Roll back | no Restore page; CLI with password on stdin; boot menu 2 s | - | TERMINAL | Settings |
| Restore points | 5 kept, shared with app installs: 5 installs delete the pre-update point | - | STUCK | Packages |

## J8 Things go wrong

| Case | What happens | Verdict | Owner |
|---|---|---|---|
| Disk full | no warning, no Storage page; Lock & Idle save drops errors | NO-FEEDBACK | Settings |
| App frozen | Quit = polite close; app menu "Force Quit <App>" and ⌥⌘Esc "Force Quit Applications" (list, Return quits; killwindow, tested) | OK | Shell |
| No network | "Not Connected"; online = default route; no next step | NO-FEEDBACK | Settings |
| Low battery | icon only; no 10 %/5 % banner, no auto-sleep, lid does nothing | STUCK | Power/Security |
| Unsupported Wi-Fi card | `/var/run/wolos-hw.json` written, read by no surface | TERMINAL | Kernel/hardware |

## J9 Screenshot

| Step | User sees / does | Feedback | Verdict | Owner |
|---|---|---|---|---|
| ⇧⌘3/4 | saved to the Desktop + clipboard; banner "Screenshot saved to Desktop" with the picture | banner | OK | Shell |
| Banner click | dismisses, does not open | - | NO-FEEDBACK | Shell |
| Super+Shift+5 | toolbar, thumbnail opens and drags | thumbnail | OK | - |
| Recording | no indicator; stop = reopen toolbar | - | STUCK | Shell |
| Non-English account | saves to new English folders | - | STUCK | Shell |

## J10 Spotlight

| Step | User sees / does | Feedback | Verdict | Owner |
|---|---|---|---|---|
| ⌘Space | new process per open | not measured | SLOW risk | Shell |
| Apps | ranked, 8 rows | instant | OK | - |
| Files | names under `$HOME` (no hidden, no build output), listed by the shell at login and hourly, again when Spotlight opens on a list older than 30 s; < 50 ms per keystroke on 200 000 names (tested) | instant | OK | Shell |
| Settings | every pane and sub-page under the name Settings opens; pane words ("dark", "menu bar") find it (tested) | instant | OK | Shell |
| No result | "No results for “…”" + "Search the web for “…”" (Return opens the browser) | sentence | OK | Shell |
| Click away | stays open with the keyboard | none | NO-FEEDBACK | Shell |

## J11 AI helper: Dark Mode, then revoke

| Step | User sees / does | Feedback | Verdict | Owner |
|---|---|---|---|---|
| Connect a helper | edit the AI client's MCP config to run `wol mcp` | - | TERMINAL | Automation |
| Sheet | "The helper “X” wants to switch to Dark appearance. Enter your password…" for a user-level change; no Allow Once / Always / Deny | sheet | JARGON | Automation |
| Answer | shell replies nothing; helper told "could not ask on screen" while the sheet is up | - | STUCK | Automation |
| Revoke | no Assistants pane; `wol helpers deny` only; revocation journaled, helper cannot undo it (tested) | - | TERMINAL | Automation / Settings |

## J12 Lock, sleep, log out, shut down

| Step | What happens | Verdict | Owner |
|---|---|---|---|
| ⌃⌘Q | lock | OK | - |
| Sleep | `acpiconf -s 3` without locking first | STUCK (security) | Power/Security |
| Shut Down / Restart / Log Out | immediate, no confirm, apps not asked to save | NO-FEEDBACK | Power/Security |
| Lock screen error | "Authentication unavailable" with no way out | STUCK | Power/Security |
| Lock crash | no respawn | STUCK | Power/Security |

## J13 Keyboard only

| Surface | Verdict | Owner |
|---|---|---|
| Control Center, Spotlight, approval, lock, setup, capture, switcher | OK | - |
| Menu bar ⌃F2, Dock ⌃F3 (arrows move, Return opens, Esc gives the keyboard back); banners: Esc or a swipe dismisses | OK | - |
| Notification Center: Esc closes; nothing focusable inside yet | NO-FEEDBACK | Shell |
| Super is ⌘: ⌘C/V/X/Z/⇧Z/A/S/O/P/F/G/T/W/N/R/L/+/−/0 reach the app as its Ctrl key (a terminal's Ctrl+Shift+C/V); no system shortcut takes a plain ⌘ letter (`hypr_conf.rs`) | OK | - |
