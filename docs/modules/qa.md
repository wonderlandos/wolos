# QA: end-to-end journeys of every shipped app and flow

Unit tests prove code. These journeys prove the product: a real WolOS install boots in a VM, a
script clicks, types and waits like a person, and every step leaves a screenshot, a timing line
and a pass or fail. Tooling lives in `wolos-iso/tests` (`tests/e2e.sh`, `tests/wolvm/wolvm`,
`tests/wolvm/journeys/e2e-*.yaml`); the VM layer is described in `docs/modules/vm-e2e.md`.

## What runs

`tests/e2e.sh [-r RUNS] [-j PARALLEL] [-k] [-o OUT] [JOURNEY...]` runs on the forge (KVM). For each
journey and run it makes a copy of one kept install (`wolvm clone`, a reflink: instant, the original is
never written), starts it, restores the whole-VM snapshot `desktop` (a logged-in desktop; journeys that
need a boot start cold with `from: cold`), replays the journey, stops and deletes the copy. A journey
that passes in every run is PASS, fails in every run FAIL, and mixed FLAKE. `OUT/index.html` is the
table (links to every run's `index.html`, a contact sheet `contact.png` with all screenshots, steps over
1 s); `summary.json` and `summary.md` carry the same.

`tests/wolvm/e2e/make-base.sh` builds the base from the install a release run keeps
(`/scratch/rel012/vm/inst-default/test/wolvm`, only read). An autoinstall makes no account, so the script
uses the hand-made `tester` of that run, adds it to group `seat` (without it the compositor dies in a
restart loop and the screen stays black) and sets its password to `tester`.

| Journey | What it proves |
|---|---|
| login-unlock | cold boot to the lock screen, wrong then right password, lock with Ctrl+Super+Q |
| setup-assistant | an install with no account shows the Setup Assistant; every page; the new account's first desktop |
| settings-pages | all 25 sidebar panes and 9 subpages open (`wol-settings "<page>"`), repaint, no panic; one sidebar click and the search box |
| settings-persist | change a switch, reboot, the value and the switch are still there |
| control-center | tiles: Dark Mode, Focus, Stay Awake, Sound, Screenshot, Record, Wi-Fi |
| store | Store opens, search, Kits, Plugins, Updates, Community, Get |
| modules | Settings > Modules, install and remove the 20 MB Sessions module |
| files / gallery | open from the Dock, new folder on disk; Photos import dialog |
| terminal | Dock to window, bash, zsh, a shell installed on the spot (fish) |
| notifications | banner, Notification Center, Focus mutes banners |
| clipboard | copy in an app, panel, history, Clear |
| theme-picker | Ctrl+Super+T, apply live, theme id on disk |
| night-shift | toggle, state, tint on screen |
| screenshot-record | shortcut writes a PNG of the right size, capture bar, Esc, recording bar |
| memory-saver | switch on, web app window, backgrounded, memory given back |
| update-check | Check Now, `wol update status` |
| plugin-maker | create, check, package, enable a plugin, see it in the menu bar |

## Step vocabulary added for this

`assert-screen` and `wait-screen` (not blank, region luminance, pixel colour, percent of pixels that
changed from `prev`, a step number, or `1` = the picture before the journey; needs Pillow on the host),
`wait-ui` (window or layer surface by regex, `layer: true`, `absent: true`; polls `hyprctl` in the guest
with a monotonic clock), `wait-cmd` (poll a guest command, report the wait in ms), `push` (copy a file
into the guest). Options for every step: `budget: S` marks a step SLOW over S seconds, `max: S` fails it,
`soft: true` records a known defect without failing the journey (shown as "defect (soft)" in the table).
Commands as another user (`user: tester`) get the desktop session environment automatically.

## Traps found while building it

- A restored snapshot makes the guest wall clock jump forward; timeouts measured with `time.time()` in the
  guest fire at once. Use monotonic clocks (the verbs do).
- A window is mapped before it is painted: `wait-ui` says "open", `wait-screen` says "drawn". Both are timed.
- A click right after a page change is sometimes ignored (seen once in `settings-persist`); the journey
  waits for the page to be drawn first.
- `wl-copy` from the agent never exits (it forks a daemon holding the pipe): copy inside an app instead.
- Cold boots show the console text on screen because wolvm adds `console=tty0`; that is the harness, not the image.

Results and the ranked defect list of the v0.12 run: `.claude/v012/e2e/REPORT.md`.
