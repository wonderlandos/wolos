# SPDX-License-Identifier: BSD-2-Clause
# Specs for the regions (`each`, `show`, `switch`), context values and the environment: rows follow
# their data, scopes give back what they own, and nothing leaks when a row goes away.
require "./spec_helper"
require "../src/wolkit/component"

class Names < Wolkit::App
  @list = list(String)
  @array = signal([] of String, of: Array(String))
  @show = signal(false)
  @kind = signal(1)
  getter list
  getter array
  getter show
  getter kind
  getter fired = 0

  def view
    window "Names" do
      column do
        each @list do |name|
          row do
            text name
            every(5.milliseconds) { @fired += 1 }
          end
        end
        each @array, key: ->(name : String) { name } do |name|
          text "keyed #{name}"
        end
        show(@show) { text "shown" }
        switch(@kind) { |kind| text "kind #{kind}" }
      end
    end
  end
end

class Theme
  getter name : String

  def initialize(@name : String)
  end
end

class Reader < Wolkit::Component
  def view
    text inject(Theme).name
  end
end

class Providing < Wolkit::App
  def view
    window "Context" do
      column do
        provide Theme.new("midnight")
        reader
      end
    end
  end
end

describe "regions" do
  after_each { Wolkit::Test.reset }

  it "keeps rows in step with a list through insert, move and reset" do
    app = Names.new
    recorder = Wolkit::Test.record(app)
    window = recorder.windows.first
    mount = Wolkit::Runtime.windows.first.mount
    base = mount.live
    app.list << "a" << "b" << "c"
    Wolkit::Test.settle
    app.list.move(0, 2)
    Wolkit::Test.settle
    window.last_ops.count(&.starts_with?("MOVE ")).should eq 1
    app.list.replace(["x", "y"])
    Wolkit::Test.settle
    window.last_ops.count(&.starts_with?("FREE ")).should eq 3
    window.ops.count(&.includes?("SET_TEXT")).should be >= 5
    app.list.clear
    Wolkit::Test.settle
    mount.live.should eq base
  end

  it "matches rows by key when an array is replaced" do
    app = Names.new
    recorder = Wolkit::Test.record(app)
    window = recorder.windows.first
    app.array.value = ["one", "two", "three"]
    Wolkit::Test.settle
    created = window.last_ops.count(&.starts_with?("CREATE "))
    # Reorder and drop one: the rows that stay are moved, not rebuilt.
    app.array.value = ["three", "one"]
    Wolkit::Test.settle
    ops = window.last_ops
    ops.count(&.starts_with?("CREATE ")).should eq 0
    ops.count(&.starts_with?("FREE ")).should eq 1
    ops.any?(&.starts_with?("MOVE ")).should be_true
    created.should eq 3
    # Adding one builds only that row.
    app.array.value = ["three", "one", "four"]
    Wolkit::Test.settle
    window.last_ops.count(&.starts_with?("CREATE ")).should eq 1
    window.last_ops.any?(&.includes?("\"keyed four\"")).should be_true
  end

  it "mounts and frees a show region and rebuilds a switch region" do
    app = Names.new
    recorder = Wolkit::Test.record(app)
    window = recorder.windows.first
    app.show.value = true
    Wolkit::Test.settle
    window.last_ops.any?(&.includes?("\"shown\"")).should be_true
    app.show.value = false
    Wolkit::Test.settle
    window.last_ops.first.should start_with("FREE ")
    app.kind.value = 2
    Wolkit::Test.settle
    window.last_ops.any?(&.includes?("\"kind 2\"")).should be_true
    window.last_ops.any?(&.starts_with?("FREE ")).should be_true
  end

  it "stops a row's timer when the row goes away" do
    app = Names.new
    Wolkit::Test.record(app)
    app.list << "a"
    Wolkit::Test.settle
    sleep 40.milliseconds
    app.fired.should be > 0
    app.list.clear
    Wolkit::Test.settle
    after_removal = app.fired
    sleep 40.milliseconds
    app.fired.should eq after_removal
  end
end

describe "context and environment" do
  after_each { Wolkit::Test.reset }

  it "injects what an enclosing view provided" do
    recorder = Wolkit::Test.record(Providing.new)
    recorder.windows.first.ops.should contain("SET_TEXT 2 \"midnight\"")
  end

  it "says what is missing when nothing provides a value" do
    expect_raises(Wolkit::MissingContext, /Nothing provides a Theme/) { Wolkit.inject(Theme) }
  end

  it "follows the theme the engine reports" do
    seen = [] of Bool
    scope = Wolkit::Scope.new(nil)
    scope.run { Wolkit.effect { seen << Wolkit::Env.dark.value } }
    Wolkit::Env.apply_theme(true, 0x112233_u32, Wolkit::Env::REDUCE_MOTION)
    Wolkit::Env.reduce_motion.value.should be_true
    Wolkit::Env.accent.value.should eq 0x112233_u32
    Wolkit::Env.apply_theme(false, 0x7c5cff_u32, 0_u32)
    seen.should eq [false, true, false]
    scope.dispose
  end
end
