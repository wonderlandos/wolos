# SPDX-License-Identifier: BSD-2-Clause
# The gallery in the real headless engine: a fixed library, a fake system, a fixed clock, and the
# golden files (screenshots with the pinned fonts, accessibility trees). WOLKIT_BLESS=1 rewrites them.
require "spec"
require "../../../../shards/wolkit/src/wolkit/test"
require "../../src/gallery"

module GallerySpec
  NOW = Time.utc(2026, 11, 3, 15, 0, 0)

  # A library that answers at once (or never, for the loading state).
  class FixedSource < Gallery::Source
    getter folder : String = "/home/ada/Pictures"
    getter loads = 0

    def initialize(@outcome : Gallery::ScanOutcome?)
    end

    def load(&done : Gallery::ScanOutcome ->) : Nil
      @loads += 1
      @outcome.try { |o| done.call(o) }
    end
  end

  # Records what the app asked of the system.
  class FakeSystem < Gallery::System
    getter trashed = [] of String
    getter shared = [] of String
    getter restored = [] of String
    property refuse : String? = nil

    def trash(path : String) : Gallery::Trashed | String
      if sentence = @refuse
        return sentence
      end
      @trashed << path
      Gallery::Trashed.new(path, File.basename(path))
    end

    def restore(item : Gallery::Trashed) : String?
      @restored << item.original
      nil
    end

    def share(path : String, &done : String? ->) : Nil
      @shared << path
      done.call(nil)
    end

    def open_with? : Bool
      false
    end

    def open_with(path : String) : String?
      Gallery::OPEN_WITH_MISSING
    end
  end

  def self.photo(name : String, at : Time, place : Gallery::GeoPoint? = nil, camera = true) : Gallery::Photo
    exif = camera ? Gallery::Exif.new(taken: at, place: place, camera: "Wolcam One", exposure: "1/120 s", aperture: "f/1.8", iso: 100) : Gallery::Exif.new
    Gallery::Photo.new("/home/ada/Pictures/#{name}", 2_480_000_i64, at, exif, 4000, 3000)
  end

  LISBON = Gallery::GeoPoint.new(38.72, -9.14)
  PORTO  = Gallery::GeoPoint.new(41.15, -8.61)

  def self.library : Array(Gallery::Photo)
    [
      photo("IMG_0412.jpg", Time.utc(2026, 11, 3, 9, 12, 0), PORTO),
      photo("IMG_0411.jpg", Time.utc(2026, 11, 3, 8, 2, 0), PORTO),
      photo("IMG_0398.jpg", Time.utc(2026, 11, 2, 19, 40, 0), PORTO),
      photo("IMG_0377.jpg", Time.utc(2026, 10, 31, 18, 4, 5), LISBON),
      photo("IMG_0376.jpg", Time.utc(2026, 10, 31, 17, 58, 0), LISBON),
      photo("IMG_0375.jpg", Time.utc(2026, 10, 31, 17, 30, 0), LISBON),
      photo("IMG_0201.jpg", Time.utc(2026, 9, 14, 11, 0, 0), LISBON),
      photo("Screenshot.png", Time.utc(2026, 9, 2, 10, 0, 0), camera: false),
      photo("IMG_0102.jpg", Time.utc(2026, 8, 20, 7, 30, 0)),
    ]
  end

  def self.ready : Gallery::ScanOutcome
    Gallery::ScanResult.new(library, 0, false, "/home/ada/Pictures")
  end

  # The app of one state.
  class App < Gallery::App
    @@outcome : Gallery::ScanOutcome? = nil
    @@size = {Gallery::App::WIDTH, Gallery::App::HEIGHT}
    class_getter system = FakeSystem.new

    def self.next(outcome : Gallery::ScanOutcome?, size = {Gallery::App::WIDTH, Gallery::App::HEIGHT}) : App.class
      @@outcome = outcome
      @@size = size
      @@system = FakeSystem.new
      self
    end

    def initialize
      super(FixedSource.new(@@outcome), @@system, -> { NOW }, size: @@size)
    end
  end

  def self.run(outcome : Gallery::ScanOutcome?, size = {Gallery::App::WIDTH, Gallery::App::HEIGHT}, & : Wolkit::Test::Session, App ->) : Nil
    Wolkit::Test.run(App.next(outcome, size)) do |t|
      yield t, t.app.as(App)
    end
  end

  # Compare with (or, with WOLKIT_BLESS=1, write) spec/golden/<name>.
  def self.golden(name : String, actual : String | Bytes) : Nil
    path = File.join(__DIR__, "..", "golden", name)
    bytes = actual.is_a?(String) ? actual.to_slice : actual
    if ENV["WOLKIT_BLESS"]?
      Dir.mkdir_p(File.dirname(path))
      File.write(path, bytes)
    else
      File.exists?(path).should be_true, "#{path} is missing; run WOLKIT_BLESS=1 make spec and review it"
      File.read(path).to_slice.should eq(bytes), "#{path} differs; run WOLKIT_BLESS=1 make spec and review the diff"
    end
  end

  # One state: the screenshot and the accessibility tree.
  def self.pin(t : Wolkit::Test::Session, name : String) : Nil
    golden("#{name}.png", t.screenshot)
    golden("#{name}.a11y", t.a11y_snapshot)
  end
end

Wolkit.on_error { |ex| STDERR.puts ex.inspect_with_backtrace }
