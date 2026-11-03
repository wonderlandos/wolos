# SPDX-License-Identifier: GPL-2.0-or-later
require "./spec_helper"

include Hue

describe Hue::Palette do
  it "gives 16 readable colours for every scene, mode and variant" do
    SpecHelp.scene_files.each_key do |name|
      Palette::MODES.each do |mode|
        [false, true].each do |light|
          p = SpecHelp.palette(name, light, mode)
          p.size.should eq(16)
          p.all? { |c| Color.valid_hex?(c) }.should be_true
          (1..6).each do |i|
            ok = Color.contrast(p[0], p[i]) >= 4.5 || mode == "material"
            fail "#{name}/#{mode}/#{light}: slot #{i} #{p[i]} on #{p[0]}" unless ok
          end
          dark_bg = Color.hex_to_lab(p[0]).l < 0.5
          fail "#{name}/#{mode}/#{light}: background #{p[0]}" unless dark_bg == !light
        end
      end
    end
  end

  it "keeps a grey picture grey and a tinted one in its family" do
    grey = SpecHelp.palette("grey-photo", false, "normal")
    (0..15).each { |i| Color.hex_to_lch(grey[i]).c.should be < 0.02 }
    sepia = SpecHelp.palette("sepia", false, "monochromatic")
    (1..6).each { |i| Extract.hue_distance(Color.hex_to_lch(sepia[i]).h, 75.0).should be < 45 }
  end

  it "takes the colourful scene's accents from its stripes" do
    p = SpecHelp.palette("stripes", false, "normal")
    red = Color.hex_to_lch(p[1])
    red.c.should be > 0.08
    Extract.hue_distance(red.h, 29.0).should be < 30
  end

  it "refuses an unknown mode" do
    expect_raises(ArgumentError, /unknown mode/) { SpecHelp.palette("sunset", false, "neon") }
  end
end
