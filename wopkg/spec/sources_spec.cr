require "spec"
require "../src/wopkg/sources"

describe Wopkg::Sources do
  conf = <<-CONF
    [options]
    Architecture = auto
    [wolos]
    Include = /etc/pacman.d/wolos-mirrorlist
    #[wolos-cachy]
    #Include = /etc/pacman.d/wolos-mirrorlist
    [core]
    Server = https://archive.archlinux.org/repos/2026/10/20/$repo/os/$arch
    [extra]
    Server = https://archive.archlinux.org/repos/2026/10/20/$repo/os/$arch
    CONF
  mirrors = "# primary\nServer = https://github.com/wonderlandos/wolos/releases/download/$repo-2026-10-20\n"

  it "reads repos, servers and included mirrorlists; commented repos stay off" do
    repos = Wopkg::Sources.pacman_repos(conf, ->(p : String) { p.ends_with?("mirrorlist") ? mirrors : nil })
    repos.map(&.name).should eq(%w(wolos core extra))
    repos[0].servers.should eq(["https://github.com/wonderlandos/wolos/releases/download/wolos-2026-10-20"])
    repos[1].servers.should eq(["https://archive.archlinux.org/repos/2026/10/20/core/os/x86_64"])
  end

  it "builds the index request, filtered by --only" do
    repos = [Wopkg::Sources::Repo.new("core", ["https://m/core/os/x86_64"])]
    j = JSON.parse(Wopkg::Sources.request(repos, nil, false))
    j["prune"].as_bool.should be_true
    j["sources"][0]["url"].should eq("https://m/core/os/x86_64/core.db")
    j["sources"].as_a.map(&.["kind"].as_s).should eq(%w(pacman aur appimage wol))
    k = JSON.parse(Wopkg::Sources.request(repos, nil, false, ["aur"]))
    k["prune"].as_bool.should be_false
    k["sources"].as_a.map(&.["name"].as_s).should eq(["aur"])
  end
end
