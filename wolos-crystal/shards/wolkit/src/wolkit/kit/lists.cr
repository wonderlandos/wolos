# SPDX-License-Identifier: BSD-2-Clause
# Section, ListRow and EmptyState: grouped lists and the empty screen (HIG sections 3 and 5).
require "../component"
require "./scaffold"

module Wolkit
  module Kit
    # A titled group of rows on a card.
    class Section < Component
      prop title : String

      def view
        column gap: :xs do
          text @title, role: :secondary, color: :dim
          column bg: :card, radius: :m do
            slot
          end
        end
      end
    end

    # One row: a title, an optional second line, whatever the block adds on the right, and a click.
    class ListRow < Component
      prop title : Readable(String)
      prop subtitle : Readable(String) = ""
      prop on_click : Proc(Nil)? = nil

      def view
        row height: Scaffold::LIST_ROW, pad_left: :md, pad_right: :md, align: :center, gap: :sm,
          a11y_role: :listitem, label: @title.value, on_click: -> { @on_click.try &.call } do
          column grow: 1 do
            text @title
            text @subtitle, role: :secondary, color: :dim, visibility: (@subtitle.value.empty? ? :hidden : :visible)
          end
          slot
        end
      end
    end

    # What a screen shows when there is nothing to show: a title, a sentence, and the block for an action.
    class EmptyState < Component
      prop title : String
      prop body : String = ""

      def view
        column grow: 1, align: :center, justify: :center, gap: :sm, pad: :xl do
          text @title, role: :title
          text @body, role: :secondary, color: :dim, text_align: :center unless @body.empty?
          slot
        end
      end
    end
  end
end
