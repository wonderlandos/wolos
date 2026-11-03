# SPDX-License-Identifier: BSD-2-Clause
# Golden patch streams, read back through the engine: the bytes the Crystal encoder writes are applied
# by the real (headless) engine and compared with its own dump of the scene and of the accessibility
# tree (`wolkit_window_dump`). This is the test that the encoder and the engine agree, not only that
# the encoder is self-consistent.
require "./engine_helper_spec"
require "../dsl_app_spec"

describe "golden patch streams through the engine" do
  it "mounts the Todos example" do
    Wolkit::Test.run(SpecTodos) do |t|
      golden("engine/todos_mounted.scene", t.scene)
      golden("engine/todos_mounted.a11y", t.a11y_snapshot)
    end
  end

  it "adds a todo by typing and clicking" do
    Wolkit::Test.run(SpecTodos) do |t|
      t.type "milk", into: "What needs doing?"
      t.click "Add"
      t.text("milk").should eq "milk"
      t.text("1 left").should eq "1 left"
      golden("engine/todos_added.scene", t.scene)
    end
  end

  it "removes it again and the ids are reused" do
    Wolkit::Test.run(SpecTodos) do |t|
      t.type "milk", into: "What needs doing?"
      t.click "Add"
      t.click "Delete milk"
      t.text("0 left").should eq "0 left"
      t.scene.should_not contain("milk")
      golden("engine/todos_removed.scene", t.scene)
    end
  end

  it "renders the same pixels twice" do
    Wolkit::Test.run(SpecTodos) do |t|
      first = t.screenshot
      second = t.screenshot
      first.size.should be > 100
      first.should eq second
      String.new(first[1, 3]).should eq "PNG"
    end
  end
end
