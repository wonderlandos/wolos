# SPDX-License-Identifier: GPL-2.0-or-later
require "./spec_helper"

include Hue

describe Hue::Imports do
  it "reads a classic Base16 scheme, leading zeros kept" do
    f = File.join(SpecHelp.tmpdir, "s.yaml")
    File.write(f, <<-Y)
      scheme: "Test Night"
      author: "someone"
      base00: "090300" # background
      base01: 3a3432
      base02: 4a4543
      base03: 5c5855
      base04: 807d7c
      base05: a5a2a2
      base06: d6d5d4
      base07: f7f7f7
      base08: db2d20
      base09: e8bbd0
      base0A: fded02
      base0B: 01a252
      base0C: b5e4f4
      base0D: 01a0e4
      base0E: a16a94
      base0F: cdab53
      Y
    i = Imports.base16(f)
    i.name.should eq("Test Night")
    i.colors[0].should eq("#090300")
    i.colors[1].should eq("#db2d20")
    i.colors[4].should eq("#01a0e4")
    i.colors[7].should eq("#a5a2a2")
    i.colors[8].should eq("#5c5855")
    i.colors[15].should eq("#f7f7f7")
    i.light.should be_nil
  end

  it "reads the palette: form with a variant" do
    f = File.join(SpecHelp.tmpdir, "t.yaml")
    body = (0..15).map { |n| "  base0#{n.to_s(16).upcase}: \"##{(n * 16 + 15).to_s(16).rjust(2, '0') * 3}\"" }.join("\n")
    File.write(f, "system: base16\nname: Day\nvariant: light\npalette:\n#{body}\n")
    i = Imports.base16(f)
    i.name.should eq("Day")
    i.light.should be_true
    i.colors[0].should eq("#0f0f0f")
  end

  it "refuses an incomplete scheme" do
    f = File.join(SpecHelp.tmpdir, "u.yaml")
    File.write(f, "base00: '000000'\n")
    expect_raises(Imports::Invalid, /not enough/) { Imports.base16(f) }
  end

  it "reads colors.toml with semantic names, aliases and the accent" do
    f = File.join(SpecHelp.tmpdir, "colors.toml")
    File.write(f, <<-T)
      # UI Colors (extended)
      accent = "#7aa2f7"
      cursor = "#a9b1d6"
      foreground = "#a9b1d6"
      background = "#1a1b26"
      color0 = "#15161e"
      color1 = "#f7768e"
      color2 = "#9ece6a"
      color3 = "#e0af68"
      color4 = "#7aa2f7"
      color5 = "#bb9af7"
      color6 = "#7dcfff"
      color7 = "#a9b1d6"
      color8 = "#414868"
      color15 = "#c0caf5"
      light_mode = false
      T
    i = Imports.colors_toml(f)
    i.colors[0].should eq("#1a1b26")
    i.colors[7].should eq("#a9b1d6")
    i.colors[9].should eq("#f7768e")
    i.colors[15].should eq("#c0caf5")
    i.accent.should eq("#7aa2f7")
    i.light.should be_false
  end

  it "says which colors.toml line it cannot read" do
    f = File.join(SpecHelp.tmpdir, "bad.toml")
    File.write(f, "background = \"#000000\"\nbackground = \"#111111\"\n")
    expect_raises(Imports::Invalid, /line 2/) { Imports.colors_toml(f) }
  end
end
