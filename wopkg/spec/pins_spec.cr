# SPDX-License-Identifier: GPL-2.0-or-later
require "spec"
require "../src/wopkg/pins"

describe Wopkg::Pins do
  it "keeps each name once and forgets it on request" do
    p = File.tempname("wopkg-pins")
    Wopkg::Pins.read(p).should eq([] of String)
    Wopkg::Pins.set(p, "gimp", true)
    Wopkg::Pins.set(p, "firefox", true)
    Wopkg::Pins.set(p, "gimp", true)
    Wopkg::Pins.read(p).should eq(%w(firefox gimp))
    Wopkg::Pins.set(p, "gimp", false)
    Wopkg::Pins.read(p).should eq(%w(firefox))
    File.delete(p)
  end
end
