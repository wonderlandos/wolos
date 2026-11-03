# SPDX-License-Identifier: BSD-2-Clause
# Specs for the HIG components: they mount, follow their signals, and keep the scaffold constants.
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

describe "the HIG components" do
  after_each { Wolkit::Test.reset }

  it "mounts the frame, the lists and the closed overlays" do
    recorder = Wolkit::Test.record(KitApp.new)
    window = recorder.windows.first
    window.title.should eq "Notes"
    ops = window.ops
    ops.should contain("SET_PROP 2 height px:52.0")
    ops.any?(&.includes?("width px:220.0")).should be_true
    golden("kit_mounted.ops", ops.join("\n") + "\n")
  end

  it "mounts a sheet while its signal is true and frees it after" do
    app = KitApp.new
    recorder = Wolkit::Test.record(app)
    window = recorder.windows.first
    app.sheet_open.value = true
    Wolkit::Test.settle
    window.last_ops.any?(&.includes?("SET_A11Y")).should be_true
    window.last_ops.any?(&.includes?("\"Preferences go here\"")).should be_true
    app.sheet_open.value = false
    Wolkit::Test.settle
    window.last_ops.first.should start_with("FREE ")
  end

  it "selects a tab" do
    app = KitApp.new
    recorder = Wolkit::Test.record(app)
    window = recorder.windows.first
    Wolkit::Test.click(window, "Starred")
    app.tab.value.should eq 1
    window.last_ops.any?(&.includes?("SET_STATE")).should be_true
  end

  it "keeps the scaffold constants of shared::theme" do
    path = File.join(__DIR__, "../../../../crates/shared/src/theme.cr".sub(".cr", ".rs"))
    next unless File.exists?(path)
    source = File.read(path)
    {"TOOLBAR_HEIGHT" => Wolkit::Scaffold::TOOLBAR_HEIGHT, "TOOLBAR_PADDING" => Wolkit::Scaffold::TOOLBAR_PADDING,
     "SIDEBAR_WIDTH" => Wolkit::Scaffold::SIDEBAR_WIDTH, "LIST_ROW" => Wolkit::Scaffold::LIST_ROW,
     "CONTROL_HEIGHT" => Wolkit::Scaffold::CONTROL_HEIGHT, "RADIUS_CONTROL" => Wolkit::Scaffold::RADIUS_CONTROL,
     "RADIUS_CARD" => Wolkit::Scaffold::RADIUS_CARD, "WINDOW_RADIUS" => Wolkit::Scaffold::WINDOW_RADIUS}.each do |name, value|
      source.should match(/pub const #{name}: i32 = #{value};/)
    end
  end
end
