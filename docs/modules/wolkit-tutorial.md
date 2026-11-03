# Your first Wolkit app in fifteen minutes (ADR 0039)

A tutorial for writing a WolOS app with the `wolkit` Crystal shard: a to-do list in eight steps. It builds the program in `wolos-crystal/examples/todos/src/todos.cr`; the reference is `docs/modules/wolkit.md` and the engine is `docs/modules/libwolkit.md`. Every program in this tutorial that starts with `require "wolkit"` is type-checked by `wolos-crystal/shards/wolkit/spec/tutorial_spec.cr`, so it cannot drift from the shard. You need the Crystal toolchain, the `wolkit` and `cryswol` shards (`/usr/lib/wolos/crystal`) and `libwolkit`.

## 1. Make the project (1 minute)

```
wolkit-new todos
cd todos
make run
```

`wolkit-new` (the work behind `wol app new todos --template app`) writes `shard.yml`, `src/app.cr`, `src/views/main.cr`, `src/components/greeting.cr`, `src/styles/app.wolcss`, `spec/app_spec.cr`, a `Makefile` and a README, and the project builds and passes its own spec as it stands. `make run` opens a window with a greeting and a button that counts. Close the window and the program ends.

## 2. Read the smallest app (1 minute)

Replace the generated files with one file, `src/todos.cr`:

```crystal
require "wolkit"

class Todos < Wolkit::App
  @n = signal(0)

  def view
    window "Todos", width: 420, height: 560 do
      column gap: :md, pad: :lg do
        text "#{@n} left", role: :caption
        button "More", on_click: -> { @n.value += 1 }
      end
    end
  end
end

Wolkit.run Todos
```

`view` runs once. It does not draw: it describes. `signal(0)` is a value that remembers who read it. `text "#{@n} left"` reads `@n` while the window is built, so Wolkit keeps one small effect for that text; clicking the button writes `@n`, that effect runs, and one `SET_TEXT` goes to the engine. Nothing else is touched. Spacing and type are names from the design scale (`:md`, `:caption`); there is no number of pixels, no colour, no markup.

## 3. Watch it change while it runs (2 minutes)

Build with `-Ddev` and the running app listens for changes: a saved `.wolcss` sheet restyles the window with no restart (under 100 ms), a saved `.cr` file rebuilds and the new process takes over with the old process's signals put back, and a compile error is shown as a banner in the window that is still running. `wol app run --watch` drives this (docs/modules/wolkit.md "Developer tools"). Try it now: change `"More"` to `"Add one"`, save, and the window follows.

## 4. A text field and a button (2 minutes)

```crystal
require "wolkit"

class Todos < Wolkit::App
  @draft = signal("")
  @added = signal(0)

  def view
    window "Todos", width: 420, height: 560 do
      column gap: :md, pad: :lg do
        row gap: :sm do
          text_field @draft, placeholder: "What needs doing?", grow: 1, on_submit: -> { add }
          button "Add", primary: true, disabled: @draft.blank?, on_click: -> { add }
        end
        text "#{@added} added", role: :caption
      end
    end
  end

  def add
    @added.value += 1 unless @draft.blank?
    @draft.value = ""
  end
end

Wolkit.run Todos
```

`text_field @draft` is two-way: typing writes the signal and clearing the signal clears the field. `disabled: @draft.blank?` is not a literal, so it is an effect that reads `@draft`; the button is disabled while the field is empty, and typing sends one small `SET_STATE`, never the text back. The field has a placeholder, and Wolkit uses it as the accessible name; a part without a name is an error in tests and a warning while developing.

## 5. A list (3 minutes)

```crystal
require "wolkit"

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

`list(String)` is an array that says what changed. `each` follows those change records: adding a todo builds one row, deleting one frees exactly that row. There is no diff and no rebuild of the list. `icon_button` must have `label:`; leave it out and the compiler stops, pointing at your line: ``icon_button needs `label:`. Screen readers and tooltips use it.`` This is the whole app of `wolos-crystal/examples/todos`.

## 6. Style it (2 minutes)

Inline arguments are checked at compile time: `pad: :lg`, `bg: :card`, `radius: :m`. A size off the scale, such as `gap: 13`, is a compile error that lists the scale; a colour must be a theme token, so the app follows dark mode, contrast and the accent colour with no code. For your own classes, write `src/styles/app.wolcss`:

```
.done { color: dim; text-decoration: strikethrough; }
```

`make styles` compiles it, after the kit look (`build/00-kit.wolcss`, so the default look keeps its class ids), with `wolkit-stylec` into `build/styles.wsb` and `src/gen/styles.cr` (one constant per class); load the sheet with `Wolkit.use_styles("build/styles.wsb")` and name a class with `class: Wolkit::Styles::DONE`. The language, its properties and its errors are in `docs/modules/wolkit-style.md`.

## 7. Test it without a display (2 minutes)

```crystal
require "spec"
require "wolkit/test"
require "./todos"

describe Todos do
  it "adds and deletes a todo" do
    Wolkit::Test.run(Todos) do |t|
      t.type "milk", into: "What needs doing?"
      t.click "Add"
      t.text("1 left").should eq "1 left"
      t.click "Delete milk"
      t.text("0 left").should eq "0 left"
    end
  end
end
```

`Wolkit::Test.run` starts the real engine with no display and drives it with pointer and key events; `t.a11y_snapshot` prints the accessibility tree and `t.screenshot` returns a PNG that is identical on every machine. `make spec` runs it; `make check` only type-checks.

## 8. Package it (1 minute)

`make` builds a release binary (`crystal build --release`); a package recipe for a Crystal app is the same as for any program in `docs/modules/wopkg.md`: build in the chroot, install the binary and a `.desktop` file, depend on `libwolkit` and `libwol`. Give the app a reverse-DNS id (`$WOLKIT_APP_ID` or `org.wolos.<name>`) so the system can show its permissions.

## What to read next

`docs/modules/wolkit.md` for components, the HIG parts (`app_window`, `toolbar`, `sidebar`, `sheet`, `menu`), the navigator, services (notifications, file panels, share, clipboard), resources and timers; `docs/modules/wolkit-style.md` for the style language.
