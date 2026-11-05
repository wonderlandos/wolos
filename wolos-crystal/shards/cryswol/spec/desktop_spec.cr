# SPDX-License-Identifier: BSD-2-Clause
# Recent files, the colour picker and printing against spec/fixtures/desktop.json.
require "./spec_helper"

describe "the desktop's verbs" do
  it "adds a recent file and says when it was refused" do
    Wol::Spec.with_fake("desktop") do |ctx, fake|
      Wol::Recents.add(ctx, "/home/a/report.pdf")
      expect_raises(Wol::Unavailable, "cannot be added to the recent files") { Wol::Recents.add(ctx, "/home/a/gone.pdf") }
      expect_raises(Wol::InvalidArgument) { Wol::Recents.add(ctx, "report.pdf") }
      fake.call_tags.should eq ["recent.add", "recent.add"]
      fake.calls.map(&.["path"].as_s).should eq ["/home/a/report.pdf", "/home/a/gone.pdf"]
    end
  end

  it "picks a colour, or nil when the person cancelled" do
    Wol::Spec.with_fake("desktop") do |ctx, _fake|
      Wol::Colors.pick(ctx).should eq 0x0a1b2c_u32
      Wol::Colors.pick(ctx).should be_nil
      expect_raises(Wol::Unavailable, "not available") { Wol::Colors.pick(ctx) }
    end
  end

  it "prints a file" do
    Wol::Spec.with_fake("desktop") do |ctx, fake|
      Wol::Print.file(ctx, "/home/a/report.pdf").should eq "printed"
      Wol::Print.file(ctx, "/home/a/report.pdf").should eq "cancelled"
      expect_raises(Wol::PermissionDenied) { Wol::Print.file(ctx, "/home/a/report.pdf") }
      expect_raises(Wol::InvalidArgument) { Wol::Print.file(ctx, "/a/../b.pdf") }
      fake.call_tags.should eq ["print.file", "print.file", "print.file"]
    end
  end
end
