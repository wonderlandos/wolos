# SPDX-License-Identifier: BSD-2-Clause
# The HIG components in one window, shared by the recorder specs and the engine specs.
require "./spec_helper"
require "../src/wolkit/kit"

class KitApp < Wolkit::App
  @menu_open = signal(false)
  @sheet_open = signal(false)
  @tab = signal(0)
  @flavour = signal(1)
  getter menu_open
  getter sheet_open
  getter tab

  def view
    app_window "Notes", width: 800, height: 500 do
      toolbar do
        button "New", primary: true
        trigger = button "More", label: "More actions"
        menu open: @menu_open, anchor: trigger, items: [
          Wolkit::Kit::MenuItem.new("Rename", -> { }),
          Wolkit::Kit::MenuItem.new("Delete", -> { }, disabled: true),
        ]
      end
      row grow: 1 do
        sidebar do
          section title: "Folders" do
            list_row title: "Inbox", subtitle: "3 new"
            list_row title: "Archive"
          end
        end
        column grow: 1 do
          tabs labels: ["All", "Starred"], selected: @tab
          dropdown options: ["Vanilla", "Mint"], selected: @flavour, label: "Flavour"
          empty_state title: "Nothing here", body: "Add a note to get started." do
            button "Add note"
          end
        end
      end
      sheet open: @sheet_open, title: "Settings" do
        text "Preferences go here"
      end
    end
  end
end
