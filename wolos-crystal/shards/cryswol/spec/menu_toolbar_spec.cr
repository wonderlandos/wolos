# SPDX-License-Identifier: BSD-2-Clause
# Menus and toolbars against the fake Kit host (spec/fixtures/menu.json): after the first show the
# host chooses File > open, File > save (greyed out), File > nope (no such item), View > grid,
# the toolbar's back and share (greyed out), then closes the window.
require "./spec_helper"

module MenuFixture
  class Seen
    property chosen = [] of String
    property closed = 0
  end

  record Parts, window : Wol::Window, file : Wol::Menu, view : Wol::Menu, toolbar : Wol::Toolbar, seen : Seen

  def self.build(ctx : Wol::Context) : Parts
    seen = Seen.new
    window = Wol::Window.new(ctx, "Menu spec")
    window.content = Wol::Label.new(ctx, "Body")
    file = Wol::Menu.new(ctx, "File") { |menu, key| seen.chosen << "#{menu.title.downcase}:#{key}" }
    file.add_item("open", "Open", "cmd+o").add_item("save", "Save", "cmd+s").add_separator.add_item("quit", "Quit")
    file.set_enabled("save", false)
    view = Wol::Menu.new(ctx, "View") { |_, key| seen.chosen << "view:#{key}" }
    view.add_item("grid", "Grid").set_checked("grid", true)
    toolbar = Wol::Toolbar.new(ctx) { |_, key| seen.chosen << "tool:#{key}" }
    toolbar.add_button("back", "Back", "go-previous-symbolic").add_spacer.add_button("share", "Share")
    toolbar.set_enabled("share", false)
    window.add_menu(file).add_menu(view)
    window.toolbar = toolbar
    window.on_close { seen.closed += 1 }
    Parts.new(window, file, view, toolbar, seen)
  end
end

describe Wol::Menu do
  it "sends the menus and the toolbar with the window" do
    Wol::Spec.with_fake("menu") do |ctx, fake|
      parts = MenuFixture.build(ctx)
      parts.window.show
      Wol::Spec.run_until(ctx) { fake.shown.size >= 2 }.should be_true
      show = fake.shown[1]
      menus = show["menus"].as_a
      menus.map(&.["title"].as_s).should eq %w(File View)
      menus[0]["entries"].should eq JSON.parse(<<-JSON)
        [{"type":"item","key":"open","label":"Open","shortcut":"cmd+o"},
         {"type":"item","key":"save","label":"Save","shortcut":"cmd+s","enabled":false},
         {"type":"separator"},
         {"type":"item","key":"quit","label":"Quit"}]
        JSON
      menus[1]["entries"].should eq JSON.parse(%([{"type":"item","key":"grid","label":"Grid","checked":true}]))
      show["toolbar"]["tools"].should eq JSON.parse(<<-JSON)
        [{"type":"button","key":"back","label":"Back","icon":"go-previous-symbolic"},
         {"type":"spacer"},
         {"type":"button","key":"share","label":"Share","enabled":false}]
        JSON
      parts.window.menus.should eq [parts.file, parts.view]
      parts.window.toolbar.should be(parts.toolbar)
    end
  end

  it "delivers only enabled, known keys, each to its own menu or toolbar" do
    Wol::Spec.with_fake("menu") do |ctx, fake|
      parts = MenuFixture.build(ctx)
      parts.window.show
      Wol::Spec.run_until(ctx) { parts.seen.closed > 0 }.should be_true
      parts.seen.chosen.should eq ["file:open", "view:grid", "tool:back"]
      fake.problems.should be_empty
    end
  end

  it "redraws a shown window when an item changes" do
    Wol::Spec.with_fake("menu") do |ctx, fake|
      parts = MenuFixture.build(ctx)
      parts.window.show
      parts.file.set_enabled("save", true)
      parts.toolbar.set_enabled("share", true)
      Wol::Spec.run_until(ctx) { fake.shown.size >= 4 }.should be_true
      fake.shown[3]["menus"][0]["entries"][1]["enabled"]?.should be_nil
      fake.shown[3]["toolbar"]["tools"][2]["enabled"]?.should be_nil
    end
  end

  it "refuses bad keys, repeated keys, unknown keys and bad shortcuts" do
    Wol::Spec.with_fake("menu") do |ctx, _fake|
      menu = Wol::Menu.new(ctx, "Edit") { }
      expect_raises(Wol::InvalidArgument, "A key is 1 to 32 lowercase letters") { menu.add_item("Bad Key", "Bad") }
      menu.add_item("copy", "Copy", "cmd+c")
      expect_raises(Wol::InvalidArgument, "Each item needs its own key.") { menu.add_item("copy", "Again") }
      expect_raises(Wol::InvalidArgument, "There is no item with that key.") { menu.set_enabled("nope", true) }
      expect_raises(Wol::InvalidArgument, "There is no item with that key.") { menu.set_checked("nope", true) }
      expect_raises(Wol::InvalidArgument, "A shortcut is like cmd+s or cmd+shift+n.") do
        menu.add_item("paste", "Paste", "not a shortcut")
      end
      toolbar = Wol::Toolbar.new(ctx) { }
      expect_raises(Wol::InvalidArgument, "A tool icon is an icon name.") { toolbar.add_button("open", "Open", "/a/path.png") }
      toolbar.add_button("open", "Open")
      expect_raises(Wol::InvalidArgument, "Each button needs its own key.") { toolbar.add_button("open", "Again") }
      expect_raises(Wol::InvalidArgument) { toolbar.set_enabled("nope", false) }
    end
  end

  it "puts a menu and a toolbar on one window only" do
    Wol::Spec.with_fake("menu") do |ctx, _fake|
      parts = MenuFixture.build(ctx)
      other = Wol::Window.new(ctx, "Other")
      expect_raises(Wol::InvalidArgument, "This menu is already on a window.") { other.add_menu(parts.file) }
      expect_raises(Wol::InvalidArgument, "This toolbar is already on a window.") { other.toolbar = parts.toolbar }
      other.menus.should be_empty
      other.toolbar.should be_nil
    end
  end

  it "removes the toolbar with nil" do
    Wol::Spec.with_fake("menu") do |ctx, fake|
      parts = MenuFixture.build(ctx)
      parts.window.toolbar = nil
      parts.window.toolbar.should be_nil
      parts.toolbar.placed?.should be_false
      parts.window.show
      Wol::Spec.run_until(ctx) { fake.shown.size >= 2 }.should be_true
      fake.shown[1]["toolbar"]?.should be_nil
      # Free again: it may go on another window.
      Wol::Window.new(ctx, "Other").toolbar = parts.toolbar
    end
  end

  it "keeps a closed menu or toolbar on a window rooted, and drops its choices" do
    Wol::Spec.with_fake("menu") do |ctx, _fake|
      parts = MenuFixture.build(ctx)
      rooted = ctx.rooted_count
      parts.file.close
      parts.toolbar.close
      ctx.rooted_count.should eq rooted
      parts.window.show
      3.times { GC.collect }
      Wol::Spec.run_until(ctx) { parts.seen.closed > 0 }.should be_true
      parts.seen.chosen.should eq ["view:grid"]
    end
  end

  it "unroots a menu or toolbar on no window when it is closed" do
    Wol::Spec.with_fake("menu") do |ctx, _fake|
      menu = Wol::Menu.new(ctx, "Loose") { }
      toolbar = Wol::Toolbar.new(ctx) { }
      ctx.rooted_count.should eq 2
      menu.close
      toolbar.close
      ctx.rooted_count.should eq 0
      expect_raises(Wol::UsageError) { menu.add_separator }
      expect_raises(Wol::UsageError) { toolbar.add_spacer }
    end
  end
end
