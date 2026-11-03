# SPDX-License-Identifier: BSD-2-Clause
# Windows and Kit parts against the fake Kit host (spec/fixtures/kit.json): after the first show
# the host clicks "OK", types "Ada" into "Name", turns "Wi-Fi" off, slides to 75, clicks "Raise"
# (whose block raises), clicks "OK" again and closes the window.
require "./spec_helper"

module KitFixture
  # What the blocks saw.
  class Seen
    property clicks = 0
    property names = [] of String
    property wifi = [] of Bool
    property volume = [] of Float64
    property closed = 0
    # When true the "Raise" button's block raises (only the example that checks on_error).
    property? loud = false
  end

  record Parts, window : Wol::Window, stack : Wol::Stack, label : Wol::Label, ok : Wol::Button,
    name : Wol::TextField, wifi : Wol::Toggle, volume : Wol::Slider, seen : Seen

  # The window the fixture expects: a stack of every part kind.
  def self.build(ctx : Wol::Context) : Parts
    seen = Seen.new
    window = Wol::Window.new(ctx, "Kit spec")
    stack = Wol::Stack.new(ctx, 8)
    label = Wol::Label.new(ctx, "Hello")
    ok = Wol::Button.new(ctx, "OK") { seen.clicks += 1 }
    boom = Wol::Button.new(ctx, "Raise") { raise "boom from a click" if seen.loud? }
    name = Wol::TextField.new(ctx, "Name") { |field| seen.names << field.text }
    wifi = Wol::Toggle.new(ctx, "Wi-Fi", on: true) { |toggle| seen.wifi << toggle.on? }
    volume = Wol::Slider.new(ctx, 0, 100, 50) { |slider| seen.volume << slider.value }
    row = Wol::Row.new(ctx, 4)
    row << Wol::Image.new(ctx, "weather-clear-symbolic", 24) << Wol::Spacer.new(ctx) << Wol::Label.new(ctx, "Sunny")
    list = Wol::List.new(ctx)
    list << Wol::Label.new(ctx, "One")
    stack << label << ok << name << wifi << volume << boom << row << list
    window.content = stack
    window.on_close { seen.closed += 1 }
    Parts.new(window, stack, label, ok, name, wifi, volume, seen)
  end
end

describe Wol::Window do
  it "sends the whole tree to the Kit host on show" do
    Wol::Spec.with_fake("kit") do |ctx, fake|
      parts = KitFixture.build(ctx)
      parts.window.show
      Wol::Spec.run_until(ctx) { fake.shown.size >= 2 }.should be_true
      fake.shown[0]["op"].should eq "hello"
      fake.shown[0]["app"].should eq "org.example.kit"
      show = fake.shown[1]
      show["op"].should eq "show"
      show["title"].should eq "Kit spec"
      show["content"]["type"].should eq "stack"
      show["content"]["spacing"].should eq 8
      kids = show["content"]["children"].as_a
      kids.map(&.["type"].as_s).should eq %w(label button text-field toggle slider button row list)
      kids[0]["text"].should eq "Hello"
      kids[1]["label"].should eq "OK"
      kids[2]["placeholder"].should eq "Name"
      kids[3]["on"].as_bool.should be_true
      kids[3]["label"].should eq "Wi-Fi"
      kids[4]["value"].as_f.should eq 50.0
      kids[4]["max"].as_f.should eq 100.0
      row = kids[6]
      row["spacing"].should eq 4
      row["children"].as_a.map(&.["type"].as_s).should eq %w(image spacer label)
      row["children"][0]["source"].should eq "weather-clear-symbolic"
      row["children"][0]["size"].should eq 24
      kids[7]["children"][0]["text"].should eq "One"
    end
  end

  it "delivers clicks, edits, toggles, slides and the close to the blocks" do
    errors = Wol::Spec.capture_errors do
      Wol::Spec.with_fake("kit") do |ctx, fake|
        parts = KitFixture.build(ctx)
        parts.seen.loud = true
        parts.window.show
        Wol::Spec.run_until(ctx) { parts.seen.closed > 0 }.should be_true
        seen = parts.seen
        seen.clicks.should eq 2
        seen.names.should eq ["Ada"]
        seen.wifi.should eq [false]
        seen.volume.should eq [75.0]
        parts.name.text.should eq "Ada"
        parts.wifi.on?.should be_false
        parts.volume.value.should eq 75.0
        fake.problems.should be_empty
      end
    end
    # The "Raise" block raised inside dispatch; the loop went on and the second click arrived.
    errors.size.should eq 1
    errors[0].message.should eq "boom from a click"
  end

  it "keeps callback parts alive through garbage collection" do
    Wol::Spec.with_fake("kit") do |ctx, _fake|
      parts = KitFixture.build(ctx)
      # The five parts with a block plus the window's close handler.
      ctx.rooted_count.should eq 6
      parts.window.show
      ctx.rooted_count.should eq 6
      3.times { GC.collect }
      Wol::Spec.run_until(ctx) { parts.seen.closed > 0 }.should be_true
      parts.seen.clicks.should eq 2
    end
  end

  it "keeps a placed part with a callback rooted after close, and drops its clicks" do
    Wol::Spec.with_fake("kit") do |ctx, _fake|
      parts = KitFixture.build(ctx)
      parts.ok.close
      parts.ok.closed?.should be_true
      # libwol's stack still holds the button and may call the trampoline: it stays rooted.
      ctx.rooted_count.should eq 6
      parts.window.show
      3.times { GC.collect }
      Wol::Spec.run_until(ctx) { parts.seen.closed > 0 }.should be_true
      # Both clicks on "OK" reached a live object with no block; the other parts still work.
      parts.seen.clicks.should eq 0
      parts.seen.names.should eq ["Ada"]
    end
  end

  it "unroots a part that is in no container when it is closed" do
    Wol::Spec.with_fake("kit") do |ctx, _fake|
      button = Wol::Button.new(ctx, "Alone") { }
      ctx.rooted_count.should eq 1
      button.close
      ctx.rooted_count.should eq 0
      Wol::Label.new(ctx, "No block")
      Wol::TextField.new(ctx, "No block")
      Wol::Toggle.new(ctx, "No block")
      Wol::Slider.new(ctx, 0, 1, 0)
      ctx.rooted_count.should eq 0
    end
  end

  it "redraws a shown window when a part changes" do
    Wol::Spec.with_fake("kit") do |ctx, fake|
      parts = KitFixture.build(ctx)
      parts.window.show
      parts.label.text = "Cloudy"
      Wol::Spec.run_until(ctx) { fake.shown.size >= 3 }.should be_true
      fake.shown[2]["content"]["children"][0]["text"].should eq "Cloudy"
    end
  end

  it "reads and writes part state before the window is shown" do
    Wol::Spec.with_fake("kit") do |ctx, fake|
      parts = KitFixture.build(ctx)
      parts.label.text.should eq "Hello"
      (parts.label.text = "Bye").should eq "Bye"
      parts.label.text.should eq "Bye"
      parts.ok.text.should eq "OK"
      parts.name.text.should eq ""
      parts.name.text = "Grace"
      parts.name.text.should eq "Grace"
      parts.wifi.on?.should be_true
      parts.wifi.on = false
      parts.wifi.on?.should be_false
      parts.volume.value.should eq 50.0
      parts.volume.value = 25
      parts.volume.value.should eq 25.0
      parts.stack.children.size.should eq 8
      parts.stack.placed?.should be_true
      parts.window.content.should be(parts.stack)
      # Nothing was shown, so nothing went to the Kit host.
      fake.shown.should be_empty
    end
  end

  it "keeps multibyte text and line breaks intact, and refuses control characters" do
    Wol::Spec.with_fake("kit") do |ctx, _fake|
      label = Wol::Label.new(ctx, "Grüße\n日本")
      label.text.should eq "Grüße\n日本"
      expect_raises(Wol::InvalidArgument, "control characters") { label.text = "bell\u0007" }
      label.text.should eq "Grüße\n日本"
    end
  end

  it "refuses bad arguments with libwol's sentence" do
    Wol::Spec.with_fake("kit") do |ctx, _fake|
      expect_raises(Wol::InvalidArgument, "Spacing is one of") { Wol::Stack.new(ctx, 5) }
      expect_raises(Wol::InvalidArgument, "Spacing is one of") { Wol::Row.new(ctx, -1) }
      expect_raises(Wol::InvalidArgument) { Wol::Image.new(ctx, "/etc/../shadow.png", 24) }
      expect_raises(Wol::InvalidArgument) { Wol::Image.new(ctx, "weather-clear-symbolic", 20) }
      expect_raises(Wol::InvalidArgument) { Wol::Slider.new(ctx, 10, 0, 5) }
      expect_raises(Wol::InvalidArgument, "A button needs a label.") { Wol::Button.new(ctx, "") { } }
      expect_raises(Wol::InvalidArgument, "A window title is one line.") { Wol::Window.new(ctx, "two\nlines") }
      slider = Wol::Slider.new(ctx, 0.0, 1.0, 0.5)
      expect_raises(Wol::InvalidArgument) { slider.value = 2 }
      slider.value.should eq 0.5
      label = Wol::Label.new(ctx, "Once")
      first = Wol::Stack.new(ctx)
      second = Wol::Row.new(ctx)
      first << label
      expect_raises(Wol::InvalidArgument, "This part is already in a stack or a window.") { second << label }
      second.children.should be_empty
      window = Wol::Window.new(ctx, "W")
      expect_raises(Wol::InvalidArgument, "This part is already in a stack or a window.") { window.content = label }
      window.content.should be_nil
    end
  end

  it "raises UsageError on a closed part and allows a second close" do
    Wol::Spec.with_fake("kit") do |ctx, _fake|
      label = Wol::Label.new(ctx, "Gone")
      label.close
      label.closed?.should be_true
      expect_raises(Wol::UsageError) { label.text }
      expect_raises(Wol::UsageError) { Wol::Stack.new(ctx) << label }
      label.close
      window = Wol::Window.new(ctx, "W")
      window.close
      expect_raises(Wol::UsageError) { window.show }
    end
  end

  it "stops calling a removed close handler" do
    Wol::Spec.with_fake("kit") do |ctx, _fake|
      parts = KitFixture.build(ctx)
      parts.window.clear_close_handler
      parts.window.show
      Wol::Spec.run_until(ctx) { parts.seen.clicks >= 2 }.should be_true
      Wol::Spec.run_until(ctx, 300.milliseconds) { false }
      parts.seen.closed.should eq 0
    end
  end

  it "takes the window off screen with hide and back with show" do
    Wol::Spec.with_fake("kit") do |ctx, fake|
      window = Wol::Window.new(ctx, "Hide me")
      window.content = Wol::Label.new(ctx, "Here")
      window.show.hide.show
      Wol::Spec.run_until(ctx) { fake.shown.size >= 4 }.should be_true
      fake.shown[1..].map(&.["op"].as_s).should eq %w(show close show)
      fake.shown[2]["window"].should eq fake.shown[1]["window"]
    end
  end

  it "clears the content with nil" do
    Wol::Spec.with_fake("kit") do |ctx, fake|
      window = Wol::Window.new(ctx, "Empty")
      label = Wol::Label.new(ctx, "Soon gone")
      window.content = label
      window.content = nil
      window.content.should be_nil
      window.show
      Wol::Spec.run_until(ctx) { fake.shown.size >= 2 }.should be_true
      fake.shown[1]["content"]["children"].as_a.should be_empty
      # The part is free again.
      label.placed?.should be_false
      Wol::Stack.new(ctx) << label
      label.placed?.should be_true
    end
  end

  it "keeps a shown window alive with no reference to it" do
    Wol::Spec.with_fake("kit") do |ctx, _fake|
      ctx.rooted_count.should eq 0
      Wol::Window.new(ctx, "Orphan").show
      ctx.rooted_count.should eq 1
    end
  end
end
