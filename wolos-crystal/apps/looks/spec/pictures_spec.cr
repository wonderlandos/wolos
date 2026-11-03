# SPDX-License-Identifier: GPL-2.0-or-later
# The pictures Looks offers: listing the folders, thumbnails, and keeping a picture the file panel
# handed over.
require "./spec_helper"

describe Looks::Pictures do
  it "lists pictures one level deep, in order, without hidden files, links or other kinds" do
    with_tmpdir do |dir|
      a = File.join(dir, "walls")
      b = File.join(dir, "pics")
      FileUtils.mkdir_p([a, b])
      %w(zeta.png alpha.JPG notes.txt .hidden.png).each { |n| File.write(File.join(a, n), "x") }
      File.write(File.join(b, "photo.webp"), "x")
      File.symlink(File.join(b, "photo.webp"), File.join(b, "link.png"))
      FileUtils.mkdir_p(File.join(a, "sub.png"))
      Looks::Pictures.list([a, b, File.join(dir, "missing")]).map { |p| File.basename(p) }.should eq %w(alpha.JPG zeta.png photo.webp)
      Looks::Pictures.list([a, b], limit: 2).size.should eq 2
    end
  end

  it "makes a title from a file name" do
    Looks::Pictures.title("/x/wonderland-day_01.svg").should eq "wonderland day 01"
  end

  it "keeps a handed-over picture once, under a safe name" do
    with_tmpdir do |dir|
      folder = File.join(dir, "kept")
      one = Looks::Pictures.keep(folder, "../../my pic.png", IO::Memory.new("bytes"))
      File.dirname(one).should eq folder
      File.basename(one).should match(/\A[0-9a-f]{12}-my-pic\.png\z/)
      File.read(one).should eq "bytes"
      File.info(one).permissions.value.should eq 0o600
      Looks::Pictures.keep(folder, "../../my pic.png", IO::Memory.new("bytes")).should eq one
      Dir.children(folder).size.should eq 1
      Looks::Pictures.keep(folder, "...", IO::Memory.new("other")).should match(/-picture\z/)
    end
  end

  it "refuses a picture over the size limit" do
    with_tmpdir do |dir|
      big = IO::Memory.new(Bytes.new(Looks::Pictures::MAX_BYTES.to_i + 10))
      expect_raises(Hue::ImageError, /64 MB/) { Looks::Pictures.keep(dir, "big.png", big) }
    end
  end
end

describe "Hue::Picture.load with a size limit" do
  it "decodes a long picture at the limit and a short one as it is" do
    with_tmpdir do |dir|
      big = File.join(dir, "big.png")
      Hue::Picture.new(3000, 1000, Bytes.new(3000 * 1000 * 4, 200_u8)).save(big)
      p = Hue::Picture.load(big, 800)
      {p.width, p.height}.should eq({800, 266})
      q = Hue::Picture.load(big)
      {q.width, q.height}.should eq({3000, 1000})
      small = Hue::Picture.load(scene_file(dir, "ocean"), 800)
      small.width.should eq 480
    end
  end
end

describe Looks::Thumbs do
  it "makes a 96 x 64 thumbnail once and finds it again" do
    with_tmpdir do |dir|
      pic = scene_file(dir, "forest")
      t = Looks::Thumbs.new(File.join(dir, "thumbs"))
      file = must(t.fetch(pic))
      Hue::Picture.load(file).tap { |p| {p.width, p.height}.should eq({96, 64}) }
      mtime = File.info(file).modification_time
      t.fetch(pic).should eq file
      File.info(file).modification_time.should eq mtime
    end
  end

  it "gives nothing for a file that is not a picture, and prunes what is not wanted" do
    with_tmpdir do |dir|
      bad = File.join(dir, "bad.png")
      File.write(bad, "no")
      t = Looks::Thumbs.new(File.join(dir, "thumbs"))
      t.fetch(bad).should be_nil
      t.fetch(File.join(dir, "gone.png")).should be_nil
      good = must(t.fetch(scene_file(dir, "ocean")))
      File.write(File.join(dir, "thumbs", "stale.png"), "x")
      t.prune(Set{File.basename(good, ".png")})
      Dir.children(File.join(dir, "thumbs")).should eq [File.basename(good)]
    end
  end
end
