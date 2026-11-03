# SPDX-License-Identifier: BSD-2-Clause
# The kit's depth through the real headless engine: a form with its controls, a table of 100,000
# rows of which only the visible ones exist, a tree, shortcuts and keys, and a switch whose branch
# holds regions directly (rebuilt without the engine refusing a part id). Scenes and accessibility
# trees are pinned as goldens.
require "./engine_helper_spec"

record Person, name : String, city : String, age : Int32

class FormsApp < Wolkit::App
  @name = field("", "Name", Wolkit::Validate.required("Enter a name."))
  @city = field("", "City")
  @form = Wolkit::Form.new
  @notes = signal("")
  @period = signal(0)
  @born = Wolkit::Signal(Time?).new(Time.local(1990, 5, 17))
  @volume = signal(0.5)
  @saved = 0

  getter form, name, city, notes, period, saved

  def view
    window "Forms", width: 480, height: 640 do
      column gap: :md, pad: :lg do
        shortcut("cmd+s") { save }
        form_field field: @name, form: @form do
          text_field @name.value, label: "Name", placeholder: "Ada Lovelace"
        end
        form_field field: @city, form: @form, hint: "Where you live." do
          combo value: @city.value, options: ["Oslo", "Osaka", "Lisbon"], label: "City"
        end
        text_area value: @notes, label: "Notes", lines: 3
        segmented options: ["Day", "Week", "Month"], selected: @period, label: "View"
        date_field value: @born, label: "Born", today: -> { Time.local(2026, 11, 3) }
        slider_field value: @volume, label: "Volume", format: ->(v : Float64) { "#{(v * 100).round.to_i}%" }
        button "Save", primary: true, on_click: -> { save }
        form_status form: @form
      end
    end
  end

  def save : Nil
    @form.submit do
      @saved += 1
      @form.status.value = "Saved."
    end
  end
end

PEOPLE = Array.new(100_000) { |i| Person.new("Person #{i}", %w(Oslo Osaka Lisbon)[i % 3], 20 + i % 60) }

class TableApp < Wolkit::App
  @people = signal(PEOPLE, of: Array(Person))
  @chosen = signal(-1)
  getter chosen
  getter activated = [] of String

  COLUMNS = [
    Wolkit::Kit::Column(Person).new("Name", ->(p : Person) { p.name }, sort: ->(p : Person) { p.name }),
    Wolkit::Kit::Column(Person).new("City", ->(p : Person) { p.city }, width: 120, sort: ->(p : Person) { p.city }),
    Wolkit::Kit::Column(Person).new("Age", ->(p : Person) { p.age.to_s }, width: 56),
  ]

  def view
    window "Table", width: 480, height: 420 do
      column pad: :md do
        table rows: @people, columns: COLUMNS, label: "People", height: 320, selected: @chosen,
          on_activate: ->(p : Person) { @activated << p.name; nil }
      end
    end
  end
end

alias N = Wolkit::Kit::TreeNode(String)

class TreeApp < Wolkit::App
  @roots = signal([N.new("Documents", [N.new("Letters", [N.new("bank.txt")]), N.new("notes.txt")]), N.new("Music")], of: Array(N))

  def view
    window "Tree", width: 360, height: 360 do
      column pad: :md do
        tree roots: @roots, title: ->(s : String) { s }, label: "Files", height: 280
      end
    end
  end
end

class TabsOfRegions < Wolkit::App
  @tab = signal(0)
  @items = signal(["one", "two", "three"], of: Array(String))
  getter tab

  def view
    window "Tabs", width: 320, height: 240 do
      column do
        switch(@tab) do |tab|
          each @items, key: ->(item : String) { item } do |item|
            text "#{tab}: #{item}"
          end
          show(Wolkit::Const(Bool).new(true)) { text "#{tab}: end" }
        end
      end
    end
  end
end

def rows_in(a11y : String) : Int32
  a11y.lines.count(&.includes?("role=listitem"))
end

describe "forms through the engine" do
  it "lays out the form and its controls" do
    Wolkit::Test.run(FormsApp) do |t|
      golden("engine/forms_mounted.scene", t.scene)
      golden("engine/forms_mounted.a11y", t.a11y_snapshot)
    end
  end

  it "shows the error when saving an empty form, and saves once it is valid" do
    Wolkit::Test.run(FormsApp) do |t|
      app = t.app.as(FormsApp)
      t.click "Save"
      app.saved.should eq 0
      t.a11y_snapshot.should contain(%(name="Enter a name."))
      t.type "Ada", into: "Name"
      t.a11y_snapshot.should_not contain(%(name="Enter a name."))
      t.press 0x73_u32, Wolkit::KeyPress::CTRL
      app.saved.should eq 1
      t.a11y_snapshot.should contain(%(name="Saved."))
    end
  end

  it "suggests cities, breaks lines in the text area and steps the segmented control" do
    Wolkit::Test.run(FormsApp) do |t|
      app = t.app.as(FormsApp)
      t.type "os", into: "City"
      t.a11y_snapshot.should contain(%(name="Osaka"))
      # The headless engine hit-tests in tree order (no layers), so the menu is used with the keys.
      t.press 0xff54_u32
      t.press 0xff54_u32
      t.press 0xff0d_u32
      app.city.value.value.should eq "Osaka"
      t.a11y_snapshot.should_not contain("role=menuitem")
      t.type "first", into: "Notes"
      t.press 0xff0d_u32
      app.notes.value.should eq "first\n"
      t.click "Week"
      app.period.value.should eq 1
      t.press 0xff53_u32
      app.period.value.should eq 2
      golden("engine/forms_filled.a11y", t.a11y_snapshot)
    end
  end
end

describe "the virtual table" do
  it "builds only the rows on screen out of 100,000" do
    Wolkit::Test.run(TableApp) do |t|
      a11y = t.a11y_snapshot
      rows_in(a11y).should be <= 16
      a11y.should contain(%(name="Person 0, Oslo, 20"))
      a11y.should_not contain("Person 50,")
      golden("engine/table_mounted.a11y", a11y)
    end
  end

  it "follows the scroll position, moves the selection with the keys and sorts" do
    Wolkit::Test.run(TableApp) do |t|
      app = t.app.as(TableApp)
      x, y, w, h = t.window.node_rect(t.find("People"))
      t.window.inject_pointer(LibWolkit::WOLKIT_POINTER_MOVE.to_u32, x + w / 2, y + h / 2)
      t.window.inject_pointer(LibWolkit::WOLKIT_POINTER_SCROLL.to_u32, 0.0_f32, 40_000.0_f32)
      t.settle
      a11y = t.a11y_snapshot
      a11y.should contain(%(name="Person 1000, Osaka, 60"))
      a11y.should_not contain(%(name="Person 0, Oslo, 20"))
      rows_in(a11y).should be <= 16
      t.click "Person 1001, Lisbon, 61"
      app.chosen.value.should eq 1001
      t.press 0xff54_u32
      app.chosen.value.should eq 1002
      t.press 0xff0d_u32
      app.activated.should eq ["Person 1002"]
      t.click "Sort by City"
      app.chosen.value.should eq -1
      t.scene.should contain(%("City ↑"))
      t.a11y_snapshot.lines.select(&.includes?("role=listitem")).all?(&.includes?(", Lisbon, ")).should be_true
    end
  end
end

describe "the virtual tree" do
  it "opens and closes nodes with clicks and keys" do
    Wolkit::Test.run(TreeApp) do |t|
      t.a11y_snapshot.should_not contain(%(name="Letters"))
      t.click "Open Documents"
      t.a11y_snapshot.should contain(%(name="Letters"))
      t.click "Letters"
      t.press 0xff53_u32
      t.a11y_snapshot.should contain(%(name="bank.txt"))
      golden("engine/tree_open.a11y", t.a11y_snapshot)
      t.click "Close Documents"
      t.a11y_snapshot.should_not contain(%(name="bank.txt"))
    end
  end
end

describe "a switch whose branch is regions" do
  it "rebuilds again and again without leaving parts behind" do
    errors = [] of String
    Wolkit.on_error { |ex| errors << ex.message.to_s }
    Wolkit::Test.run(TabsOfRegions) do |t|
      app = t.app.as(TabsOfRegions)
      1.upto(4) do |tab|
        app.tab.value = tab
        t.settle
        t.scene.should contain(%("#{tab}: two"))
        t.scene.should_not contain(%("#{tab - 1}: two"))
      end
      t.scene.lines.count(&.includes?(": ")).should eq 4
    end
    errors.should be_empty
  end
end
