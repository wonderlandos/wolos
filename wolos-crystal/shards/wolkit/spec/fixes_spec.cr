# SPDX-License-Identifier: BSD-2-Clause
# The four defects found while building the first bundled app: paint pairs and borders compile and
# send their values, a screenshot is a PNG every reader accepts, a component may be the only element
# of a row, and a region placed directly in a switch row frees its rows when the switch changes.
require "./spec_helper"
require "../src/wolkit/component"
require "../src/wolkit/test/png"

class Painted < Wolkit::App
  @lift = signal(2.0)

  getter lift

  def view
    window "Paint" do
      column border: {1, :line}, outline: 2, translate: {4, @lift.value}, scale: 1.5 do
        text "inside", border: :none
      end
    end
  end
end

class Badge < Wolkit::Component
  prop word : String

  def view
    text @word
  end
end

class OnlyComponent < Wolkit::App
  @on = signal(false)
  @tab = signal(1)

  getter on, tab

  def view
    window "Only" do
      column do
        show(@on) { badge word: "shown" }
        switch(@tab) { |tab| badge word: "tab #{tab}" }
      end
    end
  end
end

class SwitchOfRegions < Wolkit::App
  @tab = signal(:a)
  @items = signal(["x", "y"], of: Array(String))

  getter tab

  def view
    window "Switch" do
      column do
        switch(@tab) do |tab|
          each @items, key: ->(item : String) { item } do |item|
            text "#{tab} #{item}"
          end
          show(Wolkit::Const(Bool).new(true)) { text "#{tab} footer" }
        end
      end
    end
  end
end

describe "paint pairs and borders" do
  after_each { Wolkit::Test.reset }

  it "sends border, outline, translate and scale" do
    app = Painted.new
    ops = Wolkit::Test.record(app).windows.first.ops
    ops.should contain("SET_PROP 1 border 1.0 token3")
    ops.should contain("SET_PROP 1 outline 2.0 token3")
    ops.should contain("SET_PROP 1 transform-translate 4.0 2.0")
    ops.should contain("SET_PROP 1 transform-scale 1.5 1.5")
    ops.should contain("SET_PROP 2 border 0.0 token3")
  end

  it "converts values known only at run time" do
    Wolkit::Scale.pair(3).should eq({3.0_f32, 3.0_f32})
    Wolkit::Scale.pair({1, 2.5}).should eq({1.0_f32, 2.5_f32})
    Wolkit::Scale.border({2, :accent}).should eq({2.0_f32, 6_u32})
    expect_raises(Wolkit::UsageError, /A border is a width/) { Wolkit::Scale.border(:thick) }
    expect_raises(Wolkit::UsageError, /theme tokens/) { Wolkit::Scale.border({1, :purple}) }
    expect_raises(Wolkit::UsageError, /A pair is one number/) { Wolkit::Scale.pair("wide") }
  end
end

describe "screenshots" do
  it "writes chunks with the CRC a PNG reader checks" do
    png = Wolkit::Test::PNG.encode(Bytes.new(2 * 2 * 4, 0x80_u8), 2, 2)
    at = 8
    chunks = [] of String
    while at < png.size
      length = IO::ByteFormat::BigEndian.decode(UInt32, png[at, 4]).to_i
      body = png[at + 4, length + 4]
      stored = IO::ByteFormat::BigEndian.decode(UInt32, png[at + 8 + length, 4])
      Digest::CRC32.checksum(body).should eq stored
      chunks << String.new(body[0, 4])
      at += 12 + length
    end
    chunks.should eq ["IHDR", "IDAT", "IEND"]
    # The IEND chunk of every PNG ends with these bytes.
    png[-4, 4].should eq Bytes[0xae, 0x42, 0x60, 0x82]
  end
end

describe "rows made of one component" do
  after_each { Wolkit::Test.reset }

  it "mounts a component as the only element of a show or switch branch" do
    app = OnlyComponent.new
    window = Wolkit::Test.record(app).windows.first
    window.ops.should contain("SET_TEXT 4 \"tab 1\"")
    app.on.value = true
    Wolkit::Test.settle
    window.last_ops.any?(&.includes?("\"shown\"")).should be_true
    app.tab.value = 2
    Wolkit::Test.settle
    window.last_ops.any?(&.starts_with?("FREE ")).should be_true
    window.last_ops.any?(&.includes?("\"tab 2\"")).should be_true
  end
end

describe "a switch whose branch holds regions directly" do
  after_each { Wolkit::Test.reset }

  it "frees the rows of the inner regions when the branch is rebuilt" do
    app = SwitchOfRegions.new
    window = Wolkit::Test.record(app).windows.first
    mount = Wolkit::Runtime.windows.first.mount
    live = mount.live
    app.tab.value = :b
    Wolkit::Test.settle
    ops = window.last_ops
    # The two keyed rows, the footer and both markers go: five FREEs, nothing left behind.
    ops.count(&.starts_with?("FREE ")).should eq 5
    ops.any?(&.includes?("\"b footer\"")).should be_true
    mount.live.should eq live
  end
end
