# SPDX-License-Identifier: GPL-2.0-or-later
require "./spec_helper"

include Hue

describe Hue::Theme do
  it "derives tokens that pass the desktop's contrast contract for every scene" do
    SpecHelp.scene_files.each_key do |name|
      Palette::MODES.each do |mode|
        [false, true].each do |light|
          t = Theme.tokens(SpecHelp.palette(name, light, mode), !light)
          t.keys.sort.should eq(Theme::TOKENS.sort)
          Theme.check(t).should eq([] of String)
        end
      end
    end
  end

  it "survives hostile palettes (all one colour, all mid grey)" do
    ["#808080", "#000000", "#ffffff", "#ffff00"].each do |c|
      [true, false].each { |dark| Theme.check(Theme.tokens([c] * 16, dark)).should be_empty }
    end
  end

  it "writes a theme.json the desktop's parser reads" do
    p = SpecHelp.palette("ocean", false, "normal")
    j = JSON.parse(Theme.json("picture-ocean", "Ocean", p, true, "/tmp/ocean.png"))
    j["id"].should eq("picture-ocean")
    j["variant"].should eq("dark")
    j["terminal"].as_a.size.should eq(16)
    j["terminal"].as_a.all? { |c| c.as_s.matches?(/\A#[0-9a-f]{6}\z/) }.should be_true
    j["tokens"].as_h.size.should eq(13)
    j["wallpaper"].should eq("/tmp/ocean.png")
  end

  it "parses colours like the desktop and reports what fails" do
    Theme.parse("#abc").should eq(Theme::Rgba.new(0xaa, 0xbb, 0xcc, 1.0))
    Theme.parse("rgba(1,2,3,0.5)").should eq(Theme::Rgba.new(1, 2, 3, 0.5))
    Theme.parse("rgb(256,0,0)").should be_nil
    Theme.parse("blue").should be_nil
    t = Theme.tokens(["#101010"] * 16, true)
    t["text"] = "#202020"
    Theme.check(t).any?(&.starts_with?("text/surface")).should be_true
    t.delete("dim")
    Theme.check(t).should contain("dim: missing")
  end

  it "keeps theme ids inside the themes folder" do
    expect_raises(Theme::Invalid) { Theme.dir("../x") }
    expect_raises(Theme::Invalid) { Theme.dir("A") }
    Theme.slug("My Sunset (2).JPG").should eq("my-sunset-2-jpg")
  end
end
