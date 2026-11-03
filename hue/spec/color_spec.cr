# SPDX-License-Identifier: GPL-2.0-or-later
require "./spec_helper"

include Hue

describe Hue::Color do
  it "converts sRGB to OKLab with the reference matrices" do
    l = Color.hex_to_lab("#ff0000")
    l.l.should be_close(0.627955, 1e-5)
    l.a.should be_close(0.224863, 1e-5)
    l.b.should be_close(0.125846, 1e-5)
    w = Color.hex_to_lab("#ffffff")
    w.l.should be_close(1.0, 1e-6)
    w.a.abs.should be < 1e-6
    Color.hex_to_lch("#0000ff").h.should be_close(264.05, 0.1)
    Color.legacy_lms = true
    Color.hex_to_lch("#0000ff").h.should be_close(266.66, 0.01) # the recorded engine's table
    Color.legacy_lms = false
  end

  it "round-trips 8-bit colours through OKLab, OKLCH and HSL" do
    %w(#000000 #ffffff #123456 #fedcba #ff8000 #7aa2f7 #1a1b26 #00ff00).each do |c|
      Color.lab_to_hex(Color.hex_to_lab(c)).should eq(c)
      Color.lch_to_hex(Color.hex_to_lch(c)).should eq(c)
      h = Color.hex_to_hsl(c)
      Color.hsl_to_hex(h.h, h.s, h.l).should eq(c)
    end
  end

  it "reads hex like the reference (bad input is black) and rounds half away from zero" do
    Color.hex_to_rgb("7AA2F7").should eq(Color::RGB.new(122.0, 162.0, 247.0))
    Color.hex_to_rgb("#zzzzzz").should eq(Color::RGB.new(0.0, 0.0, 0.0))
    Color.rgb_to_hex(0.5, 1.5, 254.5).should eq("#0102ff")
    Color.fmod(-30.0, 360.0).should eq(-30.0)
  end

  it "measures WCAG contrast" do
    Color.contrast("#000000", "#ffffff").should be_close(21.0, 1e-9)
    Color.contrast("#777777", "#ffffff").should be_close(4.478, 0.01)
  end

  it "runs the slider pipeline" do
    Color.adjust("#ff0000", {} of String => Float64).should eq("#ff0000")
    Color.adjust("#ff0000", {"hueShift" => 180.0}).should eq("#00ffff")
    Color.adjust("#3366cc", {"saturation" => -100.0}).should eq("#808080")
    Color.adjust("#808080", {"brightness" => 20.0}).should eq("#b3b3b3")
    Color.adjust("#808080", {"gamma" => 2.0}).should eq("#b5b5b5")
    Color.adjust("#202020", {"blackPoint" => 10.0}).should_not eq("#202020")
    Color::SLIDERS.size.should eq(12)
  end
end
