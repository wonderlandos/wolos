# SPDX-License-Identifier: BSD-2-Clause
require "./spec_helper"

private def photo(name : String, at : Time, place : Gallery::GeoPoint? = nil) : Gallery::Photo
  Gallery::Photo.new("/pictures/#{name}", 1000_i64, at, Gallery::Exif.new(taken: at, place: place))
end

NOW = Time.utc(2026, 11, 3, 15, 0, 0) # a Tuesday

describe Gallery::Sections do
  it "names the last week by day and older pictures by month, newest first" do
    list = [
      photo("a.jpg", Time.utc(2026, 11, 3, 9, 0, 0)),
      photo("b.jpg", Time.utc(2026, 11, 2, 23, 0, 0)),
      photo("c.jpg", Time.utc(2026, 10, 31, 12, 0, 0)),
      photo("d.jpg", Time.utc(2026, 10, 2, 12, 0, 0)),
      photo("e.jpg", Time.utc(2026, 10, 1, 12, 0, 0)),
      photo("f.jpg", Time.utc(2025, 12, 24, 12, 0, 0)),
    ]
    s = Gallery::Sections.by_date(list.reverse, NOW)
    s.map(&.title).should eq ["Today", "Yesterday", "Saturday", "October 2026", "December 2025"]
    s[3].photos.map(&.name).should eq ["d.jpg", "e.jpg"]
    s[3].detail.should eq "2 photos"
    s[0].detail.should eq "1 photo"
  end

  it "groups by place cell, newest place first, pictures without a place last" do
    lisbon = Gallery::GeoPoint.new(38.72, -9.14)
    porto = Gallery::GeoPoint.new(41.15, -8.61)
    list = [
      photo("l1.jpg", Time.utc(2026, 9, 1), lisbon),
      photo("l2.jpg", Time.utc(2026, 10, 5), Gallery::GeoPoint.new(38.73, -9.13)),
      photo("p1.jpg", Time.utc(2026, 11, 1), porto),
      photo("x.jpg", Time.utc(2026, 11, 2)),
    ]
    s = Gallery::Sections.by_place(list)
    s.map(&.title).should eq ["41.2° N, 8.6° W", "38.7° N, 9.1° W", "No place"]
    s[1].photos.map(&.name).should eq ["l2.jpg", "l1.jpg"]
    s[1].detail.should eq "2 photos, September to October 2026"
  end

  it "says a span across years" do
    list = [photo("a.jpg", Time.utc(2025, 12, 1)), photo("b.jpg", Time.utc(2026, 1, 2))]
    Gallery::Sections.span(list).should eq "December 2025 to January 2026"
  end

  it "keeps the same order for pictures of the same second" do
    t = Time.utc(2026, 1, 1)
    a = Gallery::Sections.sorted([photo("b.jpg", t), photo("a.jpg", t)])
    a.map(&.name).should eq ["a.jpg", "b.jpg"]
  end
end

describe Gallery::Format do
  it "writes dates, sizes and the camera line" do
    Gallery::Format.long_date(Time.utc(2026, 10, 31, 18, 4, 5)).should eq "Saturday 31 October 2026 at 18:04"
    Gallery::Format.bytes(820).should eq "820 bytes"
    Gallery::Format.bytes(14_200).should eq "14 KB"
    Gallery::Format.bytes(3_240_000).should eq "3.2 MB"
    Gallery::Format.pixels({4032, 3024}).should eq "4032 × 3024"
    Gallery::Format.pixels(nil).should eq "Unknown"
    e = Gallery::Exif.new(exposure: "1/120 s", aperture: "f/1.8", iso: 100)
    Gallery::Format.exposure(e).should eq "1/120 s, f/1.8, ISO 100"
    Gallery::Format.exposure(Gallery::Exif.new).should be_nil
    Gallery::Format.position(2, 12).should eq "3 of 12"
  end
end
