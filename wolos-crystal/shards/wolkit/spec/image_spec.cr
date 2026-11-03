# SPDX-License-Identifier: BSD-2-Clause
# Specs for the `image` element (WK-image): `src:` names a file the engine reads and decodes, `fit:`
# places it (contain, cover, fill, none), `radius:` rounds its corners, and `alt:` is its name for
# assistive technology (or `decorative: true` for none). What it emits is pinned as ops.
require "./spec_helper"

class SpecImageOps < Wolkit::App
  @path = signal("/pictures/b.jpg")

  getter path

  def view
    window "Pictures", width: 320, height: 200 do
      row gap: :md do
        image src: "/pictures/a.jpg", fit: :cover, radius: :m, width: 160, height: 120, alt: "A beach at dusk"
        image src: @path, fit: :none, alt: "The second picture"
        image src: "/pictures/c.png", decorative: true
      end
    end
  end
end

describe "the image element" do
  after_each { Wolkit::Test.reset }

  it "writes the source, fit, radius and alt text" do
    window = Wolkit::Test.record(SpecImageOps.new).windows.first
    golden("image_mounted.ops", window.ops.join("\n") + "\n")
    ops = window.ops.join("\n")
    ops.should contain %(SET_A11Y 2 image "A beach at dusk" "")
    ops.should contain %(image-source "/pictures/a.jpg")
  end

  it "follows a signal for its source" do
    app = SpecImageOps.new
    window = Wolkit::Test.record(app).windows.first
    app.path.value = "/pictures/d.jpg"
    Wolkit::Frame.flush_all
    window.last_ops.should eq [%(SET_PROP 3 image-source "/pictures/d.jpg")]
  end
end
