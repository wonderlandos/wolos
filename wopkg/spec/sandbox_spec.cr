require "spec"
require "../src/wopkg/sandbox"

describe Wopkg::Sandbox do
  it "isolates builds: no network unless fetching, only the build folder writable" do
    a = Wopkg::Sandbox.argv("/c/b", "make", net: false, memory: false)
    a.first.should eq("bwrap")
    a.should contain("--unshare-all")
    a.should_not contain("--share-net")
    a.each_cons(3).to_a.should contain(["--bind", "/c/b", "/work"])
    a.each_cons(3).to_a.should contain(["--ro-bind", "/usr", "/usr"])
    a.join(" ").should_not contain(ENV["HOME"])
    a.last(3).should eq(["sh", "-ec", "make"])
    Wopkg::Sandbox.argv("/c/b", "cargo fetch --locked", net: true, memory: false).should contain("--share-net")
  end

  it "really has no network and no home inside", tags: "sandbox" do
    next unless Process.find_executable("bwrap")
    dir = File.tempname("wopkg-sb")
    Dir.mkdir_p("#{dir}/src")
    log = "#{dir}/log"
    ok = Wopkg::Sandbox.run(dir, "test ! -e #{ENV["HOME"]}/.bashrc && ! getent hosts example.org >/dev/null && echo inside > /work/src/out", false, log)
    File.read(log).should contain("no network")
    ok.should be_true
    File.read("#{dir}/src/out").should eq("inside\n")
  end
end
