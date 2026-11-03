# SPDX-License-Identifier: GPL-2.0-or-later
# Wol::Pkg (opt-in, GPL) on the freshly built libwopkg: a local pacman source indexed, searched and
# completed, history and rollback, and the error mapping, including the two statuses only libwopkg
# has (Untrusted, NotFound). Needs `bsdtar` for the test repository, as wopkg's own spec does.
require "./spec_helper"
require "../src/cryswol/pkg"

private def pkg_home : String
  d = File.tempname("cryswol-pkg")
  Dir.mkdir_p("#{d}/db/vim-9.1-1")
  File.write("#{d}/db/vim-9.1-1/desc", "%NAME%\nvim\n\n%VERSION%\n9.1-1\n\n%DESC%\nVi Improved, a text editor\n")
  Process.run("bsdtar", ["-czf", "#{d}/extra.db", "-C", "#{d}/db", "vim-9.1-1"]).success?.should be_true
  d
end

private def pkg_config(d : String) : String
  {"home" => d, "cache_dir" => "#{d}/cache", "state_dir" => "#{d}/state", "pacman_local" => "#{d}/none"}.to_json
end

describe Wol::Pkg do
  it "indexes a local source with progress, searches, completes and rolls back" do
    d = pkg_home
    pkg = Wol::Pkg::Context.new(pkg_config(d))
    events = [] of String
    req = {"sources" => [{"name" => "extra", "kind" => "pacman", "path" => "#{d}/extra.db"}]}.to_json
    report = JSON.parse(pkg.index_update(req) { |e| events << e })
    report["sources"][0]["records"].should eq 1
    events.should_not be_empty
    events.all? { |e| JSON.parse(e)["event"]?.try(&.as_s?) }.should be_true
    hit = JSON.parse(pkg.search("text editor"))["results"][0]
    hit["key"].should eq "vim"
    pkg.complete("vi").lines.should contain "vim"
    JSON.parse(pkg.history)["entries"].as_a.should be_empty
    entry = {"source" => "flatpak", "action" => "install", "name" => "org.x.Y", "undo" => {"type" => "run", "argv" => ["true"]}}.to_json
    id = pkg.history_record(entry)
    JSON.parse(pkg.rollback(id))["undo"]["type"].should eq "run"
    pkg.close
    pkg.closed?.should be_true
    expect_raises(Wol::UsageError) { pkg.installed }
  ensure
    FileUtils.rm_rf(d) if d
  end

  it "maps NotFound, Untrusted and InvalidArgument to Wol::Error classes" do
    d = pkg_home
    pkg = Wol::Pkg::Context.new(pkg_config(d))
    nf = expect_raises(Wol::Pkg::NotFound) { pkg.info("nothing-like-this") }
    nf.code.should eq LibWopkg::WOPKG_ERR_NOT_FOUND
    nf.should be_a Wol::Error
    File.write("#{d}/f.txt", "text")
    bad = {"kind" => "sha256", "path" => "#{d}/f.txt", "sha256" => "0" * 64}.to_json
    expect_raises(Wol::Pkg::Untrusted, "The file does not match its checksum.") { pkg.verify(bad) }
    expect_raises(Wol::InvalidArgument, /not a package name/) do
      pkg.system_call(%({"call":"package.change","verb":"install","names":["-Syu"]}))
    end
    expect_raises(Wol::InvalidArgument) { Wol::Pkg::Context.new(%({"no_such_key":1})) }
    pkg.close
  ensure
    FileUtils.rm_rf(d) if d
  end

  it "sends a raising progress block to on_error and finishes the call" do
    d = pkg_home
    pkg = Wol::Pkg::Context.new(pkg_config(d))
    req = {"sources" => [{"name" => "extra", "kind" => "pacman", "path" => "#{d}/extra.db"}]}.to_json
    errors = Wol::Spec.capture_errors do
      JSON.parse(pkg.index_update(req) { |_e| raise "boom" })["sources"][0]["records"].should eq 1
    end
    errors.should_not be_empty
    errors.first.message.should eq "boom"
    pkg.close
  ensure
    FileUtils.rm_rf(d) if d
  end

  it "is the libwopkg its generated declarations describe" do
    want = (LibWopkg::WOPKG_VERSION_MAJOR << 16) | (LibWopkg::WOPKG_VERSION_MINOR << 8) | LibWopkg::WOPKG_VERSION_PATCH
    Wol::Pkg.abi_version.should eq want
    Wol::Pkg.version.should eq "#{LibWopkg::WOPKG_VERSION_MAJOR}.#{LibWopkg::WOPKG_VERSION_MINOR}.#{LibWopkg::WOPKG_VERSION_PATCH}"
  end
end
