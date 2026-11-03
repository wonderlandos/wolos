# SPDX-License-Identifier: BSD-2-Clause
# Specs for the DSL: the section 3.1 Todos example compiles, and what it emits is pinned as a
# golden patch stream (decoded to one line per op).
require "./spec_helper"
require "./dsl_app_spec"

describe "the DSL" do
  after_each { Wolkit::Test.reset }

  it "mounts the Todos example as one patch" do
    recorder = Wolkit::Test.record(SpecTodos.new)
    window = recorder.windows.first
    window.patches.size.should eq 1
    window.shown?.should be_true
    golden("todos_mounted.ops", window.ops.join("\n") + "\n")
  end

  it "adds a row for a new todo and updates only what depends on the change" do
    app = SpecTodos.new
    recorder = Wolkit::Test.record(app)
    window = recorder.windows.first
    Wolkit::Test.type(window, "What needs doing?", "milk")
    # Typing changes only the button's disabled flag; the typed text is not sent back.
    window.last_ops.should eq ["SET_STATE 4 0"]
    Wolkit::Test.click(window, "Add")
    app.items.to_a.should eq ["milk"]
    golden("todos_added.ops", window.last_ops.join("\n") + "\n")
    window.patches.size.should eq 3
  end

  it "frees a row and gives its ids back" do
    app = SpecTodos.new
    recorder = Wolkit::Test.record(app)
    window = recorder.windows.first
    Wolkit::Test.type(window, "What needs doing?", "milk")
    Wolkit::Test.click(window, "Add")
    Wolkit::Test.click(window, "Delete milk")
    app.items.to_a.should be_empty
    window.last_ops.should eq ["FREE 7", "SET_TEXT 6 \"0 left\""]
    # The next row reuses the freed ids with a new generation.
    Wolkit::Test.type(window, "What needs doing?", "eggs")
    Wolkit::Test.click(window, "Add")
    window.last_ops.first(3).should eq ["SET_TEXT 3 \"\"", "SET_STATE 4 16", "CREATE 16777226 box"]
  end
end
