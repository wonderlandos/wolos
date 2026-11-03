# Manual IME and screen-reader checklist (CR3x parity gate, plan 6.2 items 1 to 3)

For the independent audit on real hardware (STANDARDS section 8). The automated lanes
(`tests/native_wayland.rs`, `tests/native_text.rs`, `tests/a11y_tree.rs`, `tests/a11y_atspi.rs`)
cover the protocol and the tree; this list covers what only a person with a real input method
and screen reader can judge. Run each of the three sample apps (Todos, Monitor, Table) on the
native backend (`WOLKIT_BACKEND=native` until the default flips) in a WolOS VM session.
Record for every row: date, image version, app, pass or fail, and a sentence on what happened.

## Screen reader

| # | Step | Expected |
|---|---|---|
| S1 | Start the screen reader, then the app. | The app appears in the reader's application list under its window title. |
| S2 | Start the app first, then the screen reader. | Same as S1 within a second (export starts when the reader attaches). |
| S3 | Tab through every control. | Each is announced with its name and role ("Add, button"); the focus ring is visible on each. |
| S4 | Focus a toggle and press Space twice. | "checked" then "not checked" (or pressed / not pressed) is announced each time. |
| S5 | Focus the text field, type "milk", press Left twice. | Typed characters are echoed; the caret moves are announced by character. |
| S6 | Select all in the field (Control+A). | The selection is announced. |
| S7 | Read the whole window (the reader's "read all"). | Headings, labels, list items and the slider value are read in tree order; no label inside a button is read twice. |
| S8 | Use the reader's "activate" on a button without moving keyboard focus. | The button acts (Todos: a row is added). |
| S9 | Move a slider with the arrow keys. | Each new value is announced. |
| S10 | Open a sheet or menu, then press Escape. | Focus is announced inside the sheet; after Escape it returns to the control that opened it. |
| S11 | A secure field: type a password. | Nothing of the text is read back (bullets or "hidden"). |
| S12 | Stop the screen reader. | The app keeps working; no delay is felt on the next keystroke. |

## Input method

| # | Step | Expected |
|---|---|---|
| I1 | A CJK input method on; focus the field; type a reading (Japanese "kanji", Chinese pinyin "nihao"). | The composition is shown underlined in the field, not committed; the app's text (`TEXT_CHANGED`) does not change yet. |
| I2 | Look at the candidate window. | It opens next to the field (below or above it), not at the screen corner. |
| I3 | Pick a candidate. | The chosen characters replace the composition; one `TEXT_CHANGED` with the new text. |
| I4 | Compose, then press Escape. | The composition is dropped; the field's text is unchanged. |
| I5 | Compose, then click another control. | The composition is dropped or committed as the input method decides; no stray text stays underlined. |
| I6 | us(intl) layout: dead acute, then e; dead diaeresis, then u. | "é" and "ü" are typed; the dead key alone types nothing. |
| I7 | The compose key: Compose, o, c. | "©". |
| I8 | Hold a letter key. | It repeats at the desktop's rate after its delay; releasing stops it at once. |
| I9 | Command+A, Command+C in one app, Command+V in the field of another app (and back). | The text arrives in both directions. |
| I10 | Drag a file from the file manager onto the Todos window. | The app receives the drop (the row shows the path); the pointer showed "copy" while over the window. |
| I11 | A field with `max-length` 4: compose 6 characters and commit. | Only 4 arrive. |
