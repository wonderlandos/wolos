# WolOS Human Interface Guidelines (binding)

Every app and shell surface follows this. One look, many themes (theme contract, IaraOS ADR 0016). A theme changes tokens, never structure.

## 1. Structure: one scaffold

Every app window is `AppWindow` (`crates/shell/src/kit`). No app builds its own window layout.

```
┌──────────────────────────────────────────────────────┐
│ ● ● ●  [‹ ›]  Title               [actions] [search] │  Toolbar (52px, glass, draggable)
├────────────┬─────────────────────────────────────────┤
│ Sidebar    │ Content                                 │
│ 220px      │ (List / Grid / Detail / Editor)         │
│ glass      │                                         │
│ Sections   │                                         │
├────────────┴─────────────────────────────────────────┤
│ Status bar (optional, 24px, dim text)                │
└──────────────────────────────────────────────────────┘
```

| Part | Rule |
|---|---|
| Toolbar | Title left after nav buttons; primary actions right; SearchField rightmost. Max 5 actions; rest in a `…` Menu. Icon-only buttons need a Tooltip. |
| Sidebar | Only for navigation (places, pages, libraries). `SourceSection` headers in small caps dim text; `SourceItem` rows 28px with 16px symbolic icon. Selection = accent tint pill. Collapsible (Super+Alt+S). |
| Content | One primary view. Empty → `EmptyState` (icon, title, one-line hint, one action). Loading > 150 ms → `Spinner` centred; never a blank pane. |
| Status bar | Counts and quiet facts only ("12 items, 40 GB available"). No actions. |
| Sheets | Modal questions (save changes, conflicts, authenticate) slide from the toolbar as a Sheet. Never a floating dialog for app-modal questions. |
| Settings-style apps | Sidebar of pages + content of grouped `Section`/`List`/`ListRow` cards; controls right-aligned in rows. |

Single-purpose utilities (Calculator, Clock, Screenshot) use `AppWindow compact`: toolbar only, no sidebar, fixed size.

## 2. Tokens, never literals

- Colours: `--surface --card --text --dim --line --accent --accent-text --control --glass*`. No hex/rgb in app code or app CSS.
- Spacing scale: 4, 8, 12, 16, 24, 32. Nothing else.
- Radius: controls 6, cards 10 (`--glass-radius-s`), windows/panels `--glass-radius`.
- Type: Inter. Title 13 semibold (toolbar), body 13, secondary 11 dim, large title 22 semibold (empty states, About). No other sizes.
- Icons: symbolic, 16 in rows/toolbar, 32/48/64 only for files and app tiles.

## 3. Behaviour

- Shortcuts (Super = ⌘): N new, O open, S save, Shift+S save as, W close window, Q quit, F find, Z/Shift+Z undo/redo, C/X/V/A, comma settings, Backspace delete/trash, I info, Space Quick Look (file lists). Same key, same meaning in every app.
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
Every surface (menu bar, dock, Control Center, Spotlight, lock, login, every Settings page) has golden screenshots at its sizes, passes the layout check, and was visually reviewed by the author before commit.
