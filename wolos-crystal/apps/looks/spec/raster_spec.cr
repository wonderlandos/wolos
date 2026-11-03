# SPDX-License-Identifier: GPL-2.0-or-later
# The preview pictures: the colours painted are the candidate's tokens, the size is fixed, and the
# same input gives the same bytes. LOOKS_SAMPLES=<folder> writes the pictures there to look at.
require "./spec_helper"

private def cand(dir : String, scene : String, mode : String, light : Bool, adj = {} of String => Float64) : Looks::Candidate
  a = Looks::Analysis.read(scene_file(dir, scene))
  Looks::Candidate.new(a.palette(light, mode), light, mode, adj)
end

describe Looks::Canvas do
  it "paints an anti-aliased rounded rectangle inside its box only" do
    c = Looks::Canvas.new(20, 20, Looks::Rgba.new(0, 0, 0, 1.0))
    c.rect(5, 5, 10, 10, Looks::Rgba.new(255, 255, 255, 1.0), 3)
    c.at(10, 10).should eq({255, 255, 255})
    c.at(2, 2).should eq({0, 0, 0})
    c.at(5, 5)[0].should be < 255 # the rounded corner is not fully covered
    c.at(0, 19).should eq({0, 0, 0})
  end

  it "ignores shapes outside the canvas" do
    c = Looks::Canvas.new(4, 4, Looks::Rgba.new(1, 2, 3, 1.0))
    c.rect(-10, -10, 100, 100, Looks::Rgba.new(9, 9, 9, 1.0))
    c.at(3, 3).should eq({9, 9, 9})
    c.rect(100, 100, 5, 5, Looks::Rgba.new(0, 0, 0, 1.0))
  end
end

describe Looks::Raster do
  it "paints each half of a mode card with its own tokens" do
    with_tmpdir do |dir|
      dark = cand(dir, "sunset", "normal", false)
      light = cand(dir, "sunset", "normal", true)
      pic = Looks::Raster.card(dark, light)
      pic.width.should eq Looks::Raster::CARD_W
      pic.height.should eq Looks::Raster::CARD_H
      px = ->(x : Int32, y : Int32) { o = (y * pic.width + x) * 4; {pic.rgba[o].to_i, pic.rgba[o + 1].to_i, pic.rgba[o + 2].to_i} }
      d = must(dark.tokens).surface
      l = must(light.tokens).surface
      # the strip under the swatches belongs to the panel's surface
      px.call(1, 1).should eq({d.r, d.g, d.b})
      px.call(pic.width - 2, 1).should eq({l.r, l.g, l.b})
      px.call(1, 1).should_not eq px.call(pic.width - 2, 1)
    end
  end

  it "paints the 16 colours of the palette in a card" do
    with_tmpdir do |dir|
      c = cand(dir, "forest", "colorful", false)
      pic = Looks::Raster.card(c, cand(dir, "forest", "colorful", true))
      pal = c.final.map { |h| Looks::Raster.hex(h) }
      16.times do |i|
        x = 4 + (i % 8) * 7 + 3
        y = 5 + (i // 8) * 7 + 3
        o = (y * pic.width + x) * 4
        {pic.rgba[o].to_i, pic.rgba[o + 1].to_i, pic.rgba[o + 2].to_i}.should eq({pal[i].r, pal[i].g, pal[i].b})
      end
    end
  end

  it "gives the same bytes twice and different bytes for a slider" do
    with_tmpdir do |dir|
      a = Looks::Raster.settings(cand(dir, "sunset", "muted", false))
      b = Looks::Raster.settings(cand(dir, "sunset", "muted", false))
      a.rgba.should eq b.rgba
      c = Looks::Raster.settings(cand(dir, "sunset", "muted", false, {"temperature" => 45.0}))
      c.rgba.should_not eq a.rgba
    end
  end

  it "draws the four app previews in the candidate's tokens and at one size" do
    with_tmpdir do |dir|
      c = cand(dir, "ocean", "normal", false)
      t = must(c.tokens)
      pal = c.final.map { |h| Looks::Raster.hex(h) }
      {Looks::Raster.terminal(c), Looks::Raster.settings(c), Looks::Raster.document(c), Looks::Raster.browser(c)}.each do |pic|
        pic.width.should eq Looks::Raster::PREVIEW_W
        pic.height.should eq Looks::Raster::PREVIEW_H
      end
      term = Looks::Raster.terminal(c)
      # a terminal is colour 0 behind its text
      o = (50 * term.width + 140) * 4
      {term.rgba[o].to_i, term.rgba[o + 1].to_i, term.rgba[o + 2].to_i}.should eq({pal[0].r, pal[0].g, pal[0].b})
      # the browser's banner is the accent
      br = Looks::Raster.browser(c)
      o = (32 * br.width + 120) * 4
      {br.rgba[o].to_i, br.rgba[o + 1].to_i, br.rgba[o + 2].to_i}.should eq({t.accent.r, t.accent.g, t.accent.b})
    end
  end

  it "shows a hatched grey for a candidate with no readable theme" do
    pic = Looks::Raster.unreadable(20, 20)
    pic.width.should eq 20
  end

  it "writes samples to look at when asked" do
    next unless dir = ENV["LOOKS_SAMPLES"]?
    FileUtils.mkdir_p(dir)
    Looks::MODES.each do |m|
      with_tmpdir do |tmp|
        a = Looks::Analysis.read(scene_file(tmp, "sunset"))
        Looks::Raster.card(Looks::Candidate.new(a.palette(false, m), false, m), Looks::Candidate.new(a.palette(true, m), true, m)).save(File.join(dir, "card-#{m}.png"))
      end
    end
    with_tmpdir do |tmp|
      a = Looks::Analysis.read(scene_file(tmp, "sunset"))
      c = Looks::Candidate.new(a.palette(false, "normal"), false, "normal")
      Looks::Raster.terminal(c).save(File.join(dir, "terminal.png"))
      Looks::Raster.settings(c).save(File.join(dir, "settings.png"))
      Looks::Raster.document(c).save(File.join(dir, "document.png"))
      Looks::Raster.browser(c).save(File.join(dir, "browser.png"))
    end
  end
end
