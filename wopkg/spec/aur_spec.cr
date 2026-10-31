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
end
