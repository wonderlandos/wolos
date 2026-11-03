# SPDX-License-Identifier: BSD-2-Clause
# Specs for components: typed props, the element a class defines, slots and lifecycle hooks.
require "./spec_helper"

class Counter < Wolkit::Component
  prop start : Int32 = 0
  @n = signal(0)

  def view
    @n.value = @start
    row gap: :sm do
      button "-", label: "Decrease", on_click: -> { @n.value -= 1 }
      text "#{@n}"
      button "+", label: "Increase", on_click: -> { @n.value += 1 }
    end
  end
end

class Panel < Wolkit::Component
  prop title : String

  def view
    column gap: :sm do
      text @title, role: :title
      slot
    end
  end
end

class ComponentApp < Wolkit::App
  def view
    window "Components" do
      column do
        counter start: 5
        panel title: "Notes" do
          text "inside the slot"
        end
      end
    end
  end
end

describe Wolkit::Component do
  after_each { Wolkit::Test.reset }

  it "mounts a component with its props and its slot" do
    recorder = Wolkit::Test.record(ComponentApp.new)
    ops = recorder.windows.first.ops
    ops.should contain("SET_TEXT 4 \"5\"")
    ops.should contain("SET_TEXT 8 \"inside the slot\"")
    golden("components.ops", ops.join("\n") + "\n")
  end
end
