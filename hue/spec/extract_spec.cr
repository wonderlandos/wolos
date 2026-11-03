# SPDX-License-Identifier: GPL-2.0-or-later
# The engine's unit fixtures (coverage weights, background bias, graded boost) plus picture checks.
require "./spec_helper"

include Hue

describe Hue::Extract do
  it "turns counts into coverage shares" do
    Extract.weights([1, 3]).should eq([0.25, 0.75])
    Extract.weights([0, 0]).should be_nil
  end

  it "picks the background by lightness alone without weights" do
    c = %w(#202020 #000000 #808080)
    Extract.background(c, nil, false)[1].should eq(1)
    Extract.background(c, nil, true)[1].should eq(2)
  end

  it "lets a dominant deep tone beat a darker speck" do
    c = %w(#050505 #10243a)
    Extract.background(c, nil, false)[1].should eq(0)
    Extract.background(c, Extract.weights([2, 98]), false)[1].should eq(1)
  end

  it "never lets coverage flip lightness" do
    Extract.background(%w(#101010 #f0f0f0), Extract.weights([5, 95]), false)[1].should eq(0)
  end

  it "grades the chroma boost" do
    at = ->(c : Float64) { Extract.boost([Color::Lab.new(0.5, c, 0.0)]).size }
    at.call(0.0).should eq(1)
    at.call(Extract::BOOST_THRESHOLD).should eq(1)
    at.call(Extract::BOOST_THRESHOLD + 0.001).should eq(2)
    at.call(Extract::BOOST_THRESHOLD + Extract::BOOST_RAMP).should eq(4)
    at.call(Extract::BOOST_THRESHOLD + 2 * Extract::BOOST_RAMP).should eq(4)
    prev = 0
    31.times { |i| n = at.call(i * 0.01); n.should be >= prev; prev = n }
  end

  it "samples a 400 px copy and skips transparent pixels" do
    pic = Scenes.scene("pastel-portrait")
    small = pic.fit(400)
    {small.width, small.height}.should eq({225, 400})
    px = Extract.sample(pic)
    px.size.should be < 225 * 400
    px.size.should be > 225 * 400 * 9 // 10
  end

  it "keeps a flat picture flat through the resampler" do
    flat = Scenes.draw(1000, 500, 1_u32, 0.0) { {40.0, 80.0, 120.0, 255.0} }.fit(400)
    {flat.width, flat.height}.should eq({400, 200})
    flat.rgba[0, 4].to_a.should eq([40, 80, 120, 255])
    flat.rgba[(flat.rgba.size - 4)..].to_a.should eq([40, 80, 120, 255])
  end

  it "finds 48 dominant colours by coverage" do
    dom, counts = Extract.dominant(Extract.sample(Picture.load(SpecHelp.scene_files["stripes"])))
    dom.size.should eq(48)
    counts.should eq(counts.sort.reverse!)
    dom.all?(&.matches?(/\A#[0-9A-F]{6}\z/)).should be_true
  end

  it "tells monochrome pictures from colourful ones" do
    mono = ->(n : String) {
      dom, counts = Extract.dominant(Extract.sample(Picture.load(SpecHelp.scene_files[n])))
      Extract.monochrome_weighted?(dom, Extract.weights(counts))
    }
    mono.call("grey-photo").should be_true
    mono.call("sepia").should be_true
    mono.call("snow-speck").should be_true
    mono.call("stripes").should be_false
    mono.call("sunset").should be_false
  end

  it "refuses what is not a picture" do
    f = File.join(SpecHelp.tmpdir, "x.png")
    File.write(f, "not a png")
    expect_raises(ImageError) { Picture.load(f) }
    expect_raises(ImageError) { Picture.load("/nonexistent.png") }
  end
end
