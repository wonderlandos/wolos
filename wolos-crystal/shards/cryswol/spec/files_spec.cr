# SPDX-License-Identifier: BSD-2-Clause
# Files the person picks and devices the app opens, against the fake wold scripted by
# spec/fixtures/files.json (it hands a real file over with each answer that has "fd").
require "./spec_helper"

describe "Wol::Panel and Wol::Device.open" do
  it "opens the picked file; to_io is a duplicate that survives close" do
    Wol::Spec.with_fake("files") do |ctx, fake|
      file = Wol::Panel.open(ctx, "Pick a note").should_not be_nil
      file.name.should eq "a.txt"
      io = file.to_io
      file.close
      expect_raises(Wol::UsageError) { file.fd }
      begin
        io.gets_to_end.should eq "from the panel"
      ensure
        io.close
      end
      fake.calls[0]["title"].as_s.should eq "Pick a note"
    end
  end

  it "gives nil when the person cancels" do
    Wol::Spec.with_fake("files") do |ctx, _fake|
      Wol::Panel.open(ctx).try(&.close)
      Wol::Panel.open(ctx).should be_nil
    end
  end

  it "saves into the file the panel hands over" do
    Wol::Spec.with_fake("files") do |ctx, fake|
      file = Wol::Panel.save(ctx, "out.txt", "Save").should_not be_nil
      file.name.should eq "out.txt"
      io = file.to_io
      io << "written"
      io.close
      file.read.should eq ""
      file.close
      fake.calls[0]["name"].as_s.should eq "out.txt"
    end
  end

  it "refuses a file name with a folder before asking wold" do
    Wol::Spec.with_fake("files") do |ctx, fake|
      expect_raises(Wol::InvalidArgument) { Wol::Panel.save(ctx, "a/b.txt") }
      fake.call_tags.should be_empty
    end
  end

  it "opens a device, and maps a missing grant" do
    Wol::Spec.with_fake("files") do |ctx, fake|
      dev = Wol::Device.open(ctx, 0x1234, 0x5678)
      dev.name.should eq "Scanner"
      dev.read.should eq "usb"
      dev.close
      expect_raises(Wol::PermissionDenied) { Wol::Device.open(ctx, 0x1234, 0x5678) }
      fake.calls[0]["vendor"].as_i.should eq 0x1234
      fake.calls[0]["product"].as_i.should eq 0x5678
    end
  end
end
