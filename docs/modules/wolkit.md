# Wolkit: the Crystal UI kit (ADR 0039)

Wolkit is the Crystal shard for writing a WolOS app that draws its own window: a declarative DSL, a signal-based reactive runtime, a headless test harness and the developer tools. It lives in `wolos-crystal/shards/wolkit/` (BSD-2-Clause, namespace `Wolkit`), builds on the Cryswol shard (`docs/modules/cryswol.md`, ADR 0036) and talks to the engine `libwolkit.so.1` (`docs/modules/libwolkit.md`, code in `crates/wolkit`). The style language is `docs/modules/wolkit-style.md`; a first app in fifteen minutes is `docs/modules/wolkit-tutorial.md`. Examples: `wolos-crystal/examples/{counter,todos}`; template: `wolos-crystal/templates/app`.

```crystal
class Todos < Wolkit::App
  @items = list(String)
  @draft = signal("")

  def view
    window "Todos", width: 420, height: 560 do
      column gap: :md, pad: :lg do
        row gap: :sm do
          text_field @draft, placeholder: "What needs doing?", grow: 1, on_submit: -> { add }
          button "Add", primary: true, disabled: @draft.blank?, on_click: -> { add }
        end
        each @items, key: ->(t : String) { t } do |todo|
          row { text todo, grow: 1; icon_button :trash, label: "Delete #{todo}", on_click: -> { @items.delete(todo) } }
        end
        text "#{@items.size} left", role: :caption
      end
    end
  end

  def add
    @items << @draft.value.strip unless @draft.blank?
    @draft.value = ""
  end
end
Wolkit.run Todos
```

There is no colour, pixel, hex or markup in it. This is the program `wolos-crystal/examples/todos/src/todos.cr`; `spec/dsl_app_spec.cr` compiles it and `spec/golden/` pins what it sends.

## Why

The engine owns everything that must be fast and safe: the scene, style, layout, text, paint, input and the accessibility tree. The shard owns what must be pleasant to write: the DSL, reactivity, components and the tools. The two meet at two binary streams, a patch stream the app sends (`wolkit_window_apply`) and an event stream it reads (`wolkit_engine_read_events`), so a state change costs one `memcpy` and one call per window per frame, and the Crystal heap holds only signals, closures and app data (the GC section has the numbers).

## Layers

| Layer | Files under `src/wolkit/` | What it does |
|---|---|---|
| Wire | `patch/{wire,writer,describe}.cr`, `event.cr` | ids mirrored from `wolkit.h`, the patch encoder, a patch decoder for specs, the event records |
| Reactive | `reactive/{source,scope,batch,effect,signal,computed,list,timer,background,resource,persisted}.cr`, `ui.cr`, `guard.cr`, `store.cr`, `context.cr`, `env.cr` | the dependency graph, scopes, batches, the UI queue, error routing |
| Mount | `mount.cr`, `region.cr`, `window.cr`, `key.cr` | node ids, the insertion point, handlers, rows, windows |
| DSL | `dsl/{tables,scale,check,elements,bindings}.cr`, `view.cr`, `app.cr`, `component.cr`, `navigator.cr` | the element macros and the classes a program is made of |
| HIG | `kit/{scaffold,window,lists,overlays}.cr` | AppWindow, Toolbar, Sidebar, Section, ListRow, EmptyState, Sheet, Menu, Dropdown, Alert, Tabs |
| Engine | `lib_wolkit.cr` (generated from `wolkit.h`), `ffi/{call,engine,window,style_table}.cr`, `engine_backend.cr`, `loop.cr`, `runtime.cr`, `gc_policy.cr`, `services.cr` | the only files with pointers, the loop, `Wolkit.run` |
| Tools | `dev/{errors,snapshot,hot_reload,hud}.cr` (only with `-Ddev`), `test/{recorder,harness,png}.cr`, `template.cr`, `bin/wolkit-new.cr` | hot reload, test harness, new projects |

`require "wolkit"` loads everything but the tests; `require "wolkit/test"` adds the headless harness; `require "wolkit/core"` loads only what needs no engine (specs, tools). Pointers exist only in `src/wolkit/ffi/*.cr` and `lib_wolkit.cr` (RCR-010, RCR-011).

## Reactivity

Reactivity is fine-grained and has no virtual tree. A `view` method runs once, at mount. Each element macro expands, at compile time, to straight-line code: allocate a node id, write the `CREATE` and static `SET_*` ops, insert. A value that is a literal (`gap: :md`, `"Add"`) is written once. Any other expression (`disabled: @draft.blank?`, `"#{@items.size} left"`) becomes an effect that runs again when a signal it read changes, and only that effect runs.

| Piece | Behaviour |
|---|---|
| `signal(v)` | `Signal(T)`: `value` (tracked read), `value=` (queues dependents when `!=`), `peek`, `update { }`, `mutate { }`, `set!`, `set_later(v)` from any thread. Unknown methods forward to the value (`@draft.blank?`). At class level the type is taken from the literal; otherwise write `signal(nil, of: String?)` |
| `computed { }` | lazy, cached, invalidated when a source changes; effects that read it are queued |
| `effect { }` | runs now and again on change, at most once per flush; `on_cleanup { }` inside runs before each re-run and at dispose; an effect that read nothing is dropped |
| `batch { }` | writes queue; the effects flush once, oldest first, when the outermost batch ends. Event handlers and timers run in a batch. More than 100 rounds in one flush is reported as a reactive cycle |
| `untracked { }`, `peek` | read without depending |
| `list(T)` | `List(T)`: `<<`, `insert`, `delete`, `delete_at`, `[]=`, `move`, `clear`, `replace`, `sort_by!`; `size`, `each` and `[]` are tracked. Mutations are recorded as Insert, Remove, Move, Reset |
| `each list, key: ->(x) { } do \|x\| ... end` | one row per item; follows the change records, so adding builds one row and removing frees one. Over a `Readable(Array(T))` rows are matched by key; over a plain `Array` the rows are built once |
| `show(cond) { }`, `switch(readable) { \|v\| }` | mount while true; rebuild when the value changes |
| `resource(source) { \|v\| fetch(v) }` | the fetch runs on a small thread pool (never on the UI context); `loading?`, `value`, `error` are signals; a result for an older input is dropped; `refetch` |
| `every(span) { }`, `after(span) { }` | timers that stop when the scope that made them is disposed; the block runs on the UI context in a batch |
| `persisted(key, default)` | a signal saved in the current `Wolkit::Store` (the app's `wol_prefs` once `Wolkit::Store.current=` is set, in memory by default); if the store refuses, the signal keeps working and `notice` holds the sentence to show |
| `provide(v)` / `inject(T)` | context values by type through the scope chain |
| `Wolkit::Env` | read-only signals: `dark`, `accent`, `reduce_motion`, `reduce_transparency`, `contrast`, `scale`, `text_scale`; updated from the engine's theme and resize events |

Every row, `show` branch, component and overlay owns a `Scope`. Disposing a scope disposes its effects, timers and handlers, frees its parts (one `FREE` for the root; the engine frees the subtree) and gives its ids back. The mount reuses a slot with a new generation only after the patch that freed it has been applied.

**Threads (ADR 0036).** Signals are read and written on the UI context only. In a `-Ddev` build a write from another thread raises ``Wolkit::Signal(Int32)#value= was called off the UI context. Use `set_later`, or `Wolkit::UI.post { ... }`.`` Heavy work goes to `resource` or `Wolkit::Background.run` and returns through `set_later` or `UI.post`.

## Elements

An element macro takes at most one positional value, then named properties and a block. Each element has its own macro (so a compile error points at the line the person wrote). `Wolkit::DSL::ELEMENTS` in `src/wolkit/dsl/tables.cr` is the table.

| Element | Positional | Takes (besides the common properties) | Events |
|---|---|---|---|
| `box`, `column`, `row` | none | a block | `on_click`, `on_hover` |
| `scroll` | none | a block | `on_scroll` |
| `spacer`, `divider` | none | | |
| `text` | the words (required) | `role` (type scale) | |
| `image` | none | `src` (a file path; `source` is the same), `picture` (an `FFI::Image`), `fit` (`:contain`, `:cover`, `:fill`, `:none`); needs `alt:` or `decorative: true` | |
| `button` | the label | `primary`, `danger`, `flat` | `on_click`, `on_hover` |
| `icon_button` | an icon name (`:trash`) | `primary`, `danger`, `flat`; needs `label:` | `on_click`, `on_hover` |
| `toggle` | a `Signal(Bool)` | needs `label:` | `on_toggle` |
| `slider` | a `Signal(Float64)` | `min`, `max`, `step`; needs `label:` | `on_change` |
| `text_field` | a `Signal(String)` (two-way) or a start text | `placeholder`, `max_length`, `secure`, `readonly` | `on_change`, `on_submit` |
| `vlist`, `canvas`, `overlay` | none | `overlay`: `anchor`, `side`, `anchor_align`, `modal` | `on_range`, `on_scroll`, `on_frame` |

**Common properties** (`COMMON` in the same file): `direction wrap grow shrink basis gap row_gap column_gap align align_self align_content justify pad pad_top pad_right pad_bottom pad_left margin margin_top margin_right margin_bottom margin_left width height min_width min_height max_width max_height aspect position top right bottom left overflow z tab_index bg border radius opacity outline translate scale rotate cursor pointer_events visibility backdrop color size weight line_height letter_spacing text_align decoration text_overflow white_space max_lines tnum italic selectable spring tooltip focusable`. Also `class:` (style class ids from `wolkit-stylec`), the state flags `disabled selected checked expanded busy` (one `SET_STATE` mask, not a restyle), and the accessibility arguments `label:`, `description:`, `a11y_role:` (`image` takes `alt:` for its name).

**Values come from the design scale.** Spacing is `:none :xs :sm :md :lg :xl` (0, 4, 8, 12, 16, 24 px, the scale of the style language); radius `:none :s :m :l :xl`; colours are the 13 theme tokens (`:surface :sidebar :card :line :text :dim :accent :accentText :glass :control :success :warning :danger`); type roles `:display :large :title :body :secondary :caption`; enum properties take the engine's names with `_` for `-` (`align: :center`, `justify: :space_between`). A size is a number (px), `:auto` or `"50%"`. A literal off the scale is a compile error, a computed one raises `Wolkit::UsageError` with the same sentence.

**Events** are procs: `on_click: -> { add }`, or with the payload `on_change: ->(text : String) { }` (text fields), `Bool` (toggle), `Float64` (slider). A handler that raises goes to `Wolkit.on_error` (Cryswol's `Wol::Loop.on_error`, through `Wol::Callbacks.guard`, the one place that catches everything) and the UI goes on. Text fields are controlled: the app's signal and the engine's text follow each other, and an edit made before the app last wrote the text is ignored by its sequence number (`Mount#stale_text?`), so an IME composition is never overwritten.

### Pictures

```crystal
image src: photo.thumbnail, fit: :cover, radius: :m, width: 160, height: 120, alt: "Harbour at dusk"
image src: "/usr/share/backgrounds/wolos/dunes.jpg", fit: :contain, bg: :sidebar, alt: "Dunes"
image picture: @scan, fit: :none, alt: "The scanned page"   # an FFI::Image the app holds
image src: icon_path, decorative: true                       # no name for assistive technology
```

`src:` is an absolute path (or a signal of one): the engine reads and decodes the file on its own worker threads (PNG and JPEG, turned upright by the EXIF orientation), at a size that fits the part, and caches it per process, so a grid of thumbnails costs little. Until the picture is ready the part shows a placeholder in the `control` colour; the window draws again by itself when it is. `fit:` places it (`:contain` is the default), `radius:` rounds it, `opacity:` fades it (a bare picture needs no offscreen layer for that), `bg:` shows where `:contain` leaves room. `alt:` is required (what the picture shows, in a few words) unless `decorative: true`; the compile error says so. For pictures made in memory, `Wolkit.load_image(bytes, max_side: 512)` or `Wolkit.load_image(path, max_side:)` returns an `FFI::Image` (`size`, `state`, `id`; `from_rgba` for raw pixels): pass it as `picture:` and keep it while it is shown. The engine side is `docs/modules/libwolkit.md`, Pictures.

## Components

```crystal
class Counter < Wolkit::Component
  prop start : Int32 = 0
  @n = signal(0)

  def view
    @n.value = @start
    row gap: :sm do
      button "-", label: "Decrease", on_click: -> { @n.value -= 1 }
      text "#{@n}"
      button "+", label: "Increase", on_click: -> { @n.value += 1 }
    end
  end
end
  # used as:  counter start: 5
```

`prop` gives a typed argument (required without a default); a prop typed `Readable(T)` also accepts a plain `T`. The class `Counter` defines the element `counter`; a block given to it is rendered by `slot`. Lifecycle: `on_mount { }`, `on_unmount { }`, `on_cleanup { }`. Fragments are plain methods that call element macros while a view is mounting; there is no second concept.

## HIG components

`src/wolkit/kit/`. Dimensions are `Wolkit::Scaffold` (mirrors `shared::theme::scaffold`; `spec/kit_spec.cr` compares the two). Elements: `app_window "Title", width:, height:`, `toolbar`, `sidebar`, `section title:`, `list_row title:, subtitle:, on_click:`, `empty_state title:, body:`, `sheet open:, title:`, `menu open:, anchor:, items:`, `dropdown options:, selected:, label:`, `alert open:, title:, message:, actions:`, `tabs labels:, selected:`. Overlays are mounted while their signal is true and freed when it turns false.

## The default look

Every window starts with the kit look: `styles/kit.wolcss` in the shard, compiled by `wolkit-stylec` into `src/wolkit/style/kit_table.cr` (the WSB1 bytes, `Wolkit::KIT_TABLE`) and `kit_classes.cr` (the class ids, `Wolkit::KitClass`). It holds the rules for every part kind (button, text field, toggle, slider, divider, overlay: padding, radius, fills and states, all theme tokens, so dark mode, high contrast and the accent colour follow with no code) and the classes the DSL names (`primary`, `danger`, `flat`, `card`, `heading`, `menu-item`, `tab`, `sheet`). The HIG components add layout and these classes, not colours. `make styles` regenerates both files, and `spec/scripts/kit_styles.sh` compiles the sheet again and fails when they drift. A class id is its first appearance in the sheet.

An app with its own sheets compiles the kit sheet **first**: `wolkit-stylec --out build/styles.wsb --crystal src/gen/styles.cr build/00-kit.wolcss src/styles/*.wolcss` (the template's `make styles` copies the kit sheet to `build/00-kit.wolcss`; sheets are read in sorted order), then calls `Wolkit.use_styles("build/styles.wsb")`. The kit ids stay 0 to 7 and the app's classes follow, as the generated `Wolkit::Styles` constants say. A table made without the kit sheet replaces the default look.

## Navigator

`Wolkit::Navigator(Route)` is a typed stack: `push`, `back`, `replace`, `reset`, `current` (a readable), `depth`. `navigator @nav do |route| ... end` shows the top route and rebuilds the screen when it changes; the routes are your own values (a `Symbol`, a record), so a screen's arguments are checked by the compiler.

## Services

`Wolkit::Services` wraps the libwol verbs so a refusal is shown, not raised: `notify(title, body) : String?`, `copy(text) : String?`, `paste : Outcome(String)`, and the blocking panels `open_file`, `save_file`, `share`, which run on `Wol.background` and call a block on the UI context with an `Outcome` (a value, or the sentence libwol wrote, for example "Allow this app to ... in System Settings"). Show the sentence inline as the HIG asks.

## Testing without a display

`require "wolkit/test"`:

```crystal
Wolkit::Test.run(Todos) do |t|
  t.type "milk", into: "What needs doing?"
  t.click "Add"
  t.text("1 left").should eq "1 left"
  t.a11y_snapshot            # the accessibility tree as text
  t.advance(100.milliseconds)
  png = t.screenshot         # CPU-rasterised PNG, the same bytes on every machine
end
```

The session starts the real engine headless: no fake wold, no Kit host. Parts are found by accessible name (or their text); `click` presses at the centre of the part's rectangle. Pure DSL specs use `Wolkit::Test.record(app)` instead: a recorder backend that keeps each patch, decoded to one line per op by `Wolkit::Patch.describe`.

**Specs of the shard.** `spec/golden/*.ops` pin what the DSL sends; `spec/golden/engine/*.scene|a11y` pin what the engine makes of it (`wolkit_window_dump`), and `spec/golden/engine/pictures.png` the pixels of the `image` fits (`spec/engine/image_spec.cr`); `spec/compile_errors/*.crfail` are programs that must not compile, each with the message and the line the compiler must print (`# expect:`, `# <= error`). `WOLKIT_BLESS=1 make spec` rewrites the goldens; review the diff.

## Compile errors

Messages are sentences, and the compiler points at the line you wrote:

```
In src/views/todo.cr:12:7
 12 | text_field @draft, placehodler: "What needs doing?"
      ^---------
Error: text_field has no property `placehodler`. Did you mean `placeholder`?
```

Pinned by `spec/compile_errors_spec.cr`: unknown property with a suggestion (edit distance, computed in the macro), `icon_button needs `label:`. Screen readers and tooltips use it.`, `Spacing is one of 0, 4, 8, 12, 16 or 24; this is 13.`, `Colours are theme tokens: ...; this is :purple.`, an unknown enum value, a handler with the wrong arity, `text` without words, ``image needs `alt:` (what the picture shows), or `decorative: true`. Screen readers use it.``, and a non-Bool `disabled:` (the compiler's own overload message, `expected argument #1 to 'Wolkit::Check.bool' to be Bool, not String`, with the caret under the argument). `.wolcss` errors are the engine's (`wolkit-style.md`). Runtime sentences: ``This window has 65,537 parts; the limit is 65,536. Use `list` for long lists.`` and the off-UI-context one above.

## Developer tools (`-Ddev`)

A dev build listens on `$XDG_RUNTIME_DIR/wolkit-dev-<pid>.sock` (mode 0600; one JSON line in, one out, lines at most 8 MiB, 30 s idle limit): `style` (a compiled sheet, base64; every window restyles with no restart), `error` and `clear` (a banner with the compiler's first line over the first window), `snapshot` (the app's JSON-serialisable signals by `Class#ivar`, restored by the next process from `$WOLKIT_DEV_SNAPSHOT`) and `inspect` (`scene`, `layout`, `a11y` dumps). `WOLKIT_HUD=1` shows the engine's frame statistics. A release build contains none of this code (each file is empty without `-Ddev`). A code change is a rebuild: see the numbers below. Snapshots cover the app class's signals, not component-local ones; `persisted` signals return from the store.

## The collector, and what was measured

The scene lives in Rust, so the Crystal heap stays small. `Wolkit::GCPolicy` collects only when the window has been idle for 100 ms and more than 1 MB was allocated since the last collection, so no collection starts in the middle of an interaction. Measured on the build forge (Crystal 1.22.2, `plan-wolkit.md` "S0 results"):

| Fact | Result |
|---|---|
| Full collection of a pointer-dense heap, 16 MB / 64 MB | 1.4 ms / 4.5 ms with parallel marking; 3.9 ms / 24 ms with one marker |
| Frame time under the allocation pattern of a UI, max, default policy / idle policy | 5.9 ms (64 MB, parallel) / 0.39 ms |
| Allocation per interaction (a text change, one effect, one patch) | 285 bytes (budget: 8 KB) |
| Allocation per add and remove of a row | 1.9 KB |
| Allocation at idle | 0 bytes over 10,000 turns |
| Effect flush, 20 dependents | far under 0.5 ms (`spec/budget_spec.cr` asserts it) |
| FFI call | 0.65 ns; a syscall 198 ns |
| Rebuild of a 3,081-line app of 70 screens (`crystal build`, 128 vCPU forge) | type-check only 1.8 s, debug build 3.8 s, one-line rebuild 2.5 s, release 17.7 s (target: at most 5 s for 2,000 lines) |

The numbers are one run each on a machine with many more cores than a target machine; the single-marker column is the one to plan with. `tools/bench/wolkit/run.sh` repeats the shard-side measurements 30 times (median / p95): full collection at a 16 MB heap 1.5 / 1.6 ms and at 64 MB 4.9 / 6.0 ms with parallel marking, 2.9 / 3.0 ms and 13.4 / 13.7 ms with `GC_MARKERS=1`; about 750 bytes allocated per text change through the real engine; a 100-op patch encoded in 2.5 microseconds.

**The loop.** `Wolkit::Loop.run` (`src/wolkit/loop.cr`) is one fiber that takes a turn when woken, plus a small fiber that watches the engine's descriptor. The descriptor is a self-pipe (`src/loop_/mod.rs`): the watcher reads a duplicate of it, which only consumes wake-up bytes, and the turn calls `dispatch(0)`, which runs what is queued whether or not a byte was there, so no wake-up is lost. If the engine ever hands out a descriptor that cannot be read (an epoll descriptor, `EINVAL`), the watcher waits with `Crystal::EventLoop.current.wait_readable`, an internal method of the Crystal runtime, used in that one place. `Runtime.quit` and `Wolkit::Loop.wake` also wake the loop, so a window closed from a timer ends the program at once. The headless engine queues injected events on the calling thread and does not write to the descriptor (the real backends do), so the harness calls `Loop.turn` itself.

## Limits and where this differs from the plan

- Patches carry at most 1 MiB (the writer asks the mount to send early); a window at most 65,536 parts; a text 64 KiB; 8 style classes per part.
- Type errors in property values use the compiler's overload message, not a custom sentence: a custom one loses the line the person wrote.
- The kit-backend limits (200 parts, no canvas) apply when the engine runs the Kit backend; `canvas` and `surface` need the native backend.
- `vlist`, `canvas`, `surface` and the Vulkan surface are exposed as elements and engine verbs; their DSL helpers (`Canvas`, `Surface`, virtual rows) belong to the GPU stream.

## Adding to the shard

A property: one row in `PROPS` (`dsl/tables.cr`) and, if the engine has a new id, `patch/wire.cr` (`spec/wire_spec.cr` compares with `abi/wolkit-1.0.props`). An element: one row in `ELEMENTS`. An HIG component: a `Wolkit::Component` in `kit/`. A rule for Crystal code: `docs/RULES-crystal.md`.
