# SPDX-License-Identifier: BSD-2-Clause
require "./spec_helper"

describe Gallery::ExifReader do
  it "reads date, place, camera, exposure and preview from a JPEG (little-endian)" do
    file = Fixtures.jpeg(tiff: Fixtures.camera_tiff)
    e = Gallery::ExifReader.read(file, UTC)
    e.taken.should eq Time.utc(2026, 10, 31, 18, 4, 5)
    place = e.place || raise "no place"
    place.latitude.should be_close(41.39, 0.001)
    place.longitude.should be_close(2.17, 0.001)
    e.camera.should eq "Wolcam One"
    e.exposure.should eq "1/120 s"
    e.aperture.should eq "f/1.8"
    e.iso.should eq 100
    e.focal_length.should eq "4.2 mm"
    e.width.should eq 4032
    e.height.should eq 3024
    range = e.preview || raise "no preview"
    file[range.begin, range.size].should eq Fixtures::PREVIEW
  end

  it "reads the same from a big-endian block, south and west" do
    file = Fixtures.jpeg(tiff: Fixtures.camera_tiff(little: false, south: true, west: true))
    e = Gallery::ExifReader.read(file, UTC)
    e.taken.should eq Time.utc(2026, 10, 31, 18, 4, 5)
    place = e.place || raise "no place"
    place.latitude.should be_close(-41.39, 0.001)
    place.longitude.should be_close(-2.17, 0.001)
    e.camera.should eq "Wolcam One"
  end

  it "reads the block of a PNG (eXIf chunk)" do
    e = Gallery::ExifReader.read(Fixtures.png(tiff: Fixtures.camera_tiff(preview: nil)), UTC)
    e.taken.should eq Time.utc(2026, 10, 31, 18, 4, 5)
    e.preview.should be_nil
  end

  it "falls back to the file date of the block when there is no original date" do
    e = Gallery::ExifReader.read(Fixtures.jpeg(tiff: Fixtures.camera_tiff(date: "garbage")), UTC)
    e.taken.should eq Time.utc(2026, 11, 1, 9, 0, 0)
  end

  it "keeps the orientation and says when it is a quarter turn" do
    e = Gallery::ExifReader.read(Fixtures.jpeg(tiff: Fixtures.camera_tiff(orientation: 6)), UTC)
    e.orientation.should eq 6
    e.quarter_turn?.should be_true
  end

  it "drops a place at 0, 0 (cameras write it when they have no fix)" do
    e = Gallery::ExifReader.read(Fixtures.jpeg(tiff: Fixtures.camera_tiff(lat: {0, 0, 0}, lon: {0, 0, 0})), UTC)
    e.place.should be_nil
  end

  it "reads nothing, and does not raise, from a file without a block" do
    e = Gallery::ExifReader.read(Fixtures.jpeg, UTC)
    e.taken.should be_nil
    e.place.should be_nil
    e.orientation.should eq 1
  end

  it "survives every truncation and every flipped byte of a real block" do
    file = Fixtures.jpeg(tiff: Fixtures.camera_tiff)
    (0..file.size).each { |n| Gallery::ExifReader.read(file[0, n], UTC) }
    # A fixed sequence of positions and values (a small linear congruential walk), the same on every run.
    x = 42_u64
    500.times do
      broken = file.dup
      4.times do
        x = (x &* 6364136223846793005_u64 &+ 1442695040888963407_u64)
        broken[((x >> 33) % broken.size).to_i] = ((x >> 17) & 0xff).to_u8
      end
      Gallery::ExifReader.read(broken, UTC)
    end
  end

  it "refuses impossible dates" do
    Gallery::ExifReader.parse_date("2026:02:30 10:00:00", UTC).should be_nil
    Gallery::ExifReader.parse_date("0000:00:00 00:00:00", UTC).should be_nil
    Gallery::ExifReader.parse_date("2024:02:29 23:59:60", UTC).should eq Time.utc(2024, 2, 29, 23, 59, 59)
  end
end

describe Gallery::Picture do
  it "tells JPEG and PNG from their bytes and reads their size" do
    Gallery::Picture.sniff(Fixtures.jpeg).should eq Gallery::Picture::Format::Jpeg
    Gallery::Picture.sniff(Fixtures.png).should eq Gallery::Picture::Format::Png
    Gallery::Picture.sniff("GIF89a".to_slice).should be_nil
    Gallery::Picture.dimensions(Fixtures.jpeg(800, 600, Fixtures.camera_tiff)).should eq({800, 600})
    Gallery::Picture.dimensions(Fixtures.png(32, 16)).should eq({32, 16})
  end

  it "lists files by extension, never hidden ones" do
    Gallery::Picture.listed?("IMG_1.JPG").should be_true
    Gallery::Picture.listed?("a.png").should be_true
    Gallery::Picture.listed?(".hidden.jpg").should be_false
    Gallery::Picture.listed?("notes.txt").should be_false
  end
end
