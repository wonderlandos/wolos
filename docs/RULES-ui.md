# UI consistency rules

What: a catalogue of checkable rules (ids `RU-nnn`) for user-facing text, GTK and kit widgets, CSS, golden screenshots and documentation. Why: the audits kept finding the same drift (three spellings of Wi-Fi, title and sentence case mixed, windows without titles, dead CSS classes), and a rule only holds if a test enforces it. How: `crates/shell/tests/rules_ui.rs` scans `crates/shell/src`, `kit.css`, the golden declarations and PNG folder, and `docs/`; each rule is one `check("RU-nnn", ...)` call. Related: `docs/STANDARDS.md` section 5 and `docs/modules/hig.md`; copy rules extend `crates/shell/src/copy.rs` (`copy::lint`) and `tests/copy_scan.rs`.

## How the test works

- Each rule counts violations. Where the tree does not comply yet, the count is frozen in `BASELINE` in the test file (growth-only): fix violations and lower the number, never raise it. A rule with no baseline entry has zero violations today and must stay that way.
- `RULES_PRINT=1 cargo test -p shell --test rules_ui -- --nocapture` prints the current count per rule and fails nothing; use it to lower baselines after a cleanup.
- The tables of RU-001 to RU-099 below are the same text as `RULES` in the test (`rules_doc_matches_table` compares them). Change both together; a rule id is never reused.
- RU-100 to RU-999 belong to the component specs (`design/components/*.toml`, ADR 0058). Their table at the end of the rules is generated (`python3 -I design/tools/components.py --rules-md --write`), CI checks it is current, and `rules_ui` compares it with the catalog: ids in range and unique, each rule enforced by `manual`, a ratchet id or a test file that exists.
- Sentences are string literals that start with a capital, contain a space and end in a full stop, question mark, exclamation mark or ellipsis. Log, assert and panic lines are skipped. Copy files are `copy.rs` and `copy_installer.rs`.

## Rules

### Copy and glossary (RU-001 to RU-028)

| Id | Rule |
|---|---|
| RU-001 | A sentence in copy.rs or copy_installer.rs is at most 140 characters. |
| RU-002 | A sentence in the copy files has at most two sentences. |
| RU-003 | No on-screen text says "failed". |
| RU-004 | No on-screen text shows an errno, exit code, OS error or panic text. |
| RU-005 | Copy-file sentences hold no path, flag, code token or file name. |
| RU-006 | Copy-file sentences name no program or plumbing (wol-, nmcli, daemon, socket); no on-screen text names an upstream project. |
| RU-007 | No exclamation marks in on-screen text. |
| RU-008 | No double spaces in on-screen text. |
| RU-009 | Quotes in copy are typographic: no escaped ASCII double quote in a sentence. |
| RU-010 | Apostrophes in sentences are typographic (’), not ASCII ('). |
| RU-011 | An ellipsis is the single character …, never three dots. |
| RU-012 | A sentence has no leading or trailing space and no line break. |
| RU-013 | Body text is sentence case, not Title Case. |
| RU-014 | Titles (window, page, section) are Title Case. |
| RU-015 | A title has no trailing full stop. |
| RU-016 | A title is at most 40 characters. |
| RU-017 | No courtesy padding: please, sorry, oops, whoops, successfully, click here. |
| RU-018 | Glossary: say Desktop, never workspace. |
| RU-019 | Glossary: Wi-Fi, never `WiFi`, `Wifi` or `wifi`. |
| RU-020 | Glossary: Bluetooth, capital B. |
| RU-021 | Glossary: startup, never start-up. |
| RU-022 | Glossary: email, never e-mail. |
| RU-023 | Glossary: the key is Space, never spacebar or space bar. |
| RU-024 | Glossary: Settings, never Preferences or Control Panel. |
| RU-025 | Glossary: Trash, never Recycle Bin. |
| RU-026 | Glossary: app, never application. |
| RU-027 | Glossary: WolOS is spelled WolOS. |
| RU-028 | Glossary: USB is upper case. |

### Buttons (RU-030 to RU-035)

| Id | Rule |
|---|---|
| RU-030 | A button label starts with an allowlisted verb or is an allowlisted noun. |
| RU-031 | A button label is Title Case. |
| RU-032 | A button label is at most four words. |
| RU-033 | A button label has no full stop or exclamation mark and no three-dot ellipsis. |
| RU-034 | No OK, Yes or No buttons: say the action. |
| RU-035 | A file with a destructive button also has a way out (Cancel, Not Now, Close, Done, Back). |

### Widgets (RU-040 to RU-050)

| Id | Rule |
|---|---|
| RU-040 | Every add_css_class literal is styled in kit.css or contrast.css, or is a named hook. |
| RU-041 | CSS class names are lower-case kebab-case. |
| RU-042 | Every symbolic icon name the shell requests ships in design/icons/symbolic. |
| RU-043 | Every file in design/icons/symbolic is named name-symbolic.svg in lower-case. |
| RU-044 | Every wolos- app icon the shell requests ships in design/icons/apps. |
| RU-045 | Every window the shell creates sets a title. |
| RU-046 | Every AlertDialog has a cancel button. |
| RU-047 | Every entry has a placeholder, label or accessible name. |
| RU-048 | No list row is made unfocusable. |
| RU-049 | No fixed width above 600 px in code (set_size_request, set_width_request). |
| RU-050 | An icon-only button has a tooltip. |

### CSS (RU-055 to RU-064)

| Id | Rule |
|---|---|
| RU-055 | No fixed width above 600 px in kit.css. |
| RU-056 | No !important in the stylesheets. |
| RU-057 | Braces balance in kit.css and contrast.css. |
| RU-058 | Stylesheets have no tabs, no trailing whitespace and end with a newline. |
| RU-059 | No font-size below 11px. |
| RU-060 | No transition or animation longer than 500 ms. |
| RU-061 | Every var(--x) in kit.css is defined somewhere (stylesheet or theme.rs). |
| RU-062 | No hex colour literal outside a variable definition in kit.css. |
| RU-063 | No empty rule block in kit.css. |
| RU-064 | kit.css and contrast.css start with a comment saying what they are. |

### Goldens (RU-070 to RU-078)

| Id | Rule |
|---|---|
| RU-070 | Every golden has a light and a dark PNG. |
| RU-071 | Golden names are lower-case kebab-case. |
| RU-072 | Installer and setup goldens end in a size; settings goldens end in wide or narrow. |
| RU-073 | A golden PNG is at most 400 KiB. |
| RU-074 | Every Settings pane has wide and narrow goldens, each light and dark. |
| RU-075 | Every surface file in src/ui has at least one golden. |
| RU-076 | Golden case names are unique in the declarations. |
| RU-077 | Every golden case declared with a literal name has its light and dark PNG. |
| RU-078 | The golden folder holds only PNG files. |

### Docs (RU-080 to RU-099)

| Id | Rule |
|---|---|
| RU-080 | Every docs/modules/*.md starts with a single # title. |
| RU-081 | A module document names its ADR in its first twelve lines. |
| RU-082 | A module document names its code by crate path. |
| RU-083 | A module document opens with a prose line saying what the thing is. |
| RU-084 | docs/README.md lists every module document. |
| RU-085 | Every ADR has a Status line in its first three lines. |
| RU-086 | ADR files are named NNNN-slug.md, lower-case. |
| RU-087 | An ADR title starts with its own number. |
| RU-088 | An ADR has Context, Decision and Consequences. |
| RU-089 | ADR numbers are unique and contiguous. |
| RU-090 | A superseded ADR names the existing ADR that replaces it. |
| RU-091 | No broken relative link in docs. |
| RU-092 | Every docs/...md path named in backticks exists. |
| RU-093 | Every wol verb in catalog.rs is documented as `wol <verb>` in docs. |
| RU-094 | Every Settings pane has a doc section (a heading or its source path). |
| RU-095 | No TODO, FIXME or XXX markers in docs. |
| RU-096 | Docs end with a newline and have no tabs. |
| RU-097 | docs/modules file names are lower-case kebab-case. |
| RU-098 | Docs prose spells Wi-Fi and Bluetooth as the glossary does. |
| RU-099 | Docs use no emoji. |

<!-- component-rules:begin (generated by design/tools/components.py --rules-md; do not edit) -->

### Component rules (RU-100 to RU-999, generated)

Each component spec in `design/components/` lists its rules; the Kit tests and `rules_ui` hold this table to the catalog. Enforced by: a test file, a ratchet id (`RH-nnn`, docs/RULES.md) or `manual`.

| Id | Component | Kind | Rule | Enforced by |
|---|---|---|---|---|
| RU-100 | Discrete slider | Do | A discrete slider has three to eleven stops; more is a continuous slider. | manual |
| RU-101 | Discrete slider | Do | The value of a discrete slider always sits on a stop; a drag between stops snaps. | `kit_choice` |
| RU-102 | Range slider | Do | Each knob of a range slider is a slider of its own with its own name. | `kit_choice` |
| RU-103 | Range slider | Do | The low knob never passes the high knob. | `kit_choice` |
| RU-104 | Floating action button | Do | A surface has at most one floating action button, and only when its toolbar does not hold that action. | manual |
| RU-105 | Floating action button | Do | A floating action button with a glyph alone has a tooltip, which is its name. | `kit_collection` |
| RU-110 | Switch | Do | A switch applies its change at once; no Save or Apply button follows it. | manual |
| RU-111 | Switch | Do not | Do not use a switch to start an action such as Install or Restart; use a button. | manual |
| RU-112 | Switch | Do | A change that leaves the process uses the pending switch: it waits, then shows the confirmed state (RC-01). | `RH-011` |
| RU-113 | Switch | Do | A switch in a row is named by the row title and described by its subtitle for assistive technology. | `kit_components` |
| RU-120 | Button | Do | A surface has one primary button; the other actions are default or plain. | manual |
| RU-121 | Button | Do not | Do not use a destructive button for an action that loses nothing. | manual |
| RU-122 | Button | Do | A button whose action runs waits and shows a spinner beside it; it never stays pressable. | `kit_actions` |
| RU-123 | Button | Do | A button is named by its own label, and a button without words is an icon button. | `kit_actions` |
| RU-124 | Button | Do | An outlined button stands beside a primary button as the second choice; it is never the only button of a surface. | manual |
| RU-130 | Icon button | Do | An icon button has a tooltip, and the tooltip is its accessible name. | `kit_actions` |
| RU-131 | Icon button | Do not | Do not use an icon button for an action the glyph cannot say; use a button with words. | manual |
| RU-132 | Icon button | Do | A toolbar holds at most five actions; the rest go in a menu button. | manual |
| RU-133 | Split button | Do | The menu under a split button holds variations of the main action, never unrelated commands. | manual |
| RU-134 | Split button | Do | The arrow of a split button is named for assistive technology. | `kit_groups` |
| RU-135 | Button group | Do | A button group has two to five buttons; for exactly one choice from several use the segmented control. | manual |
| RU-136 | Button group | Do | A button group is named for assistive technology, and every glyph button in it has a tooltip. | `kit_groups` |
| RU-137 | Floating toolbar | Do | A floating toolbar holds two to five actions, every one with a tooltip, and at most one primary. | `kit_groups` |
| RU-138 | Floating toolbar | Do not | Do not float a toolbar over text a person is reading or typing; keep it for images, maps and canvases. | manual |
| RU-140 | Segmented control | Do | A segmented control holds two to four options and applies the pick at once. | `kit_actions` |
| RU-141 | Segmented control | Do not | Do not use a segmented control for more than four options or for options that start actions; use a dropdown or buttons. | manual |
| RU-142 | Segmented control | Do | A segmented control in a row is named by the row title for assistive technology. | `kit_actions` |
| RU-150 | Slider | Do | A slider reports its value once, after the knob settles, not on every pixel. | `kit_actions` |
| RU-151 | Slider | Do | A slider row names its ends in words and its title names the setting. | manual |
| RU-152 | Slider | Do not | Do not use a slider for a choice among a few named options; use a segmented control. | manual |
| RU-160 | Stepper | Do | A button that would leave the range waits instead of doing nothing. | `kit_actions` |
| RU-161 | Stepper | Do not | Do not use a stepper for a range of more than about 30 steps; use a slider. | manual |
| RU-162 | Stepper | Do | A stepper exposes its value, minimum and maximum to assistive technology. | manual |
| RU-170 | Dropdown | Do | A dropdown reports its pick once and applies it at once. | `kit_actions` |
| RU-171 | Dropdown | Do | A list of more than about twelve options has a search field. | manual |
| RU-172 | Dropdown | Do | A change that leaves the process uses the pending dropdown, which waits and puts the confirmed choice back on failure (RC-01). | manual |
| RU-173 | Dropdown | Do not | Do not use a dropdown for two or three options that fit on screen; use a segmented control. | manual |
| RU-180 | Menu button | Do | A menu button with no actions is not built. | `kit_actions` |
| RU-181 | Menu button | Do | A menu button has a tooltip and opens with Space and Enter. | `kit_actions` |
| RU-182 | Menu button | Do not | Do not hide the one action people need most behind a menu button. | manual |
| RU-190 | Checkbox | Do | Use a checkbox for an option among several that can be on together; for one setting that applies at once use a switch. | manual |
| RU-191 | Checkbox | Do | A checkbox carries its label and the label toggles it too. | `kit_choice` |
| RU-192 | Checkbox | Do not | Do not phrase a checkbox label as a negative (Do not show); say what is true when it is on. | manual |
| RU-193 | Radio group | Do | Use a radio group for two to six exclusive options a person compares; use a dropdown for more. | manual |
| RU-194 | Radio group | Do | A radio group always has one option picked, and the group is named by its title. | `kit_choice` |
| RU-195 | Radio group | Do not | Do not use a radio group to start actions; each option is a state. | manual |
| RU-196 | Chip | Do | Chips appear in a set, 8 px apart, about the content above or below them; never as the main action of a surface. | manual |
| RU-197 | Chip | Do | A filter chip shows a check when it is on, so the state is not colour alone. | `kit_choice` |
| RU-198 | Chip | Do | An input chip's remove button is named Remove followed by what it removes. | `kit_choice` |
| RU-199 | Chip | Do not | Do not put more than eight chips in a set; use a dropdown or a search field. | manual |
| RU-210 | Text field | Do | A text field commits once on Enter or when it loses the keyboard with a changed text, never on every key. | `kit_input` |
| RU-211 | Text field | Do | A commit that can fail or be cancelled uses the pending field, which waits and lets Enter commit the same text again (QA-SET-12). | `kit_input` |
| RU-212 | Text field | Do | A text field in a row is named by the row title for assistive technology. | `kit_input` |
| RU-213 | Text field | Do not | Do not use a placeholder as the only label of a field. | manual |
| RU-214 | Date input | Do | A date field says the shape it wants and marks a day that does not exist as invalid for assistive technology, not by colour alone. | `kit_groups` |
| RU-215 | Date input | Do | A date field always comes with the calendar button, so a far date is a click away. | `kit_groups` |
| RU-216 | Date range picker | Do | Every day button is named by its full date for assistive technology, not by its number alone. | `kit_calendar` |
| RU-217 | Date range picker | Do | A range picker starts a new range at the third click and never keeps a half range once the second end is picked. | `kit_calendar` |
| RU-220 | Password field | Do | A password field never shows what was typed unless the person asks (peek). | `kit_input` |
| RU-221 | Password field | Do | The Caps Lock note says so in words under the field, not only with an icon. | `kit_input` |
| RU-222 | Password field | Do not | Do not put a secret in a text field, a log, a tooltip or a golden. | manual |
| RU-230 | Search field | Do | A search field is the rightmost item of the toolbar. | manual |
| RU-231 | Search field | Do | A search field shows results as the person types and keeps the typed text when they leave. | manual |
| RU-232 | Search field | Do | A search field has a name for assistive technology. | `kit_input` |
| RU-240 | Shortcut recorder | Do | A recorder pauses the other shortcuts while it listens and resumes them when it stops. | manual |
| RU-241 | Shortcut recorder | Do | A recorder that loses the keyboard stops without a change. | `kit_input` |
| RU-242 | Shortcut recorder | Do | A combination that another command has is marked with the warning outline and offers to swap. | manual |
| RU-250 | Colour well | Do | A colour is also shown as text (the tooltip), never by the disc alone. | `kit_input` |
| RU-251 | Colour well | Do | The palette reports a valid colour only; a mistyped hex changes nothing. | manual |
| RU-260 | Text area | Do | A text area is at least three lines (84 px) tall and grows to the content up to its page. | `kit_calendar` |
| RU-261 | Text area | Do not | Do not use a text area for one line; use a text field. | manual |
| RU-270 | Date picker | Do | Every day button is named by its full date for assistive technology, not by its number alone. | `kit_calendar` |
| RU-271 | Date picker | Do | Weeks start on Monday, like the calendar widget. | `kit_calendar` |
| RU-272 | Date picker | Do not | Do not make a person scroll month by month to a far date; offer a text field for typing it as well. | manual |
| RU-280 | Time picker | Do | Times show two digits on each side of the colon (09:05). | `kit_calendar` |
| RU-281 | Time picker | Do | The hour and the minute are named for assistive technology. | `kit_calendar` |
| RU-290 | Search view | Do | When nothing matches, the view says so in a sentence fragment that names what was searched. | `kit_sheets` |
| RU-291 | Search view | Do not | Do not open a search view on its own; it opens from the search field a person typed in. | manual |
| RU-300 | Tabs | Do | Tabs hold two to six views of one subject; more than six scroll and say so with the cut-off tab. | manual |
| RU-301 | Tabs | Do | The picked tab is shown by the indicator line and the label colour, never colour alone. | `kit_navigation` |
| RU-302 | Tabs | Do | Each tab panel is labelled by its tab and each tab controls its panel for assistive technology. | `kit_navigation` |
| RU-303 | Tabs | Do not | Do not nest primary tabs; inside a tab use secondary tabs or a segmented control. | manual |
| RU-310 | Navigation bar | Do | A navigation bar shows only when the window is narrower than 640 px; wider windows use the sidebar. | manual |
| RU-311 | Navigation bar | Do | Every destination has a glyph and a label; never a glyph alone. | `kit_navigation` |
| RU-315 | Navigation rail | Do | A rail replaces the sidebar between the narrow breakpoint and the sidebar's width; never both at once. | manual |
| RU-316 | Navigation rail | Do | Every rail destination has a glyph and a label. | `kit_navigation` |
| RU-317 | Navigation drawer | Do | The drawer opens over the content with a scrim and closes when a destination is picked or Escape is pressed; the wide window shows the sidebar instead, never both. | manual |
| RU-318 | Navigation drawer | Do | Every drawer destination has a glyph and a label, and the drawer is named by its title. | `kit_groups` |
| RU-335 | Bottom app bar | Do | A bottom app bar holds at most four actions and at most one primary action, at the end. | `kit_navigation` |
| RU-336 | Bottom app bar | Do not | Do not show a bottom app bar and a toolbar with the same actions at once. | manual |
| RU-340 | Breadcrumb | Do | The last crumb is the current page and is not a link. | `kit_navigation` |
| RU-341 | Breadcrumb | Do not | Do not use a breadcrumb for a flat set of pages; use the sidebar or tabs. | manual |
| RU-345 | Status bar | Do not | Do not put buttons or links in a status bar. | `kit_navigation` |
| RU-346 | Status bar | Do | A status bar is a status for assistive technology, so its changes are announced politely. | `kit_navigation` |
| RU-350 | Menu | Do | A menu item says what it does in Title Case and shows its shortcut at the end in the dim ink. | `kit_navigation` |
| RU-351 | Menu | Do | A context menu offers the same verbs as the toolbar, with the same names and keys. | manual |
| RU-352 | Menu | Do not | Do not put more than twelve items in a menu without separators between groups. | manual |
| RU-360 | Disclosure | Do | A disclosure exposes its expanded state, so a screen reader says open or closed. | `kit_navigation` |
| RU-361 | Disclosure | Do not | Do not hide something a person needs on most visits behind a disclosure. | manual |
| RU-400 | Card | Do | A card insets its content equally on all four sides by the density's inset. | `kit_collection` |
| RU-401 | Card | Do | Use one card style per surface: elevated on the window ground, filled inside another card, outlined in dense lists. | manual |
| RU-402 | Card | Do not | Do not nest an elevated card in an elevated card. | manual |
| RU-410 | List item | Do | A trailing control in a list item is named by the item's title. | `kit_collection` |
| RU-411 | List item | Do | A three-line item ellipsizes its supporting text after two lines instead of growing. | `kit_collection` |
| RU-412 | List item | Do not | Do not mix one-, two- and three-line items in one list without a reason a person can see. | manual |
| RU-420 | Divider | Do | A divider inside a card is inset by the card's inset, never drawn to the card edge. | `kit_collection` |
| RU-430 | Sheet | Do | A sheet's buttons are verbs that say what happens, the default one last. | `kit_sheets` |
| RU-431 | Sheet | Do | A sheet is a dialog for assistive technology, named by its title and described by its text. | `kit_sheets` |
| RU-432 | Sheet | Do not | Do not stack a sheet on a sheet; finish or cancel the first. | manual |
| RU-440 | Bottom sheet | Do | A bottom sheet is for narrow windows and touch; wide windows use a sheet from the toolbar or a side sheet. | manual |
| RU-441 | Bottom sheet | Do | A bottom sheet closes with Escape and shows its grab handle. | `kit_sheets` |
| RU-445 | Side sheet | Do | A side sheet sits beside the content and narrows it; it never covers it. | manual |
| RU-446 | Side sheet | Do | A side sheet has a close button named Close and is labelled by its title. | `kit_sheets` |
| RU-450 | Split view | Do | Under the narrow breakpoint a split view shows one pane at a time with a way back, like the AppWindow sidebar. | manual |
| RU-460 | Section header | Do | A section header is a heading for assistive technology. | `kit_collection` |
| RU-461 | Section header | Do not | Do not put more than one action in a section header. | manual |
| RU-470 | Carousel | Do | A carousel can be moved with its buttons and the keyboard, never only by dragging. | `kit_collection` |
| RU-471 | Carousel | Do not | Do not advance a carousel by itself. | manual |
| RU-500 | Toast | Do | A toast replaces the previous toast and leaves after six seconds, longer when Give me more time is set. | `kit_comms` |
| RU-501 | Toast | Do not | Do not put a decision or an error that needs reading in a toast; use a notice or a sheet. | manual |
| RU-502 | Toast | Do | A toast has at most one action, and using it hides the toast. | `kit_comms` |
| RU-510 | Notice | Do | A notice sits next to what it is about, not at the top of the window. | manual |
| RU-511 | Notice | Do | A notice is one sentence and its buttons are verbs. | manual |
| RU-512 | Notice | Do not | Do not use a notice for something that only needs a toast. | manual |
| RU-520 | Badge | Do | A badge is one word or a count, twelve characters at most. | `kit_comms` |
| RU-521 | Badge | Do not | Do not carry meaning by the tone alone; the word says it too. | manual |
| RU-522 | Badge | Do not | Do not make a badge a button. | manual |
| RU-530 | Skeleton | Do | A skeleton is busy and named for assistive technology by what is loading. | `kit_collection` |
| RU-531 | Skeleton | Do | A skeleton shows only after 150 ms of waiting and has the shape of what will come. | manual |
| RU-535 | On-screen display | Do | The on-screen display never takes the keyboard from the app in front. | manual |
| RU-536 | On-screen display | Do | The level is a bar and a glyph, never colour alone. | `kit_sheets` |
| RU-540 | Spinner | Do | A spinner shows only while work runs and is hidden and still otherwise. | `kit_comms` |
| RU-541 | Spinner | Do not | Do not show a spinner for work whose progress is known; use a progress bar. | manual |
| RU-542 | Spinner | Do | Work longer than a second shows a spinner, and longer than ten seconds says what runs. | manual |
| RU-550 | Progress bar | Do | A progress bar that knows its fraction shows it; one that does not is indeterminate. | `kit_comms` |
| RU-551 | Progress bar | Do not | Do not move a bar backwards. | manual |
| RU-552 | Progress bar | Do | A progress bar has a name and exposes its value to assistive technology. | `kit_comms` |
| RU-560 | Progress ring | Do | A ring with a known value shows it from the top, clockwise. | manual |
| RU-561 | Progress ring | Do | A ring exposes its value and a name to assistive technology. | manual |
| RU-570 | Empty state | Do | An empty page says what to do next, in one line and at most one action. | `kit_comms` |
| RU-571 | Empty state | Do not | Do not leave a page blank when there is nothing to show. | manual |
| RU-572 | Avatar | Do | An avatar is square, so it draws a circle. | `kit_comms` |
| RU-573 | Avatar | Do | The same name always gets the same tone. | `kit_comms` |
| RU-574 | Avatar | Do | An avatar is named by the person's full name for assistive technology, not by its initials. | manual |
| RU-575 | Tile | Do | A tile shows one action at a time: the button or the progress, never both. | `kit_comms` |
| RU-576 | Tile | Do | An app that does not come from WolOS's packages says where from. | `kit_comms` |
| RU-580 | Error line | Do | An error line says what to do in one sentence and is drawn in the danger colour. | `kit_comms` |
| RU-581 | Error line | Do not | Do not show an error line before the person has done something. | manual |
| RU-590 | Tooltip | Do | A tooltip is a name of at most three words in Title Case with no full stop. | `kit_comms` |
| RU-591 | Tooltip | Do not | Do not hide information in a tooltip that a person needs to act; tooltips name, they do not explain. | manual |
| RU-592 | Rich tooltip | Do | A rich tooltip holds at most one action and never the only way to do something. | manual |
| RU-593 | Rich tooltip | Do not | Do not put a rich tooltip on a control that has a plain tooltip; one or the other. | manual |
| RU-600 | Meter | Do | A meter is clamped to the whole: a share above 100 % draws full. | `kit_comms` |
| RU-601 | Meter | Do not | Do not carry meaning by the tone alone; the value is in words beside it. | manual |
| RU-602 | Gauge | Do | A gauge shows its value in words as well as in the ring. | `kit_comms` |
| RU-603 | Gauge | Do | A gauge exposes its caption and value to assistive technology. | manual |
| RU-610 | Table | Do | Number columns are right-aligned in tabular figures. | `kit_collection` |
| RU-611 | Table | Do | A table row is 32 px at standard density and follows the density. | `kit_collection` |
| RU-612 | Table | Do not | Do not put more than one control in a table cell. | manual |
| RU-620 | Tree | Do | Right and Left open and close the focused row of a tree. | manual |
| RU-621 | Tree | Do | A tree row is 28 px at standard density and follows the density. | `kit_collection` |

#### Component rules a person checks

- RU-100 (Discrete slider): A discrete slider has three to eleven stops; more is a continuous slider.
- RU-104 (Floating action button): A surface has at most one floating action button, and only when its toolbar does not hold that action.
- RU-110 (Switch): A switch applies its change at once; no Save or Apply button follows it.
- RU-111 (Switch): Do not use a switch to start an action such as Install or Restart; use a button.
- RU-120 (Button): A surface has one primary button; the other actions are default or plain.
- RU-121 (Button): Do not use a destructive button for an action that loses nothing.
- RU-124 (Button): An outlined button stands beside a primary button as the second choice; it is never the only button of a surface.
- RU-131 (Icon button): Do not use an icon button for an action the glyph cannot say; use a button with words.
- RU-132 (Icon button): A toolbar holds at most five actions; the rest go in a menu button.
- RU-133 (Split button): The menu under a split button holds variations of the main action, never unrelated commands.
- RU-135 (Button group): A button group has two to five buttons; for exactly one choice from several use the segmented control.
- RU-138 (Floating toolbar): Do not float a toolbar over text a person is reading or typing; keep it for images, maps and canvases.
- RU-141 (Segmented control): Do not use a segmented control for more than four options or for options that start actions; use a dropdown or buttons.
- RU-151 (Slider): A slider row names its ends in words and its title names the setting.
- RU-152 (Slider): Do not use a slider for a choice among a few named options; use a segmented control.
- RU-161 (Stepper): Do not use a stepper for a range of more than about 30 steps; use a slider.
- RU-162 (Stepper): A stepper exposes its value, minimum and maximum to assistive technology.
- RU-171 (Dropdown): A list of more than about twelve options has a search field.
- RU-172 (Dropdown): A change that leaves the process uses the pending dropdown, which waits and puts the confirmed choice back on failure (RC-01).
- RU-173 (Dropdown): Do not use a dropdown for two or three options that fit on screen; use a segmented control.
- RU-182 (Menu button): Do not hide the one action people need most behind a menu button.
- RU-190 (Checkbox): Use a checkbox for an option among several that can be on together; for one setting that applies at once use a switch.
- RU-192 (Checkbox): Do not phrase a checkbox label as a negative (Do not show); say what is true when it is on.
- RU-193 (Radio group): Use a radio group for two to six exclusive options a person compares; use a dropdown for more.
- RU-195 (Radio group): Do not use a radio group to start actions; each option is a state.
- RU-196 (Chip): Chips appear in a set, 8 px apart, about the content above or below them; never as the main action of a surface.
- RU-199 (Chip): Do not put more than eight chips in a set; use a dropdown or a search field.
- RU-213 (Text field): Do not use a placeholder as the only label of a field.
- RU-222 (Password field): Do not put a secret in a text field, a log, a tooltip or a golden.
- RU-230 (Search field): A search field is the rightmost item of the toolbar.
- RU-231 (Search field): A search field shows results as the person types and keeps the typed text when they leave.
- RU-240 (Shortcut recorder): A recorder pauses the other shortcuts while it listens and resumes them when it stops.
- RU-242 (Shortcut recorder): A combination that another command has is marked with the warning outline and offers to swap.
- RU-251 (Colour well): The palette reports a valid colour only; a mistyped hex changes nothing.
- RU-261 (Text area): Do not use a text area for one line; use a text field.
- RU-272 (Date picker): Do not make a person scroll month by month to a far date; offer a text field for typing it as well.
- RU-291 (Search view): Do not open a search view on its own; it opens from the search field a person typed in.
- RU-300 (Tabs): Tabs hold two to six views of one subject; more than six scroll and say so with the cut-off tab.
- RU-303 (Tabs): Do not nest primary tabs; inside a tab use secondary tabs or a segmented control.
- RU-310 (Navigation bar): A navigation bar shows only when the window is narrower than 640 px; wider windows use the sidebar.
- RU-315 (Navigation rail): A rail replaces the sidebar between the narrow breakpoint and the sidebar's width; never both at once.
- RU-317 (Navigation drawer): The drawer opens over the content with a scrim and closes when a destination is picked or Escape is pressed; the wide window shows the sidebar instead, never both.
- RU-336 (Bottom app bar): Do not show a bottom app bar and a toolbar with the same actions at once.
- RU-341 (Breadcrumb): Do not use a breadcrumb for a flat set of pages; use the sidebar or tabs.
- RU-351 (Menu): A context menu offers the same verbs as the toolbar, with the same names and keys.
- RU-352 (Menu): Do not put more than twelve items in a menu without separators between groups.
- RU-361 (Disclosure): Do not hide something a person needs on most visits behind a disclosure.
- RU-401 (Card): Use one card style per surface: elevated on the window ground, filled inside another card, outlined in dense lists.
- RU-402 (Card): Do not nest an elevated card in an elevated card.
- RU-412 (List item): Do not mix one-, two- and three-line items in one list without a reason a person can see.
- RU-432 (Sheet): Do not stack a sheet on a sheet; finish or cancel the first.
- RU-440 (Bottom sheet): A bottom sheet is for narrow windows and touch; wide windows use a sheet from the toolbar or a side sheet.
- RU-445 (Side sheet): A side sheet sits beside the content and narrows it; it never covers it.
- RU-450 (Split view): Under the narrow breakpoint a split view shows one pane at a time with a way back, like the AppWindow sidebar.
- RU-461 (Section header): Do not put more than one action in a section header.
- RU-471 (Carousel): Do not advance a carousel by itself.
- RU-501 (Toast): Do not put a decision or an error that needs reading in a toast; use a notice or a sheet.
- RU-510 (Notice): A notice sits next to what it is about, not at the top of the window.
- RU-511 (Notice): A notice is one sentence and its buttons are verbs.
- RU-512 (Notice): Do not use a notice for something that only needs a toast.
- RU-521 (Badge): Do not carry meaning by the tone alone; the word says it too.
- RU-522 (Badge): Do not make a badge a button.
- RU-531 (Skeleton): A skeleton shows only after 150 ms of waiting and has the shape of what will come.
- RU-535 (On-screen display): The on-screen display never takes the keyboard from the app in front.
- RU-541 (Spinner): Do not show a spinner for work whose progress is known; use a progress bar.
- RU-542 (Spinner): Work longer than a second shows a spinner, and longer than ten seconds says what runs.
- RU-551 (Progress bar): Do not move a bar backwards.
- RU-560 (Progress ring): A ring with a known value shows it from the top, clockwise.
- RU-561 (Progress ring): A ring exposes its value and a name to assistive technology.
- RU-571 (Empty state): Do not leave a page blank when there is nothing to show.
- RU-574 (Avatar): An avatar is named by the person's full name for assistive technology, not by its initials.
- RU-581 (Error line): Do not show an error line before the person has done something.
- RU-591 (Tooltip): Do not hide information in a tooltip that a person needs to act; tooltips name, they do not explain.
- RU-592 (Rich tooltip): A rich tooltip holds at most one action and never the only way to do something.
- RU-593 (Rich tooltip): Do not put a rich tooltip on a control that has a plain tooltip; one or the other.
- RU-601 (Meter): Do not carry meaning by the tone alone; the value is in words beside it.
- RU-603 (Gauge): A gauge exposes its caption and value to assistive technology.
- RU-612 (Table): Do not put more than one control in a table cell.
- RU-620 (Tree): Right and Left open and close the focused row of a tree.

<!-- component-rules:end -->

## Not automated

- Whether a sentence says what happened and what to do (needs a reader); only its shape is checked.
- Whether a button's verb matches what it does, and whether Title Case proper nouns (product names) are right.
- Whether a Settings pane's doc section is accurate; RU-094 only checks it exists.
- Pixel width of window default sizes (only fixed minimum widths are checked).
- Contrast, focus order and screen-reader text beyond the entry and icon-button label checks.
- Whether a golden still shows the intended state; goldens only catch change (`tests/golden.rs`).
- The component rules marked `manual` in the generated table; they are listed under it so the set that needs a person is visible and shrinks.

## Baselines

See `BASELINE` in `crates/shell/tests/rules_ui.rs` for the frozen counts.
