# SPDX-License-Identifier: BSD-2-Clause
# The kit's depth without an engine: keys and shortcuts, forms and their rules, the manual timer
# clock with debounce, throttle and interval, the arithmetic of the virtual table and tree, the
# helpers of the form controls, and the file panel with a runner of the spec's own.
require "./spec_helper"
require "../src/wolkit/kit"

class KeyedApp < Wolkit::App
  getter keys = [] of String
  getter saved = 0
  getter focus = [] of Bool

  def view
    window "Keys" do
      on_key { |key| @keys << key.name if key.pressed? }
      shortcut("Ctrl+S") { @saved += 1 }
      shortcut("cmd+shift+s") { @saved += 10 }
      column on_key: ->(key : Wolkit::KeyPress) { @keys << "column #{key.name}" }, on_focus: ->(focused : Bool) { @focus << focused } do
        text "body"
      end
    end
  end
end

class PanelApp < Wolkit::App
  @open = signal(false)
  getter open
  getter names = [] of String

  def view
    window "Panel" do
      column do
        file_panel open: @open, title: "Open a picture", on_file: ->(file : Wolkit::Kit::PickedFile) { @names << file.name; nil }
      end
    end
  end
end

def key_payload(keysym : UInt32, pressed = true, mods = 0_u32) : Bytes
  io = IO::Memory.new
  [keysym, pressed ? 1_u32 : 0_u32, mods, 0_u32].each { |w| io.write_bytes(w, IO::ByteFormat::LittleEndian) }
  io.to_slice
end

describe "keys" do
  after_each { Wolkit::Test.reset }

  it "names keys the way people say them" do
    Wolkit::KeyPress.new(0xff1b_u32).name.should eq "escape"
    Wolkit::KeyPress.new(0x41_u32).name.should eq "a"
    Wolkit::KeyPress.new(0xffc2_u32).name.should eq "f5"
    Wolkit::KeyPress.new(0xffe1_u32).name.should eq ""
    key = Wolkit::KeyPress.new(0x73_u32, true, Wolkit::KeyPress::CTRL)
    key.ctrl?.should be_true
    key.is?("s", ctrl: true).should be_true
    key.is?("s").should be_false
  end

  it "writes shortcuts in the engine's form and refuses ones that never arrive" do
    Wolkit::Keys.normalise("Ctrl+Shift+S").should eq "cmd+shift+s"
    Wolkit::Keys.normalise("alt+super+1").should eq "cmd+alt+1"
    Wolkit::Keys.normalise("f5").should eq "f5"
    expect_raises(Wolkit::UsageError, /needs cmd or alt/) { Wolkit::Keys.normalise("shift+a") }
    expect_raises(Wolkit::UsageError, /modifiers are cmd, shift and alt/) { Wolkit::Keys.normalise("hyper+a") }
    expect_raises(Wolkit::UsageError, /ends with a letter/) { Wolkit::Keys.normalise("cmd+enter") }
  end

  it "delivers keys up through the parts, runs shortcuts and reports focus" do
    app = KeyedApp.new
    window = Wolkit::Test.record(app).windows.first
    window.ops.should contain("BIND_EVENT 0 1000")
    column = window.ops.find!(&.starts_with?("CREATE ")).split[1].to_u32
    key = ->(node : UInt32, sym : UInt32, mods : UInt32) do
      Wolkit::Runtime.dispatch(Wolkit::Test.event(Wolkit::Wire::Event::KEY, window.id, node, payload: key_payload(sym, true, mods)))
    end
    key.call(0_u32, 0xff1b_u32, 0_u32)
    key.call(column, 0x6a_u32, 0_u32)
    key.call(column, 0x73_u32, Wolkit::KeyPress::CTRL)
    key.call(0_u32, 0x53_u32, Wolkit::KeyPress::CTRL | Wolkit::KeyPress::SHIFT)
    key.call(0_u32, 0x71_u32, Wolkit::KeyPress::CTRL)
    Wolkit::Runtime.dispatch(Wolkit::Test.event(Wolkit::Wire::Event::FOCUS, window.id, column, payload: Bytes[0, 0, 0, 0]))
    app.keys.should eq ["escape", "column j", "j", "column s", "s", "s", "q"]
    app.saved.should eq 11
    app.focus.should eq [false]
  end

  it "names shortcuts the way the engine does" do
    Wolkit::Keys.shortcut_name(Wolkit::KeyPress.new(0x53_u32, true, Wolkit::KeyPress::SUPER | Wolkit::KeyPress::SHIFT)).should eq "cmd+shift+s"
    Wolkit::Keys.shortcut_name(Wolkit::KeyPress.new(0xffc2_u32)).should eq "f5"
    Wolkit::Keys.shortcut_name(Wolkit::KeyPress.new(0x61_u32, true, Wolkit::KeyPress::SHIFT)).should be_nil
    Wolkit::Keys.shortcut_name(Wolkit::KeyPress.new(0x31_u32, true, Wolkit::KeyPress::ALT)).should eq "alt+1"
  end
end

describe "forms" do
  it "checks a field and shows its error only once it was touched" do
    form = Wolkit::Form.new
    name = form.field("", "Name", Wolkit::Validate.required("Enter a name."))
    age = form.field("", "Age", Wolkit::Validate.integer(0, 150))
    name.error.value.should eq "Enter a name."
    name.shown.value.should be_nil
    form.valid?.should be_false
    name.touch
    name.shown.value.should eq "Enter a name."
    name.value.value = "Ada"
    name.shown.value.should be_nil
    age.value.value = "200"
    age.error.value.should eq "Enter 150 or less."
    age.value.value = "x"
    age.error.value.should eq "Enter a whole number."
    age.value.value = ""
    form.valid?.should be_true
  end

  it "submits only a valid form and shows every error when it is not" do
    form = Wolkit::Form.new
    mail = form.field("", "E-mail", Wolkit::Validate.required, Wolkit::Validate.email)
    code = form.field("", "Code") { |v| "Six digits." unless v.size == 6 }
    ran = 0
    form.submit { ran += 1 }.should be_false
    ran.should eq 0
    mail.shown.value.should eq "This can't be empty."
    code.shown.value.should eq "Six digits."
    mail.value.value = "ada@example"
    mail.shown.value.should eq "Enter an address like name@example.com."
    mail.value.value = "ada@example.com"
    code.value.value = "123456"
    form.submit { ran += 1 }.should be_true
    ran.should eq 1
    form.reset
    mail.value.value.should eq ""
    mail.shown.value.should be_nil
    form.submitted.value.should be_false
  end

  it "has rules for lengths, numbers and choices" do
    Wolkit::Validate.length(2, 4).call("a").should eq "Use at least 2 characters."
    Wolkit::Validate.length(2, 4).call("abcde").should eq "Use at most 4 characters."
    Wolkit::Validate.number(0.0, 1.0).call("0.5").should be_nil
    Wolkit::Validate.number(0.0, 1.0).call("1.5").should eq "Enter 1.0 or less."
    Wolkit::Validate.chosen(Time).call(nil).should eq "Choose one."
    Wolkit::Validate.matches(/\A\d+\z/, "Digits only.").call("12a").should eq "Digits only."
  end
end

describe "timers on the manual clock" do
  before_each { Wolkit::Timer.manual! }
  after_each do
    Wolkit::Timer.real!
    Wolkit::Scope.reset_root
  end

  it "runs every and after when the clock moves, and not after the scope ends" do
    scope = Wolkit::Scope.new(nil)
    ticks = 0
    once = 0
    scope.run do
      Wolkit.every(100.milliseconds) { ticks += 1 }
      Wolkit.after(250.milliseconds) { once += 1 }
    end
    Wolkit::Timer.advance(99.milliseconds)
    ticks.should eq 0
    Wolkit::Timer.advance(301.milliseconds)
    ticks.should eq 4
    once.should eq 1
    scope.dispose
    Wolkit::Timer.advance(1.second)
    ticks.should eq 4
    Wolkit::Timer.pending.should eq 0
  end

  it "debounces: the value arrives once the source has been still" do
    query = Wolkit::Signal(String).new("")
    quiet = Wolkit.debounce(query, 300.milliseconds)
    seen = [] of String
    Wolkit.effect { seen << quiet.value }
    query.value = "m"
    Wolkit::Timer.advance(100.milliseconds)
    query.value = "mi"
    Wolkit::Timer.advance(100.milliseconds)
    query.value = "milk"
    Wolkit::Timer.advance(299.milliseconds)
    seen.should eq [""]
    Wolkit::Timer.advance(1.millisecond)
    seen.should eq ["", "milk"]
    Wolkit::Timer.pending.should eq 0
  end

  it "throttles: the first change at once, then the latest once per span" do
    position = Wolkit::Signal(Int32).new(0)
    slow = Wolkit.throttle(position, 100.milliseconds)
    seen = [] of Int32
    Wolkit.effect { seen << slow.value }
    position.value = 1
    seen.should eq [0, 1]
    position.value = 2
    position.value = 3
    Wolkit::Timer.advance(100.milliseconds)
    seen.should eq [0, 1, 3]
    Wolkit::Timer.advance(100.milliseconds)
    Wolkit::Timer.pending.should eq 0
    position.value = 4
    seen.should eq [0, 1, 3, 4]
  end

  it "counts with interval" do
    ticks = Wolkit.interval(1.second)
    Wolkit::Timer.advance(3.5.seconds)
    ticks.value.should eq 3
  end
end

describe "the virtual table and tree" do
  it "works out the rows on screen" do
    Wolkit::Kit.visible_rows(100_000, 0.0, 320.0, 40, 4).should eq 0...12
    Wolkit::Kit.visible_rows(100_000, 4000.0, 320.0, 40, 4).should eq 96...112
    Wolkit::Kit.visible_rows(5, 0.0, 320.0, 40, 4).should eq 0...5
    Wolkit::Kit.visible_rows(0, 0.0, 320.0, 40, 4).should eq 0...0
  end

  it "lists the open nodes of a tree" do
    leaf = Wolkit::Kit::TreeNode(String).new("notes.txt")
    docs = Wolkit::Kit::TreeNode(String).new("Documents", [leaf])
    roots = Wolkit::Signal(Array(Wolkit::Kit::TreeNode(String))).new([docs, Wolkit::Kit::TreeNode(String).new("Music")])
    tree = Wolkit::Kit::Tree(String).new(roots, ->(s : String) { s }, "Files")
    tree.lines.value.map(&.path).should eq ["Documents", "Music"]
    tree.toggle("Documents")
    tree.lines.value.map { |l| {l.path, l.depth} }.should eq [{"Documents", 0}, {"Documents/notes.txt", 1}, {"Music", 0}]
    tree.selected.value = "Documents/notes.txt"
    tree.move(Wolkit::KeyPress.new(0xff51_u32))
    tree.selected.value.should eq "Documents"
    tree.move(Wolkit::KeyPress.new(0xff51_u32))
    tree.lines.value.size.should eq 2
  end

  it "sorts a table by a column and turns the order round" do
    rows = Wolkit::Signal(Array(String)).new(["pear", "apple", "fig"])
    columns = [Wolkit::Kit::Column(String).new("Fruit", ->(s : String) { s }, sort: ->(s : String) { s })]
    table = Wolkit::Kit::Table(String).new(rows, columns, "Fruit")
    table.shown.value.should eq ["pear", "apple", "fig"]
    table.sort(0)
    table.shown.value.should eq ["apple", "fig", "pear"]
    table.sort(0)
    table.shown.value.should eq ["pear", "fig", "apple"]
    table.move(Wolkit::KeyPress.new(0xff54_u32))
    table.selected.value.should eq 0
    table.move(Wolkit::KeyPress.new(0xff57_u32))
    table.selected.value.should eq 2
  end
end

describe "the form controls' helpers" do
  it "suggests prefix matches first" do
    options = ["Oslo", "Lisbon", "Bologna", "Osaka", "oslo"]
    Wolkit::Kit::Combo.matches(options, "os", 8).should eq ["Oslo", "Osaka", "oslo"]
    Wolkit::Kit::Combo.matches(options, "lo", 8).should eq ["Oslo", "Bologna", "oslo"]
    Wolkit::Kit::Combo.matches(options, "", 8).should be_empty
  end

  it "reads and steps dates" do
    Wolkit::Kit::DateField.parse("2026-11-03").try(&.day).should eq 3
    Wolkit::Kit::DateField.parse("2026-13-03").should be_nil
    Wolkit::Kit::DateField.parse("3 Nov").should be_nil
    value = Wolkit::Signal(Time?).new(nil)
    today = -> { Time.local(2026, 2, 28, 15, 0, 0) }
    Wolkit::Kit::DateField.shift(value, 1, today)
    Wolkit::Kit::DateField.format(value.value).should eq "2026-03-01"
    Wolkit::Kit::DateField.key(value, Wolkit::KeyPress.new(0xff54_u32), today)
    Wolkit::Kit::DateField.format(value.value).should eq "2026-02-28"
  end

  it "moves a segmented choice with the arrows and wraps" do
    chosen = Wolkit::Signal(Int32).new(0)
    Wolkit::Kit::Segmented.step(chosen, 3, Wolkit::KeyPress.new(0xff51_u32))
    chosen.value.should eq 2
    Wolkit::Kit::Segmented.step(chosen, 3, Wolkit::KeyPress.new(0xff53_u32))
    chosen.value.should eq 0
  end
end

describe "the file panel" do
  after_each do
    Wolkit::Kit::FilePanel.runner = nil
    Wolkit::Test.reset
  end

  it "runs the panel while open, hands over the file and shows a refusal" do
    calls = [] of {String?, String}
    answer = nil.as(Proc(Wolkit::Kit::PickedFile?, String?, Nil)?)
    Wolkit::Kit::FilePanel.runner = ->(save_as : String?, title : String, done : Proc(Wolkit::Kit::PickedFile?, String?, Nil)) do
      calls << {save_as, title}
      answer = done
      nil
    end
    app = PanelApp.new
    window = Wolkit::Test.record(app).windows.first
    calls.should be_empty
    app.open.value = true
    Wolkit::Test.settle
    calls.should eq [{nil, "Open a picture"}]
    answer.try &.call(nil, "Allow this app to read files in System Settings.")
    Wolkit::Test.settle
    app.open.value.should be_false
    window.last_ops.any?(&.includes?("Allow this app to read files")).should be_true
    app.open.value = true
    Wolkit::Test.settle
    File.open(__FILE__) do |io|
      answer.try &.call(Wolkit::Kit::PickedFile.new("cat.jpg", io), nil)
    end
    app.names.should eq ["cat.jpg"]
    calls.size.should eq 2
  end
end
