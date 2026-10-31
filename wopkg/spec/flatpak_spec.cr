require "spec"
require "../src/wopkg/flatpak"

describe Wopkg::Flatpak do
  it "installs and updates for the person only" do
    Wopkg::Flatpak.install_argv("flathub", "org.gimp.GIMP").should contain("--user")
    Wopkg::Flatpak.update_argv.should contain("--user")
    Wopkg::Flatpak.rollback_argv("org.gimp.GIMP", "abc").should eq(%w(flatpak update --user --noninteractive -y --commit=abc org.gimp.GIMP))
  end

  it "finds remotes without signature checks" do
    text = "flathub\tsystem\nevil\tuser,no-gpg-verify\nother\t\n"
    Wopkg::Flatpak.unsigned_remotes(text).should eq(["evil"])
    Wopkg::Flatpak.ids("Application ID\norg.a.B\n\norg.c.D\n").should eq(["org.a.B", "org.c.D"])
  end
end
