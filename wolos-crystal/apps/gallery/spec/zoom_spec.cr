# SPDX-License-Identifier: BSD-2-Clause
require "./spec_helper"

describe Gallery::Zoom do
  it "fits a large picture and never blows up a small one" do
    z = Gallery::Zoom.fit({800, 600}, {4000, 3000})
    z.scale.should eq 0.2
    z.label.should eq "Fit"
    z.left.should eq 0.0
    Gallery::Zoom.fit({800, 600}, {100, 50}).scale.should eq 1.0
  end

  it "steps in and out through the fixed steps and back to fit" do
    z = Gallery::Zoom.fit({800, 600}, {4000, 3000})
    z = z.zoom_in
    z.scale.should eq 0.25
    z = z.zoom_in.zoom_in
    z.scale.should eq 1.0
    z.label.should eq "100 %"
    z.zoom_out.zoom_out.zoom_out.fitted?.should be_true
    8.times { z = z.zoom_in }
    z.scale.should eq Gallery::Zoom::MAX
  end

  it "keeps the point under the pointer where it is" do
    z = Gallery::Zoom.fit({800, 600}, {4000, 3000})
    at = {600.0, 450.0}
    # The picture point under `at` before: (at - left) / scale.
    before = {(at[0] - z.left) / z.scale, (at[1] - z.top) / z.scale}
    z2 = z.to(1.0, at)
    after = {(at[0] - z2.left) / z2.scale, (at[1] - z2.top) / z2.scale}
    after[0].should be_close(before[0], 1e-6)
    after[1].should be_close(before[1], 1e-6)
  end

  it "clamps the pan to the picture's edges" do
    z = Gallery::Zoom.fit({800, 600}, {4000, 3000}).to(1.0)
    z.pannable?.should be_true
    far = z.pan(1e6, -1e6)
    far.left.should eq 0.0
    (far.top + far.height).should eq 600.0
    Gallery::Zoom.fit({800, 600}, {4000, 3000}).pan(50, 50).offset.should eq({0.0, 0.0})
  end

  it "toggles actual size and refits after a resize" do
    z = Gallery::Zoom.fit({800, 600}, {4000, 3000})
    z.toggle_actual.scale.should eq 1.0
    z.toggle_actual.toggle_actual.fitted?.should be_true
    z.resize({400, 300}).scale.should eq 0.1
  end
end

describe Gallery::Step do
  it "moves, stops at the ends, or wraps" do
    Gallery::Step.move(0, 5, -1).should eq 0
    Gallery::Step.move(4, 5, 1).should eq 4
    Gallery::Step.move(4, 5, 1, wrap: true).should eq 0
    Gallery::Step.move(0, 0, 1).should eq 0
  end

  it "centres the filmstrip window and keeps it inside the list" do
    Gallery::Step.window(10, 100, 5).should eq 8...13
    Gallery::Step.window(0, 100, 5).should eq 0...5
    Gallery::Step.window(99, 100, 5).should eq 95...100
    Gallery::Step.window(1, 3, 13).should eq 0...3
  end
end
