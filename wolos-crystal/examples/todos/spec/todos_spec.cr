# SPDX-License-Identifier: BSD-2-Clause
# Todos in a headless engine: type, add, delete, and a screenshot that is the same on every machine.
require "spec"
require "../../../shards/wolkit/src/wolkit/test"
require "../src/todos"

describe Todos do
  it "adds and removes a todo" do
    Wolkit::Test.run(Todos) do |t|
      t.text("0 left").should eq "0 left"
      t.type "milk", into: "What needs doing?"
      t.click "Add"
      t.text("milk").should eq "milk"
      t.text("1 left").should eq "1 left"
      t.type "eggs", into: "What needs doing?"
      t.click "Add"
      t.text("2 left").should eq "2 left"
      t.click "Delete milk"
      t.text("1 left").should eq "1 left"
      t.scene.should_not contain("milk")
      t.scene.should contain("eggs")
    end
  end

  it "does not add an empty todo" do
    Wolkit::Test.run(Todos) do |t|
      t.click "Add"
      t.text("0 left").should eq "0 left"
    end
  end

  it "renders a window" do
    Wolkit::Test.run(Todos) do |t|
      png = t.screenshot
      String.new(png[1, 3]).should eq "PNG"
    end
  end
end
