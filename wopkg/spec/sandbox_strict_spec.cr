require "spec"
require "../src/wopkg/sandbox"

describe Wopkg::Sandbox, "untrusted builds" do
  it "builds as nobody in a user namespace with no capabilities, home, keys or bus" do
    a = Wopkg::Sandbox.argv("/c/b", "makepkg", net: false, memory: false, strict: true)
    pairs = a.each_cons(3).to_a
    pairs.should contain(["--uid", "65534", "--gid"])
    a.should contain("--unshare-user")
    pairs.should contain(["--cap-drop", "ALL", "--setenv"]) # before the environment
    a.should_not contain("--share-net")
    pairs.should_not contain(["--ro-bind", "/etc", "/etc"])
    j = a.join(" ")
    j.should_not contain(ENV["HOME"])
    j.should_not contain("/run/user")
    j.should_not contain("DBUS")
    j.should_not contain("SSH_AUTH_SOCK")
    j.should contain("--ro-bind /etc/makepkg.conf /etc/makepkg.conf")
    j.should contain("--ro-bind-try /etc/ssl /etc/ssl")
    j.should_not contain("--ro-bind /etc/shadow")
  end

  it "keeps the network for the fetch step only and says so" do
    Wopkg::Sandbox.argv("/c/b", "makepkg --verifysource", net: true, memory: false, strict: true).should contain("--share-net")
    Wopkg::Sandbox.describe(net: true).should contain("needs the network")
    Wopkg::Sandbox.describe(net: false).should contain("no network")
  end

  it "can record the system calls of a build for the package analyser" do
    a = Wopkg::Sandbox.argv("/c/b", "makepkg", net: false, memory: false, strict: true, trace: "/work/.trace")
    cmd = a.last
    cmd.should start_with("strace -f -qq")
    cmd.should contain("-o /work/.trace")
    cmd.should contain("makepkg")
    Wopkg::Sandbox.argv("/c/b", "makepkg", net: false, memory: false, strict: true).last.should eq("makepkg")
  end

  it "really runs as nobody, without the caller's home, in a real sandbox", tags: "sandbox" do
    next unless Process.find_executable("bwrap")
    dir = File.tempname("wopkg-strict")
    Dir.mkdir_p("#{dir}/src")
    log = "#{dir}/log"
    ok = Wopkg::Sandbox.run(dir, "test \"$(id -u)\" = 65534 && test ! -e #{ENV["HOME"]}/.bashrc && echo nobody > /work/src/out", false, log, strict: true)
    next unless ok || !File.read(log).includes?("Operation not permitted") # no user namespaces here
    ok.should be_true
    File.read("#{dir}/src/out").should eq("nobody\n")
  end
end