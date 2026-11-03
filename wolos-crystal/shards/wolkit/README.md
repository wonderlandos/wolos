# wolkit

The Crystal UI kit for WolOS apps (ADR 0039): a declarative DSL, a signal-based reactive runtime, the HIG components, a headless test harness and the developer tools, over the engine `libwolkit.so.1`. It builds on the `cryswol` shard (ADR 0036). BSD-2-Clause.

    require "wolkit"

    class Counter < Wolkit::App
      @n = signal(0)

      def view
        window "Counter", width: 280, height: 160 do
          column grow: 1, gap: :md, pad: :lg, align: :center, justify: :center do
            text "#{@n}", role: :display, tnum: true
            button "+", label: "Increase", primary: true, on_click: -> { @n.value += 1 }
          end
        end
      end
    end

    Wolkit.run Counter

- Reference: `docs/modules/wolkit.md`; first app in fifteen minutes: `docs/modules/wolkit-tutorial.md`; the style language: `docs/modules/wolkit-style.md`; the engine: `docs/modules/libwolkit.md`.
- The default look is `styles/kit.wolcss` (compiled into `src/wolkit/style/kit_*.cr` by `make styles`).
- Examples: `wolos-crystal/examples/counter`, `wolos-crystal/examples/todos`. New project: `wol app new NAME` (`crates/controller/src/app.rs`, templates app, cli, tool and ai); `bin/wolkit-new.cr` writes the app template where `wol` is not installed. Template in `wolos-crystal/templates/app`.
- `make spec` builds libwolkit and libwol into `build/` and runs every spec; `make spec-core` runs the ones that need no engine; `make format` checks the formatter; `make examples` runs the examples' specs. `WOLKIT_BUILD=<dir>` uses libraries built elsewhere. `WOLKIT_BLESS=1 make spec` rewrites the golden files (review the diff).
- `make install` puts the shard in `/usr/lib/wolos/crystal/wolkit` (so `require "wolkit"` works) and the template in `/usr/share/wolos/templates/app`.
- Pointers exist only in `src/wolkit/ffi/*.cr` and the generated `src/wolkit/lib_wolkit.cr`. `lib_wolkit.cr` is generated from `crates/wolkit/include/wolkit.h` by the generator in `crates/wol-abi/tests/common/bindgen.rs`; do not edit it.
- Dev tools exist only in `-Ddev` builds.
