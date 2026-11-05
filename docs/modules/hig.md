# WolOS Human Interface Guidelines (binding)

Every app and shell surface follows this. One look, many themes (theme contract, IaraOS ADR 0016). A theme changes tokens, never structure.

## 1. Structure: one scaffold

Every app window is `AppWindow` (`crates/shell/src/kit`). No app builds its own window layout.

```
┌──────────────────────────────────────────────────────┐
│ [‹ ›]                             [actions] [search] │  Toolbar (40px, glass, draggable), under the 28px compositor strip
├────────────┬─────────────────────────────────────────┤
│ Sidebar    │ Content                                 │
│ 240px      │ (List / Grid / Detail / Editor)         │
│ glass      │                                         │
│ Sections   │                                         │
├────────────┴─────────────────────────────────────────┤
│ Status bar (optional, 24px, dim text)                │
└──────────────────────────────────────────────────────┘
```

| Part | Rule |
|---|---|
| Toolbar | Title left after nav buttons; primary actions right; SearchField rightmost. Max 5 actions; rest in a `…` Menu. Icon-only buttons need a Tooltip. |
| Sidebar | Only for navigation (places, pages, libraries). `SourceSection` headers in small caps dim text; `SourceItem` rows 24px high with a 4px gap (a pitch of 28) and a 16px symbolic icon. Selection = accent tint pill. Collapsible (Super+Alt+S). |
| Content | One primary view. Empty → `EmptyState` (icon, title, one-line hint, one action). Loading > 150 ms → `Spinner` centred; never a blank pane. |
| Status bar | Counts and quiet facts only ("12 items, 40 GB available"). No actions. |
| Sheets | Modal questions (save changes, conflicts, authenticate) slide from the toolbar as a Sheet. Never a floating dialog for app-modal questions. |
| Settings-style apps | Sidebar of pages + content of grouped `Section`/`List`/`ListRow` cards; controls right-aligned in rows. |

Single-purpose utilities (Calculator, Clock, Screenshot) use `AppWindow compact`: toolbar only, no sidebar, fixed size.

### 1.1 The numbers (one table)

The sizes of the scaffold, in px at scale 1 (every length is a whole number of 4 px fen). Each row names the constant that holds it; `crates/shared/tests/hig_numbers.rs` reads this table and fails when it, `shared::theme::scaffold`, the design repo's `brand-screens.md` and `brand-ui.md` or `kit.css` disagree. Change a number here and in the constant together.

| Part | px | Held by |
|---|---|---|
| Toolbar | 40 | `KIT_TOOLBAR_HEIGHT` (the Kit toolbar, under the strip) |
| Compositor strip | 28 | `titlebar::HEIGHT` in `crates/realtime`, above the toolbar: the compositor's surface, not the Kit's |
| Sidebar width | 240 | `SIDEBAR_WIDTH` |
| Sidebar row (`SourceItem`) | 24 | `SOURCE_ROW`; 4 px gap, so the pitch is 28 |
| Status bar | 24 | `label.status-bar` in `kit.css`; dim text, no actions |
| List row | 40 | `LIST_ROW` (the contract; the Kit draws 52 until the design system fixes it) |
| Control (button, entry, dropdown, menu item) | 24 | `CONTROL_HEIGHT` |
| Switch | 36 x 24 | thumb 20, padding 2 (3:2); in a Control Center detail header 24 x 16, thumb 12 |
| Third-party app header bar | 52 | `TOOLBAR_HEIGHT`: the header bar the generated GTK 3/4 themes give apps we do not own; not a Kit surface |

The switch is the design number. GtkSwitch is two thumbs wide, so the Kit draws it 44 wide (and 28 high today); ADR 0058 (proposed) freezes that gap as a known deviation until a libWolkit-drawn switch replaces it. The brand documents (`brand-screens.md` section 1.1, `brand-ui.md` section 2) say 36 x 24 too.

### 1.2 Density (proposed, ADR 0058)

Density is the one setting that changes heights and gaps; type, icons (16), colour and radius never change with it. A component never picks its own density: the window carries the class (`density-compact`, none for standard, `density-comfortable`) and Settings > Appearance chooses it. Standard is what ships today and is the table in 1.1. Compact and comfortable are proposals until the lead approves them; nothing draws them yet.

| Metric | Compact | Standard | Comfortable |
|---|---|---|---|
| List row | 32 | 40 | 48 |
| Control height | 20 | 24 | 32 |
| Sidebar row pitch | 24 | 28 | 32 |
| Toolbar | 32 | 40 | 48 |

Compact never makes a pointer target under 20 px and is never the default on touch. Larger text wins over density: a row is never shorter than its text line plus padding.

Density applies to the shell and to apps built on the Kit. Stock GTK and Qt apps follow the theme's colours and shapes through the generated app themes, not density, and libadwaita widgets in them ignore it.

### 1.3 The spec

Each component has a spec file in the design repo (its components folder, one TOML file per component, validated by `components.py`) with its anatomy, sizes per density, states, keyboard, accessibility and rules. The spec is the reference; the wolos.org/design pages and the Kit widget tests are generated from it. This document keeps the rules that apply to every surface; a number that belongs to one component lives in its spec.

## 2. Tokens, never literals

- Colours: `--surface --card --text --dim --line --accent --accent-text --control --glass*`. No hex/rgb in app code or app CSS.
- Spacing scale: 4, 8, 12, 16, 24 (`shared::theme::SCALE`, `--space-1` to `--space-6`). Nothing else.
- Radius: controls 8, cards 12 (`--glass-radius-s`), windows/panels `--glass-radius`.
- Type: Inter. Title 13 semibold (toolbar), body 13, secondary 11 dim, heading 16, large title 27 bold (page titles; `shared::theme::scaffold`), 24 semibold inside an empty state; every size sits on a 4 px line (11/16, 13/20, 16/24, 24/36, 27/40). No other sizes.
- Screen chrome: the installer and Setup show the door mark and the word WolOS at the upper left (60 px from the edges) only on a screen at least 968 px wide (`CHROME_MIN_WIDTH` in `ui/lock.rs`). Below that the centred 624 px card reaches the mark and the two overlap.
- Icons: symbolic, 16 in rows/toolbar, 32/48/64 only for files and app tiles.

## 3. Behaviour

- Shortcuts (Super = ⌘): N new, O open, S save, Shift+S save as, W close window, Q quit, F find, Z/Shift+Z undo/redo, C/X/V/A, comma settings, Shift+E share (File > Share…: the selection goes to the share sheet, `wol-share`; an app with nothing to share greys the row), Backspace delete/trash, I info, Space Quick Look (file lists). Same key, same meaning in every app.
- Selection: click selects, double-click/Enter opens, Shift/Super extend. Right-click → context `Menu` with the same verbs as the toolbar.
- Destructive actions: confirm in a Sheet with the destructive button on the right in `destructive` style; prefer undo over confirm.
- Long work (> 1 s): progress in place (row ProgressBar or toolbar Spinner), cancellable, off the UI thread. Never freeze.
- Errors: inline, one sentence, what happened + what to do. Root-only actions: the Authenticate sheet (admin), never "run as root".
- Windows remember size, position, sidebar width, view mode (per app, `$HOME/.config/wolos/<app>.json`).
- Text: sentence case labels, title case only for app names and menu titles. No exclamation marks. Ellipsis "…" when the action asks for more input.

## 4. Do not

- Re-implement a library component (kit architecture test enforces).
- Hand-roll toolbars, sidebars, headers, cards, list rows, empty states.
- Add app-local CSS beyond layout glue; visual style lives in the library.
- Invent new spacing, colours, font sizes, icon sizes.
- Put actions in the status bar or navigation in the toolbar.

## 5. Checklist before commit (each app)

1. Root view is `AppWindow`.
2. Golden test at 900×600 and narrow 480×360 without overflow (kit golden test).
3. Empty, loading and error states rendered in tests.
4. Shortcuts table above wired.
5. VM screenshot compared side by side with Files and Settings: same toolbar height, sidebar width, row height, fonts.

## 6. Layout check (binding, from IaraKit)

| Rule | Fires when |
|---|---|
| Overflow | child box extends past its parent (scroll content exempt) |
| Row alignment | item in a horizontal box is > 1 px off the row's vertical centre |
| Clipped text | label ellipsized or cut where the HIG does not allow it |
| Insets | a glass panel or Control Center module insets its drawn content unequally on any side, or a card/sheet/list has left != right or top != bottom (> 1 px). Measured through layout-only boxes, so a stretched card with centred rows fails. Panels and modules pad with the one `--inset` token. |

`WOL_DEBUG_LAYOUT=1` outlines widgets; `WOL_LAYOUT_CHECK=exit` exits 1 on any issue (CI and golden tests).
Every surface (menu bar, dock, Control Center, Search, lock, login, every Settings page) has golden screenshots at its sizes, passes the layout check, and was visually reviewed by the author before commit.

## 7. Motion

Motion says where a thing came from and where it went; it never makes the user wait. One library serves every shell surface: `crates/shell/src/kit/motion.rs` (curves, durations, springs, stagger: pure arithmetic) and `crates/shell/src/kit/anim.rs` (the frame clock and the `Animated` handle). The names, control points and durations are the theme contract that libWolkit apps use (`crates/wolkit/src/style/value.rs` `Ease`, `crates/wolkit/src/anim/mod.rs` `DURATIONS_NS`), so a shell panel and a Wolkit app move alike. `crates/shell/tests/motion.rs` holds both to this section.

**Durations** (`kit::motion::Duration`, kit.css tokens):

| Token | ms | For |
|---|---|---|
| `--motion-fast` (`Duration::Fast`) | 150 | hover, press, small toggles, anything leaving |
| `--motion-normal` (`Duration::Normal`) | 250 | panels, popovers, banners entering, list rows, the Dock hover |
| `--motion-slow` (`Duration::Slow`) | 400 | large surfaces: Overview, sheets, the wallpaper cross-fade |

Nothing in the shell animates longer than 400 ms, and nothing waits on an animation to accept input.

**Curves** (`kit::motion::Curve`, `Curve::at(t)` gives progress, `Curve::css()` the timing function):

| Name | cubic-bezier | kit.css | Use | Compositor |
|---|---|---|---|---|
| `ease` (`Curve::Ease`) | .2, .8, .2, 1 | `--ease-out` | entering and moving: fast start, soft landing (the default) | `fadeIn`, `layers` (in), `fadeLayersIn`; minimize to the Dock (slow) |
| `in` (`Curve::In`) | .4, 0, 1, 1 | `--ease-in` | leaving: slow start, gone at full speed | `fadeOut`, `layersOut`, `fadeLayersOut` (fast) |
| `out` (`Curve::Out`) | 0, 0, .2, 1 | | a gentler entrance for large surfaces | defined, unused |
| `in-out` (`Curve::InOut`) | .4, 0, .2, 1 | `--ease-in-out` | a surface that moves and stays on screen | defined, unused |
| `linear` (`Curve::Linear`) | none | | progress and value-driven fades; never for movement | built in |
| `glass` (`Curve::Glass`) | .34, 1.56, .64, 1 | `--glass-spring` | outside the contract: a glass panel landing with a strong overshoot (peaks near 1.1) | `windowsIn` under Effects & Looks Pop |
| `overshoot` (`Curve::Overshoot`) | .22, 1.2, .36, 1 | | outside the contract: a light overshoot (window moves; the 0.56 compositor's `spring`) | `windowsMove` |
| `soft` (`Curve::Soft`) | .25, .1, .25, 1 | | outside the contract: fades and workspace slides (the 0.56 compositor's `ease`) | `fade`; `workspaces`, `specialWorkspace` (slow) |

The first five are the theme contract (`Curve::CONTRACT`, libWolkit's `Ease`). The last three name the curves already in use elsewhere so the compositor and the shell map onto one table; new shell code picks from the contract, and `glass`/`overshoot` are not monotonic. Enter with `ease`, leave with `in` one step faster (a panel opens in 250 ms and closes in 150 ms). `Motion::enter(d)` and `Motion::leave(d)` are those two.

**Springs** (`kit::motion::Spring`) are for drops and drags only: things the hand moved and lets go (a dragged tile landing, the Dock magnify following the pointer, window open and close). A click or a hover is a curve, not a spring. Settle is the time a 0-to-1 move from rest takes to stay within 2% of its target.

| Spring | Stiffness | Damping | Mass | Overshoot | Settle (2%) |
|---|---|---|---|---|---|
| `gentle` | 170 | 26 | 1 | none (ratio 1.0) | 450 ms |
| `snappy` | 320 | 30 | 1 | slight (ratio 0.84) | 230 ms |
| `bouncy` | 260 | 14 | 1 | visible (ratio 0.43) | 520 ms |
| `window-open` | 385.5 | 31.4 | 1 | slight (ratio 0.80) | 200 ms |
| `window-close` | 322.3 | 30.5 | 1 | slight (ratio 0.85) | 240 ms |

`gentle`, `snappy` and `bouncy` are libWolkit's `spring:` presets; `window-open` and `window-close` are `kit::motion::window::OPEN`/`CLOSE` (`.spring()`). `Spring::step(state, target, dt)` is the integrator (240 Hz semi-implicit Euler, a stalled frame counts at most 250 ms).

**The compositor** moves windows, layers and workspaces on the same table. `crates/realtime/src/anim.rs` holds it on the compositor side: `Curve` (these eight names and control points), `Duration` (150/250/400 ms), `Spring::WINDOW_OPEN`/`WINDOW_CLOSE` (the `window-open`/`window-close` springs above, solved in closed form as the compositor does: `windowsIn` and `windowsOut`, both `popin 90%`) and `LEAVES`, which animation runs on what (the Compositor column). `anim::lua()` renders the animations block of `desktop/overlay/usr/share/wolos/hypr/hyprland.lua`: one `hl.curve` per curve but the built-in `linear`, one spring `hl.curve` per window spring, one `hl.animation` per leaf; `crates/realtime/tests/hypr_conf.rs` holds the shipped file to it, and the compositor's `--verify-config` accepts it. The rule is the shell's: enter with `ease` at normal, leave with `in` at fast, window moves `overshoot` at normal, workspaces `soft` at slow, window open and close on their springs (a spring ignores the `speed` field; the file carries its settle time there). The plugin and other C code read a curve by name through `wol_anim_curve(name, out)` and list the names with `wol_anim_curve_name(i)` (`crates/realtime/include/wol_realtime.h`). `anim::window_in` is Effects & Looks Motion on this table: Zoom `popin 90%` and Slide `slide` on `window-open`, Pop `popin 80%` on `glass` at normal (`crates/core/src/looks.rs`, held to it by the realtime test). A spring has no speed, so at an Effects & Looks speed other than 1 windows open on `ease` and close on `in` over the springs' settle time scaled by the speed.

**Stagger** (`kit::motion::Stagger::new(n, gap_ms)`): rows of a list enter one after another, `Stagger::GAP_MS` (30 ms) apart, first row first (`delay_ms(i)`); they leave last row first (`delay_rev_ms(i)`). The whole spread is capped at 300 ms (`Stagger::MAX_SPREAD_MS`): a long list shrinks the gap and keeps the order. Banners, Notification Center rows and Control Center tiles stagger; menus and the Dock do not.

**The clock and `Animated`.** Hand-made motion goes through `kit::anim::Animated`, one number (opacity, a translation, a scale) per handle:

```rust
use shell::kit::anim::{Animated, Motion};
use shell::kit::motion::Duration;

let fade = Animated::new(&panel, 0.0, {
    let p = panel.clone();
    move |v| p.set_opacity(v.into())
});
fade.connect_settled({ let w = window.clone(); move |v| if v == 0.0 { w.set_visible(false) } });
fade.retarget(1.0, Motion::enter(Duration::Normal));   // open
fade.retarget(0.0, Motion::leave(Duration::Fast));     // close, from wherever it is now
```

- **One clock per window.** `Animated::new(widget, ...)` joins the clock of the widget's window (`Clock::of`): one `add_tick_callback` on the root, installed while something moves and removed when nothing does. Create the handle after the widget is in its window. `Animated::on_clock` takes an explicit clock (several widgets in step).
- **Interruptible.** `retarget` may come at any time: the value turns around from where it is (no jump back), a spring keeps its velocity (a tween hands its speed to a spring), and a retarget to the target it already heads for changes nothing, so a second hover or toggle never restarts the move. `retarget_after(target, motion, delay_ms)` takes a stagger delay; `set(v)` jumps; `finish()` ends the move at its target.
- **`settled`** (`connect_settled`) runs once each time the value reaches its target, after the last `apply`: hide the window there, not on a timer.
- **Nothing stuck.** When the window unmaps, its clock finishes every animation (values at their targets, `settled` runs).
- **Reduce motion.** With Accessibility > Reduce motion on (`kit::motion::reduced()`, which also turns off GTK's CSS animations), every `retarget` jumps to its target at once and stagger delays are ignored; layout and state end the same, only the frames between go.
- **Larger text.** Accessibility > Larger text sets the desktop's text scale, gsettings `org.gnome.desktop.interface text-scaling-factor` 1.25 (off = 1), which GTK and Qt apps follow. The kit draws its type in px, which no toolkit scale reaches, so `kit::style` multiplies every `font-size` and `line-height` in px of the style sheet by the scale and restyles at once when the key changes, whoever changes it (QA-ACC-5). Scales below 1 are ignored; above 3 are cut to 3.
- **Give me more time.** Accessibility > Give me more time (`$XDG_CONFIG_HOME/wol/timeouts`: `2`, `5` or `never`, no file = normal; `shared::patience`) stretches every wait the shell puts on a person, read through `kit::patience::current()` when a timer starts: the approval sheet (50 s), banners (5 s, never = until dismissed), toasts, the screenshot thumbnail, the copied-code clipboard clearing (60 s, at most ten minutes) and the Keep countdown of a changed display mode (15 s, at most two minutes: a picture nobody can read is put back). The waits behind a sheet stretch with it: `wol` (`controller::approve`) and wold (`api::portal::patience_of`, which reads the account's own small regular file only) wait for the sheet's stretched time plus ten seconds, never past `shared::patience::NEVER_SECS` (an hour). A web app's call (`WEBAPP_CALL_DEADLINE_SECS`) keeps its own deadline (QA-ACC-6).
- **Tests and goldens** use `Clock::manual()` and drive time with `clock.step(now_ns)`; `clock.set_reduced(Some(true))` checks the reduced path.

**CSS.** Transitions and keyframes in kit.css use the tokens above and move `transform` and `opacity` only, never margins or padding (a margin per frame is a relayout of the whole window). The Dock bounce and the lock-screen shake are `translateY`/`translateX` keyframes.
