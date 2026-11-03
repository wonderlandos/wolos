# SPDX-License-Identifier: BSD-2-Clause
# Counter: the smallest Wolkit app. One signal, one text that follows it, two buttons that change it.
# `text "#{@n}"` is not re-run on every change: the macro made an effect for it, and only that effect
# runs when `@n` changes (docs/modules/wolkit.md "Reactivity").
require "../../../shards/wolkit/src/wolkit"

class Counter < Wolkit::App
  @n = signal(0)

  def view
    window "Counter", width: 280, height: 160 do
      column grow: 1, gap: :md, pad: :lg, align: :center, justify: :center do
        text "#{@n}", role: :display, tnum: true
        row gap: :sm do
          button "-", label: "Decrease", on_click: -> { @n.value -= 1 }
          button "+", label: "Increase", primary: true, on_click: -> { @n.value += 1 }
        end
      end
    end
  end
end
