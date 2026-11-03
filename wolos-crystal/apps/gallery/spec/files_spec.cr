# SPDX-License-Identifier: GPL-2.0-or-later
require "./spec_helper"

describe Gallery::Library do
  it "lists the pictures under a folder, skips hidden folders, links and non-pictures" do
    with_tmpdir do |dir|
      Dir.mkdir_p(File.join(dir, "Trip"))
      Dir.mkdir_p(File.join(dir, ".cache"))
      File.write(File.join(dir, "a.jpg"), Fixtures.jpeg(tiff: Fixtures.camera_tiff))
      File.write(File.join(dir, "Trip", "b.png"), Fixtures.png)
      File.write(File.join(dir, ".cache", "c.jpg"), Fixtures.jpeg)
      File.write(File.join(dir, "fake.jpg"), "not a picture")
      File.write(File.join(dir, "notes.txt"), "hello")
      File.symlink(File.join(dir, "Trip"), File.join(dir, "loop"))
      r = Gallery::Library.new(dir, nil, UTC).scan
      r.photos.map(&.name).sort!.should eq ["a.jpg", "b.png"]
      r.skipped.should eq 1
      r.missing.should be_false
      a = r.photos.find!(&.name.==("a.jpg"))
      a.date.should eq Time.utc(2026, 10, 31, 18, 4, 5)
      a.width.should eq 640
      a.thumbnail.should eq a.path
    end
  end

  it "says when the folder is not there" do
    with_tmpdir do |dir|
      Gallery::Library.new(File.join(dir, "nope"), nil).scan.missing.should be_true
    end
  end

  it "reads only the folder itself at depth 0" do
    with_tmpdir do |dir|
      Dir.mkdir_p(File.join(dir, "sub"))
      File.write(File.join(dir, "a.jpg"), Fixtures.jpeg)
      File.write(File.join(dir, "sub", "b.jpg"), Fixtures.jpeg)
      Gallery::Library.new(dir, nil, depth: 0).scan.photos.map(&.name).should eq ["a.jpg"]
    end
  end
end

describe Gallery::Thumbs do
  it "copies the stored preview into the cache once, and prunes what no picture uses" do
    with_tmpdir do |dir|
      pics = File.join(dir, "pics")
      cache = File.join(dir, "cache")
      Dir.mkdir_p(pics)
      File.write(File.join(pics, "a.jpg"), Fixtures.jpeg(tiff: Fixtures.camera_tiff))
      File.write(File.join(pics, "b.jpg"), Fixtures.jpeg)
      File.write(File.join(dir, "cache-stray"), "x")
      thumbs = Gallery::Thumbs.new(cache)
      r = Gallery::Library.new(pics, thumbs, UTC).scan
      a = r.photos.find!(&.name.==("a.jpg"))
      b = r.photos.find!(&.name.==("b.jpg"))
      a.thumbnail.should start_with(cache)
      File.read(a.thumbnail).to_slice.should eq Fixtures::PREVIEW
      File.info(a.thumbnail).permissions.should eq File::Permissions.new(0o600)
      b.thumbnail.should eq b.path
      stale = File.join(cache, "#{"0" * 64}.jpg")
      File.write(stale, "old")
      Gallery::Library.new(pics, thumbs, UTC).scan
      File.exists?(stale).should be_false
      File.exists?(a.thumbnail).should be_true
    end
  end

  it "keys a thumbnail by path, size and change time" do
    t = Gallery::Thumbs.new("/c")
    k = t.key("/p/a.jpg", 10_i64, Time.unix(1))
    k.size.should eq 64
    t.key("/p/a.jpg", 11_i64, Time.unix(1)).should_not eq k
    t.key("/p/a.jpg", 10_i64, Time.unix(2)).should_not eq k
  end
end

describe Gallery::Trash do
  it "moves a picture into the shared Trash with its note, and puts it back" do
    with_tmpdir do |dir|
      pic = File.join(dir, "My pics", "a b.jpg")
      Dir.mkdir_p(File.dirname(pic))
      File.write(pic, "x")
      trash = Gallery::Trash.new(File.join(dir, "Trash"))
      item = trash.move(pic, Time.local(2026, 11, 3, 15, 4, 5))
      File.exists?(pic).should be_false
      File.read(File.join(trash.files, "a b.jpg")).should eq "x"
      note = File.read(File.join(trash.info, "a b.jpg.trashinfo"))
      note.should eq "[Trash Info]\nPath=#{URI.encode_path(pic)}\nDeletionDate=2026-11-03T15:04:05\n"
      note.should contain("My%20pics/a%20b.jpg")
      trash.restore(item)
      File.read(pic).should eq "x"
      File.exists?(File.join(trash.info, "a b.jpg.trashinfo")).should be_false
    end
  end

  it "gives a second picture of the same name a free name" do
    with_tmpdir do |dir|
      trash = Gallery::Trash.new(File.join(dir, "Trash"))
      2.times do |i|
        Dir.mkdir_p(File.join(dir, i.to_s))
        File.write(File.join(dir, i.to_s, "a.jpg"), i.to_s)
      end
      trash.move(File.join(dir, "0", "a.jpg")).name.should eq "a.jpg"
      trash.move(File.join(dir, "1", "a.jpg")).name.should eq "a 2.jpg"
    end
  end

  it "says so when the picture is gone, or when its place is taken on restore" do
    with_tmpdir do |dir|
      trash = Gallery::Trash.new(File.join(dir, "Trash"))
      expect_raises(Gallery::TrashError, "This picture is gone already.") { trash.move(File.join(dir, "none.jpg")) }
      pic = File.join(dir, "a.jpg")
      File.write(pic, "x")
      item = trash.move(pic)
      File.write(pic, "new")
      expect_raises(Gallery::TrashError, /stays in the Trash/) { trash.restore(item) }
    end
  end
end

describe Gallery::Dirs do
  it "reads the Pictures folder from the folder settings, else ~/Pictures" do
    env = {"HOME" => "/h"}
    Gallery::Dirs.pictures(env, %(# x\nXDG_PICTURES_DIR="$HOME/Photos"\n)).should eq "/h/Photos"
    Gallery::Dirs.pictures(env, %(XDG_PICTURES_DIR="../etc"\n)).should eq "/h/Pictures"
    Gallery::Dirs.pictures(env, "").should eq "/h/Pictures"
    Gallery::Dirs.thumbnails(env).should eq "/h/.local/share/org.wolos.gallery/thumbnails"
    Gallery::Dirs.trash({"HOME" => "/h", "XDG_DATA_HOME" => "/d"}).should eq "/d/Trash"
  end
end
