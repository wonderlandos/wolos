# SPDX-License-Identifier: GPL-2.0-or-later
require "spec"
require "../src/wopkg/app"

# QA-STORE-62: root's home is in the sealed slot; its cache and state are under /var.
describe "Wopkg::App.dir" do
  it "puts root's folders under /var, an XDG folder first, a person's in the home" do
    Wopkg::App.dir(nil, true, Wopkg::App::ROOT_STATE) { "/root/.local/state/wopkg" }.should eq("/var/lib/wolos/wopkg/state")
    Wopkg::App.dir("", true, Wopkg::App::ROOT_CACHE) { "/root/.cache/wopkg" }.should eq("/var/cache/wopkg")
    Wopkg::App.dir("/x", true, Wopkg::App::ROOT_CACHE) { "/root/.cache/wopkg" }.should eq("/x/wopkg")
    Wopkg::App.dir(nil, false, Wopkg::App::ROOT_CACHE) { "/home/ana/.cache/wopkg" }.should eq("/home/ana/.cache/wopkg")
  end
end

# QA-STORE-68: one channel in 0.13; a file left from before naming another reads as stable.
describe "Wopkg::App.channel_of" do
  it "reads stable whatever the file says" do
    Wopkg::App.channel_of(nil).should eq("stable")
    Wopkg::App.channel_of("stable\n").should eq("stable")
    Wopkg::App.channel_of("beta\n").should eq("stable")
  end
end
