require "spec"
require "file_utils"
require "../src/wopkg/aur"

describe Wopkg::Aur do
  it "checks names and reads dependencies" do
    Wopkg::Aur.valid?("paru-bin").should be_true
    Wopkg::Aur.valid?("-x").should be_false
    Wopkg::Aur.valid?("../x").should be_false
    src = "pkgbase = x\n\tmakedepends = cargo\n\tdepends = gcc-libs>=13\n\tdepends = glibc\n\tprovides = x\n"
    Wopkg::Aur.depends(src).should eq(%w(cargo gcc-libs glibc))
  end

  it "shows the whole PKGBUILD first, then only what changed" do
    d = File.tempname("wopkg-aur")
    Dir.mkdir_p("#{d}/repo")
    File.write("#{d}/repo/PKGBUILD", "pkgname=x\npkgver=1\n")
    Wopkg::Aur.review_text("#{d}/repo", "#{d}/last").should contain("==> PKGBUILD\npkgname=x")
    FileUtils.cp_r("#{d}/repo", "#{d}/last")
    Wopkg::Aur.review_text("#{d}/repo", "#{d}/last").should eq("")
    File.write("#{d}/repo/PKGBUILD", "pkgname=x\npkgver=2\n")
    diff = Wopkg::Aur.review_text("#{d}/repo", "#{d}/last")
    diff.should contain("-pkgver=1")
    diff.should contain("+pkgver=2")
    FileUtils.rm_rf(d)
  end
end
