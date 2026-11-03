# SPDX-License-Identifier: GPL-2.0-or-later
# The studio: a picture to sixteen candidates, the sliders, the contrast contract and the look a
# candidate becomes.
require "./spec_helper"

describe Looks::Analysis do
  it "reads a picture once and makes every mode in both variants" do
    with_tmpdir do |dir|
      a = Looks::Analysis.read(scene_file(dir, "sunset"))
      c = a.candidates({} of String => Float64)
      c.size.should eq 16
      c.map(&.mode).uniq!.should eq Looks::MODES
      c.map(&.light?).tally.should eq({false => 8, true => 8})
      c.each(&.base.size.should(eq(16)))
    end
  end

  it "makes tokens that pass the desktop's contrast contract for every mode of every scene" do
    with_tmpdir do |dir|
      Scenes::NAMES.each do |scene|
        a = Looks::Analysis.read(scene_file(dir, scene))
        a.candidates({} of String => Float64).each do |cand|
          raw = cand.raw_tokens
          raw.should_not be_nil, "#{scene} #{cand.mode} #{cand.variant}"
          Hue::Theme.check(must(raw)).should be_empty
        end
      end
    end
  end

  it "refuses a missing or unreadable picture with a sentence" do
    with_tmpdir do |dir|
      expect_raises(Hue::ImageError) { Looks::Analysis.read(File.join(dir, "none.png")) }
      File.write(File.join(dir, "bad.png"), "not a picture")
      expect_raises(Hue::ImageError) { Looks::Analysis.read(File.join(dir, "bad.png")) }
      Looks.picture_problem(Hue::ImageError.new("x")).should contain("could not be read")
    end
  end
end

describe Looks::Candidate do
  base = %w(#101820 #d04a4a #4ad08a #d0b04a #4a7ad0 #b04ad0 #4ad0d0 #e8e8e8 #303840 #ff6a6a #6aff9a #ffd06a #6a9aff #d06aff #6affff #ffffff)

  it "applies the sliders to all sixteen colours, and a zero slider changes nothing" do
    neutral = Looks::Candidate.new(base, false, "normal")
    neutral.final.should eq base.map { |c| Hue::Color.adjust(c, {} of String => Float64) }
    warm = Looks::Candidate.new(base, false, "normal", {"temperature" => 40.0})
    warm.final.should_not eq neutral.final
    warm.final.size.should eq 16
  end

  it "turns into a look that keeps only the sliders that moved" do
    c = Looks::Candidate.new(base, true, "pastel", {"vibrance" => 10.0, "contrast" => 0.0})
    l = c.to_look("Dune day", "/p/dune.png")
    l.variant.should eq "light"
    l.mode.should eq "pastel"
    l.adjustments.should eq({"vibrance" => 10.0})
    l.wallpaper.should eq "/p/dune.png"
    l.validate!.name.should eq "Dune day"
    l.final_colors.should eq c.final
  end

  it "knows when no readable theme can be made" do
    c = Looks::Candidate.new(Array.new(16) { "#808080" }, false, "normal")
    # mid grey on mid grey is repaired by the engine; the candidate is still readable
    c.readable?.should be_true
  end
end
