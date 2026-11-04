# Default Apps and Open With (ADR 0059)

What a person chooses, where it is written, and what the Dock does with a dropped file. Surfaces S05, S06 and
S24 of `.claude/v012/apps-e2e/EXPERIENCE-MAP.md`. Decision record: ADR 0059 (the experience; draft in
`.claude/v012/apps-e2e/ADR-0059-experience.draft.md`, promoted by E2E-CONTRACT). Code: `crates/shell/src/open_with.rs`.

## Settings > General > Default Apps

Nine rows: Web browser, Email, Files, Images, Music, Video, PDF documents, Text, Calendar
(`open_with::CATEGORIES`). Each row lists the installed apps whose `.desktop` says they open the row's first type
(`MimeType=`), by name, with the current one selected. A pick writes every type of the row that the app opens
(`Category::mimes_for`: all of the row's types the app declares, and always the first) into
`~/.config/mimeapps.list`, as one journaled change (`what` = "default apps", shown in Restore and undone by Undo).

- Order of reading, as the specification: the person's `~/.config`, `$XDG_CONFIG_DIRS`, then `applications/` of the data
  directories, in each first `<desktop>-mimeapps.list` for every name in `XDG_CURRENT_DESKTOP`, then `mimeapps.list`
  (`open_with::mimeapps_paths`). `effective_default` picks the first installed Default Applications entry that no file
  at the same or a higher place removes, then the first such Added Association; a repeated key counts as its last
  line, as GKeyFile reads it. The page writes only the plain file in `~/.config`; a `<desktop>-mimeapps.list` there
  would still outrank it.
- The file is edited line by line (`MimeApps`): only the lines of the types being set change. Comments, blank lines,
  other groups and unknown lines stay as they were, and `render(parse(text)) == text`. A byte order mark is kept and
  is not part of the first header; a file with CR LF line ends gets CR LF in the lines a change writes. A pick puts the app first on the
  `[Default Applications]` line, keeps up to three earlier choices behind it as fall-backs, adds it first to
  `[Added Associations]` (Open With lists it first) and lifts it out of `[Removed Associations]`.
- A row whose current app is not among the choices starts on "Not chosen"; a row no app serves has one greyed entry.
- The shipped defaults (`desktop/overlay/usr/share/applications/mimeapps.list`) are not touched: they belong to the
  image (E2E-SET). The person's file overrides them.

## Open With remembers

`open_with::remember(path, app_id)` records the app picked in the Open With chooser as the first
`[Added Associations]` entry of the file's type (a write of its own through a uniquely named temporary file and a
rename). The default app is not changed. The chooser in `app_session.rs` calls it after a successful launch; that call
is made on E2E-SHARE's branch (it owns the file). For the chooser to list the remembered app first there are the helpers
`open_with::added_for` (the person's Added Associations for the file) and `open_with::promote` (a stable move to the
front), both tested in `open_with::tests`.

## Dock tile: drop a file, jump list

- Each app tile is a drop target for files. Dropping starts the tile's app with the files, filling the `Exec`
  field codes (`open_with::expand_exec`): `%F` and `%U` take all files in one start; `%f` and `%u` start the program once
  per file; with no file the code vanishes; `%i` is `--icon <Icon>`, `%c` the name, `%k` the file, `%%` a percent sign;
  the removed codes (`%d %n ...`) are nothing. A file with no local path is passed to `%u`/`%U` only; a drop with nothing usable for the app's codes (a remote file
  onto a `%f`/`%F` app, or no file) is refused and the icon does not bounce (`Handler::usable`). An app whose
  `Exec` has none of `%f %F %u %U` refuses the drop (`ui::dock::accepts`), and so does a tile with no desktop entry (the
  Apps launcher). The icon bounces like after a click.
- Right-click on a tile lists the `[Desktop Action]` names in the `Actions=` order (New Window, New Private Window),
  each starting that action (`Handler::jump_list`). An app with none has no menu.
- The tile finds its entry by the pin's app id, then by its program name (`ids_for_pin`), off the GTK thread.

## Tests

- `open_with::tests`: field codes, quoting, jump list, the file editor, fifty generated files that must round-trip and
  keep their comments and other groups (`fifty_generated_files_round_trip_and_keep_their_comments`).
- `ui::settings::default_apps::tests`: the menus (current app, "Not chosen", empty row).
- `tests/golden_cases/settings.rs` `default_apps_write_the_choice_and_keep_the_file`: the real page writes the file and a
  journal entry. `tests/golden_cases/regressions.rs` `a_file_dropped_on_a_dock_tile_opens_with_its_app`: the Dock.
- Journey `e2e-default-apps` (`.claude/v012/apps-e2e/journeys/`): `xdg-mime query default` after a pick (forge only).
