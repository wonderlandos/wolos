require "spec"
require "file_utils"
require "../src/wopkg/core"

# Crystal on libwopkg (`make spec` links the freshly built library).
describe Wopkg::Core do
  it "indexes local sources, searches them, completes names and reports errors" do
    d = File.tempname("wopkg-core")
    Dir.mkdir_p("#{d}/db/vim-9.1-1")
    File.write("#{d}/db/vim-9.1-1/desc", "%NAME%\nvim\n\n%VERSION%\n9.1-1\n\n%DESC%\nVi Improved, a text editor\n")
    Process.run("bsdtar", ["-czf", "#{d}/extra.db", "-C", "#{d}/db", "vim-9.1-1"]).success?.should be_true
    core = Wopkg::Core.new({"home" => d, "cache_dir" => "#{d}/cache", "state_dir" => "#{d}/state", "pacman_local" => "#{d}/none"}.to_json)
    events = 0
    req = {"sources" => [{"name" => "extra", "kind" => "pacman", "path" => "#{d}/extra.db"}]}.to_json
    r = core.index_update(req, ->(_e : JSON::Any) { events += 1; nil })
    r["sources"][0]["records"].should eq(1)
    events.should be > 0
    hit = core.search("text editor")["results"][0]
    {hit["key"], hit["sources"][0]["repo"]}.should eq({"vim", "extra"})
    core.complete("vi").should eq("vim")
    core.history["entries"].as_a.should be_empty
    id = core.record({"source" => "flatpak", "action" => "install", "name" => "org.x.Y", "undo" => {"type" => "run", "argv" => ["true"]}}.to_json)
    core.rollback(id)["undo"]["type"].should eq("run")
    err = expect_raises(Wopkg::Failure) { core.info("nothing-like-this") }
    err.kind.should eq("not-found")
    expect_raises(Wopkg::Failure, /not a package name/) { core.system_call(%({"call":"package.change","verb":"install","names":["-Syu"]})) }
    FileUtils.rm_rf(d)
  end
end
