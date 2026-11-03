# SPDX-License-Identifier: BSD-2-Clause
# AppWindow, Toolbar and Sidebar: the standard frame of a WolOS app (HIG sections 1 and 2). The
# dimensions are the scaffold constants; the look comes from the Kit style table.
#
# ```
# app_window "Notes", width: 900, height: 600 do
#   toolbar { button "New", primary: true }
#   row grow: 1 do
#     sidebar { list_row "Inbox" }
#     column(grow: 1) { text "Pick a note" }
#   end
# end
# ```
require "../component"
require "./scaffold"

module Wolkit
  module Kit
    # A window with the standard content column.
    class AppWindow < Component
      prop title : String
      prop width : Int32 = 900
      prop height : Int32 = 600

      def view
        window @title, width: @width, height: @height do
          column grow: 1, bg: :surface do
            slot
          end
        end
      end
    end

    # The toolbar row at the top of a window.
    class Toolbar < Component
      def view
        row height: Scaffold::TOOLBAR_HEIGHT, pad_left: :lg, pad_right: :lg, align: :center, gap: :sm, bg: :surface do
          slot
        end
      end
    end

    # The sidebar column at the left of a window.
    class Sidebar < Component
      def view
        column width: Scaffold::SIDEBAR_WIDTH, bg: :sidebar, pad: :sm, gap: :xs do
          slot
        end
      end
    end
  end
end
