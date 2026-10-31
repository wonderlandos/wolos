require "spec"
require "file_utils"
require "../src/wopkg/aur"

describe Wopkg::Aur do
  it "finds symbolic links that could point a PKGBUILD at your files" do
    d = File.tempname("wopkg-ln")
    Dir.mkdir_p("#{d}/sub")
    File.write("#{d}/PKGBUILD", "x")
    Wopkg::Aur.symlinks(d).should be_empty
    File.symlink("/etc/passwd", "#{d}/sub/a.install")
    Wopkg::Aur.symlinks(d).should eq(["sub/a.install"])
    Wopkg::Aur.review_files(d).should eq(["PKGBUILD"])
    FileUtils.rm_rf(d)
  end

  it "remembers where an installed package came from, and ignores a damaged record" do
    d = File.tempname("wopkg-prov")
    a = Wopkg::Aur.new("#{d}/c", d)
    a.installed_commit("x").should be_nil
    Dir.mkdir_p("#{d}/aur-provenance")
    File.write(a.provenance_path("x"), %({"commit":"#{"b" * 40}"}))
    File.write(a.provenance_path("y"), %({"commit":"--evil"}))
    a.installed_commit("x").should eq("b" * 40)
    a.installed_commit("y").should be_nil
    a.installed.should eq(["x", "y"])
    FileUtils.rm_rf(d)
  end

  it "shows the whole package the first time and the diff since the installed commit after" do
    d = File.tempname("wopkg-rev")
    a = Wopkg::Aur.new("#{d}/c", d)
    r = a.repo("p")
    Dir.mkdir_p(r)
    git = ->(args : Array(String)) { Process.run("git", ["-C", r, "-c", "user.name=t", "-c", "user.email=t@t"] + args, output: Process::Redirect::Close).success? }
    git.call(["init", "-q"])
    File.write("#{r}/PKGBUILD", "pkgver=1\n")
    git.call(["add", "."]); git.call(["commit", "-qm", "1"])
    c1 = Wopkg::AurGuard.head(r)
    a.review_text("p", c1).should contain("==> PKGBUILD\npkgver=1")
    File.write("#{r}/PKGBUILD", "pkgver=2\n")
    git.call(["commit", "-qam", "2"])
    c2 = Wopkg::AurGuard.head(r)
    Dir.mkdir_p("#{d}/aur-provenance")
    File.write(a.provenance_path("p"), %({"commit":"#{c1}"}))
    a.review_text("p", c2).should contain("+pkgver=2")
    File.write(a.provenance_path("p"), %({"commit":"#{c2}"}))
    a.review_text("p", c2).should eq("")
    FileUtils.rm_rf(d)
  end
end