# SPDX-License-Identifier: BSD-2-Clause
# Todos: the app of docs/modules/wolkit-tutorial.md. A text field bound to a signal, a button that is
# disabled while the field is empty, a keyed list (one row per todo, built and freed individually) and a
# count. There is no colour, pixel or markup in it: spacing and type are names from the design scale.
require "../../../shards/wolkit/src/wolkit"

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
