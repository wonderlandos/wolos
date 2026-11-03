# SPDX-License-Identifier: BSD-2-Clause
# The section 3.1 Todos example as a spec fixture, shared by the recorder specs and the engine specs.
require "../src/wolkit/core"

class SpecTodos < Wolkit::App
  @items = list(String)
  @draft = signal("")

  getter items
  getter draft

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
