# SPDX-License-Identifier: GPL-2.0-or-later
require "spec"
require "file_utils"
require "../src/wopkg/shard"

# `wopkg shard` on libwopkg 1.1 with the signed test list in spec/fixtures/shards (written by
# `cargo test -p wopkg --lib index::shards`, signed by a test-only key), and, when git and shards
# are installed, a real `shards install --frozen --local` against a primed cache.
FIXTURE = File.expand_path("fixtures/shards", __DIR__)

APP_YML = <<-Y
  name: demo
  version: 0.1.0
  crystal: ">= 1.22.0"
  license: MIT

  targets:
    demo:
      main: src/demo.cr

  Y

private def project(tmp : String) : {Wopkg::Core, String}
  Dir.mkdir_p("#{tmp}/cache/shard-index")
  {"index.json", "index.json.sig"}.each { |f| FileUtils.cp("#{FIXTURE}/#{f}", "#{tmp}/cache/shard-index/#{f}") }
  Dir.mkdir_p("#{tmp}/demo")
  File.write("#{tmp}/demo/shard.yml", APP_YML)
  core = Wopkg::Core.new({"home" => tmp, "cache_dir" => "#{tmp}/cache", "state_dir" => "#{tmp}/state", "keys_dir" => "#{FIXTURE}/keys"}.to_json)
  {core, "#{tmp}/demo"}
end

private def shard(core : Wopkg::Core, argv : Array(String)) : {Int32, String}
  io = IO::Memory.new
  code = Wopkg::Shard.new(core, Wopkg::Args.parse(["shard"] + argv), io).run
  {code, io.to_s}
end

private def git(args : Array(String), dir : String? = nil) : Nil
  st = Process.run("git", args, chdir: dir, env: {"GIT_AUTHOR_NAME" => "t", "GIT_AUTHOR_EMAIL" => "t@t", "GIT_COMMITTER_NAME" => "t", "GIT_COMMITTER_EMAIL" => "t@t"}, error: Process::Redirect::Inherit)
  raise "git #{args.join(" ")} failed" unless st.success?
end

private def git_out(args : Array(String), dir : String) : String
  io = IO::Memory.new
  Process.run("git", args, chdir: dir, output: io)
  io.to_s.strip
end

describe Wopkg::Shard do
  it "splits words into one project folder and shard names" do
    Wopkg::Shard.split(["wolkit", "./app"]).should eq({File.expand_path("./app"), ["wolkit"]})
    Wopkg::Shard.split(["wolkit"]).should eq({Dir.current, ["wolkit"]})
    expect_raises(Wopkg::UsageError) { Wopkg::Shard.split(["./a", "./b"]) }
  end

  it "finds the mirror where shards keeps it" do
    Wopkg::Shard.cache_path("https://git.wolos.org/shards/wolkit.git", "/c").should eq("/c/git.wolos.org/shards/wolkit.git")
    Wopkg::Shard.cache_path("https://git.wolos.org/shards/wolkit", "/c").should eq("/c/git.wolos.org/shards/wolkit.git")
    Wopkg::Shard.cache_path("file:///srv/dep", "/c").should eq("/c/srv/dep.git")
  end

  it "lists, adds with the lock and checks clean against the signed list" do
    tmp = File.tempname("wopkg-shard")
    core, dir = project(tmp)
    code, out = shard(core, ["list"])
    code.should eq(0)
    out.should contain("wolkit")
    out.should contain("cryswol")
    shard(core, ["list", "nothing-like-it"])[1].should contain("No shard")
    code, out = shard(core, ["add", "wolkit", dir, "--local"])
    code.should eq(0)
    out.should contain("Pinned wolkit 0.3.0 at 111111111111.")
    yml = File.read("#{dir}/shard.yml")
    yml.should start_with(APP_YML)
    yml.should contain("    commit: #{"1" * 40}\n")
    lock = File.read("#{dir}/shard.lock")
    lock.should contain("  cryswol:\n    git: https://git.wolos.org/shards/cryswol.git\n    version: 0.3.0+git.commit.#{"2" * 40}\n")
    shard(core, ["add", "wolkit", dir, "--local"])[1].should contain("already at the commit")
    code, out = shard(core, ["check", dir])
    {code, out.includes?("match the WolOS shard list")}.should eq({0, true})
    json = Wopkg::Shard::Checked.from_json(core.shard_check({dir: dir}.to_json))
    {json.ok?, json.index?}.should eq({true, true})
    err = expect_raises(Wopkg::Failure) { shard(core, ["add", "nope", dir, "--local"]) }
    err.kind.should eq("not-found")
    FileUtils.rm_rf(tmp)
  end

  it "fails check on a dependency that is not pinned, and needs no list offline" do
    tmp = File.tempname("wopkg-shard")
    core, dir = project(tmp)
    File.write("#{dir}/shard.yml", APP_YML + "dependencies:\n  wolkit:\n    github: someone/wolkit\n    version: ~> 0.2\n")
    code, out = shard(core, ["check", dir, "--local"])
    code.should eq(1)
    out.should contain("RCR-040")
    out.should contain("RCR-041")
    out.should contain("only RCR-040 and RCR-041")
    expect_raises(Wopkg::UsageError) { shard(core, ["frobnicate"]) }
    FileUtils.rm_rf(tmp)
  end

  it "primes the cache so shards install --frozen --local needs no network" do
    unless Process.find_executable("git") && Process.find_executable("shards")
      pending!("git and shards are not installed")
    end
    tmp = File.tempname("wopkg-shard-e2e")
    Dir.mkdir_p("#{tmp}/dep/src")
    File.write("#{tmp}/dep/shard.yml", "name: dep\nversion: 0.2.0\ncrystal: \">= 1.12.0\"\nlicense: MIT\n")
    File.write("#{tmp}/dep/src/dep.cr", "# SPDX-License-Identifier: MIT\nmodule Dep\nend\n")
    git(["init", "--quiet", "-b", "main"], "#{tmp}/dep")
    git(["add", "."], "#{tmp}/dep")
    git(["commit", "--quiet", "-m", "one"], "#{tmp}/dep")
    sha = git_out(["rev-parse", "HEAD"], "#{tmp}/dep")
    url = "file://#{tmp}/dep"
    cache = "#{tmp}/shards-cache"
    Wopkg::Shard.prime(url, sha, cache).should be_true
    Wopkg::Shard.prime(url, "f" * 40, cache).should be_false
    Dir.mkdir_p("#{tmp}/app")
    File.write("#{tmp}/app/shard.yml", APP_YML + "dependencies:\n  dep:\n    git: #{url}\n    commit: #{sha}\n")
    File.write("#{tmp}/app/shard.lock", "version: 2.0\nshards:\n  dep:\n    git: #{url}\n    version: 0.2.0+git.commit.#{sha}\n")
    # The remote goes away: only the primed mirror can serve the install.
    FileUtils.mv("#{tmp}/dep", "#{tmp}/gone")
    io = IO::Memory.new
    st = Process.run("shards", ["install", "--frozen", "--local"], chdir: "#{tmp}/app", env: {"SHARDS_CACHE_PATH" => cache}, output: io, error: io)
    st.success?.should be_true, io.to_s
    File.exists?("#{tmp}/app/lib/dep/src/dep.cr").should be_true
    FileUtils.rm_rf(tmp)
  end
end
