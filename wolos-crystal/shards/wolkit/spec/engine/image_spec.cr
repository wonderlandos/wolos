# SPDX-License-Identifier: BSD-2-Clause
# The `image` element against the headless engine (WK-image): a PNG on disk drawn with each fit,
# rounded by `radius:`, named by `alt:` in the accessibility tree, and a missing file shown as the
# placeholder. The headless engine waits for its decodes before it draws, so the screenshot is the
# same on every run and is pinned as a golden PNG. Also `FFI::Image`, the handle for `picture:`.
require "./engine_helper_spec"

# A 64 x 48 picture in four quadrants (red, green, blue, and yellow at half alpha) with a white
# cross through the middle, written once per run.
module SpecPicture
  PATH = File.join(Dir.tempdir, "wolkit-image-spec-#{Process.pid}.png")

  def self.pixels : Bytes
    w, h = 64, 48
    px = Bytes.new(w * h * 4)
    h.times do |y|
      w.times do |x|
        c = if x == 31 || x == 32 || y == 23 || y == 24
              {255, 255, 255, 255}
            elsif y < 24
              x < 32 ? {220, 40, 40, 255} : {40, 180, 60, 255}
            else
              x < 32 ? {40, 70, 220, 255} : {240, 200, 30, 128}
            end
        i = (y * w + x) * 4
        px[i] = c[0].to_u8
        px[i + 1] = c[1].to_u8
        px[i + 2] = c[2].to_u8
        px[i + 3] = c[3].to_u8
      end
    end
    px
  end

  def self.path : String
    File.write(PATH, Wolkit::Test::PNG.encode(pixels, 64, 48)) unless File.exists?(PATH)
    PATH
  end
end

class SpecPictures < Wolkit::App
  def view
    window "Pictures", width: 360, height: 140 do
      row gap: :md, pad: :md do
        image src: SpecPicture.path, fit: :cover, radius: :l, width: 120, height: 100, alt: "Four colours, cut to fill"
        image src: SpecPicture.path, fit: :contain, bg: :line, width: 100, height: 100, alt: "Four colours, whole"
        image src: SpecPicture.path, fit: :none, width: 48, height: 32, alt: "Four colours, actual size"
        image src: "/nonexistent/wolkit-spec.png", radius: :s, width: 40, height: 40, decorative: true
      end
    end
  end
end

def golden_png(name : String, actual : Bytes) : Nil
  path = File.join(__DIR__, "..", "golden", name)
  if ENV["WOLKIT_BLESS"]?
    Dir.mkdir_p(File.dirname(path))
    File.write(path, actual)
  else
    File.read(path).to_slice.should eq(actual), "#{path} differs; run WOLKIT_BLESS=1 make spec and look at the picture"
  end
end

describe "pictures through the engine" do
  it "draws a picture from a file with each fit and rounded corners" do
    Wolkit::Test.run(SpecPictures) do |t|
      golden_png("engine/pictures.png", t.screenshot)
      golden("engine/pictures.a11y", t.a11y_snapshot)
      t.a11y_snapshot.should contain %(role=image name="Four colours, cut to fill")
    end
  end

  it "holds a picture made from bytes and reports its size and state" do
    Wolkit::Test.run(SpecPictures) do |t|
      engine = Wolkit::Runtime.backend.as(Wolkit::EngineBackend).engine
      png = Wolkit::Test::PNG.encode(SpecPicture.pixels, 64, 48)
      pic = Wolkit::FFI::Image.from_bytes(engine, png, max_side: 32)
      pic.size.should eq({64_u32, 48_u32})
      pic.id.should be > 0
      t.screenshot # waits for the decode
      pic.state.should eq Wolkit::FFI::Image::State::Ready
      raw = Wolkit::FFI::Image.from_rgba(engine, 2, 1, Bytes[1, 2, 3, 255, 4, 5, 6, 255])
      raw.state.should eq Wolkit::FFI::Image::State::Ready
      expect_raises(Wol::InvalidArgument) { Wolkit::FFI::Image.from_bytes(engine, "GIF89a".to_slice) }
      pic.close
      raw.close
    end
  end
end
