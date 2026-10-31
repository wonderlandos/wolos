require "spec"
require "file_utils"
require "../src/wopkg/aur_hook"

include Wopkg

def fake_analyser(dir : String, body : String) : String
  path = "#{dir}/wopkg-aurlaws"
  File.write(path, "#!/bin/sh\n#{body}\n")
  File.chmod(path, 0o755)
  path
end

describe Wopkg::AurHook do
  it "is a single hook that gets the package folder, the built file and the trace" do
    d = File.tempname("wopkg-hook")
    Dir.mkdir_p(d)
    bin = fake_analyser(d, %(echo "$@" > #{d}/args; echo '{"verdict":"pass","evidence":[]}'))
    v = AurHook.check("/pkg", "/pkg/x.pkg.tar.zst", "/pkg/.trace", analyser: bin)
    v.pass?.should be_true
    File.read("#{d}/args").strip.should eq("--dir /pkg --artifact /pkg/x.pkg.tar.zst --trace /pkg/.trace --json")
    FileUtils.rm_rf(d)
  end

  it "returns the analyser's flag or block with its evidence" do
    d = File.tempname("wopkg-hook")
    Dir.mkdir_p(d)
    bin = fake_analyser(d, %(echo '{"verdict":"block","evidence":["the build connects to 185.1.2.3","it writes ~/.ssh"]}'))
    v = AurHook.check("/p", "/a", nil, analyser: bin)
    v.block?.should be_true
    v.evidence.should eq(["the build connects to 185.1.2.3", "it writes ~/.ssh"])
    FileUtils.rm_rf(d)
  end

  it "says not checked, never pass, when the analyser is missing, fails or talks nonsense" do
    AurHook.check("/p", "/a", nil, analyser: nil).unavailable?.should be_true
    d = File.tempname("wopkg-hook")
    Dir.mkdir_p(d)
    AurHook.check("/p", "/a", nil, analyser: fake_analyser(d, "exit 3")).unavailable?.should be_true
    AurHook.check("/p", "/a", nil, analyser: fake_analyser(d, %(echo '{"verdict":"fine"}'))).unavailable?.should be_true
    AurHook.check("/p", "/a", nil, analyser: fake_analyser(d, "echo nope")).unavailable?.should be_true
    AurHook.check("/p", "/a", nil, analyser: nil).evidence.first.should contain("not checked")
    FileUtils.rm_rf(d)
  end

  it "audits the folder alone, asks for the graph, and hands back the analyser's own words" do
    d = File.tempname("wopkg-hook")
    Dir.mkdir_p(d)
    bin = fake_analyser(d, %(echo "$@" > #{d}/args; echo '{"verdict":"flag","evidence":["curl runs it"]}'))
    a = AurHook.audit("/pkg", "/out/x.svg", analyser: bin)
    a.verdict.flag?.should be_true
    a.raw.should contain("curl runs it")
    File.read("#{d}/args").strip.should eq("--dir /pkg --graph /out/x.svg --json")
    FileUtils.rm_rf(d)
  end

  it "says what an audit means and never more" do
    AurHook.summary(AurHook::Verdict.new(AurHook::Kind::Pass, [] of String)).should eq("No law broken. That does not mean the package is safe.")
    AurHook.summary(AurHook::Verdict.new(AurHook::Kind::Flag, [] of String)).should eq("Needs review.")
    AurHook.summary(AurHook::Verdict.new(AurHook::Kind::Unavailable, [] of String)).should eq("Not checked.")
  end
end
