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

### Widgets (RU-040 to RU-051)

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
| RU-051 | A link that opens a Settings page names it by its page id (`wol-settings privacy`), never by a title: titles are translated and edited. |

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
| RU-078 | The golden folder holds only PNG files and the button-label manifest (button-labels.tsv). |

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
| RU-110 | Switch | Do | A switch applies its change at once; no Save or Apply button follows it. | manual |
| RU-111 | Switch | Do not | Do not use a switch to start an action such as Install or Restart; use a button. | manual |
| RU-112 | Switch | Do | A change that leaves the process uses the pending switch: it waits, then shows the confirmed state (RC-01). | `RH-011` |
| RU-113 | Switch | Do | A switch in a row is named by the row title and described by its subtitle for assistive technology. | `kit_components` |

#### Component rules a person checks

- RU-110 (Switch): A switch applies its change at once; no Save or Apply button follows it.
- RU-111 (Switch): Do not use a switch to start an action such as Install or Restart; use a button.

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
