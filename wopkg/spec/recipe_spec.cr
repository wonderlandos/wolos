# SPDX-License-Identifier: GPL-2.0-or-later
require "spec"
require "file_utils"
require "../src/wopkg/recipe"

SUM = "3fafa02949c005e70869074ac9db489ad92eaf92c78b4dcf6c0b45d98982c08d"

def recipe(extra = "", fetch = %(["cargo fetch --locked"]))
  <<-T
    name = "hvm"
    version = "2.0.22"
    license = "Apache-2.0"
    needs = ["cargo"]
    [[source]]
    url = "https://static.crates.io/crates/hvm/hvm-2.0.22.crate"
    sha256 = "#{SUM}"
    #{extra}
    [fetch]
    run = #{fetch}
    [build]
    run = ["cargo build --release --locked --offline"]
    [install]
    bin = ["target/release/hvm"]
    T
end

describe Wopkg::Recipe do
  it "reads a recipe" do
    r = Wopkg::Recipe.parse(recipe)
    {r.name, r.version, r.sources[0].file, r.fetch, r.bins}.should eq({"hvm", "2.0.22", "hvm-2.0.22.crate", ["cargo fetch --locked"], ["target/release/hvm"]})
    r.key.size.should eq(16)
    Wopkg::Recipe.parse(recipe).key.should eq(r.key)
    Wopkg::Recipe.parse(recipe("file = \"x.tar.gz\"")).key.should_not eq(r.key)
  end

  it "refuses unpinned, insecure or networked build steps" do
    {
      recipe.sub(SUM, "abc")                                  => /64 hex/,
      recipe.sub("https://static", "http://static")           => /HTTPS only/,
      recipe(fetch: %(["curl https://evil"]))                 => /fetch steps may only run/,
      recipe(fetch: %(["cargo fetch --locked; curl x"]))      => /one command/,
      recipe.sub(%("target/release/hvm"), %("../../.bashrc")) => /inside the build/,
      recipe.sub(%(name = "hvm"), %(name = "HVM!"))           => /name is lowercase/,
      recipe("file = \"../x\"")                               => /bad source file/,
    }.each do |text, err|
      expect_raises(Wopkg::Recipe::Invalid, err) { Wopkg::Recipe.parse(text) }
    end
  end

  it "every shipped recipe is valid and the index lists them" do
    dir = "#{__DIR__}/../recipes"
    idx = JSON.parse(Wopkg::Recipe.index(dir))
    names = idx["recipes"].as_a.map(&.["name"].as_s)
    names.should eq(names.sort)
    names.should contain("hvm")
    names.should contain("bend")
    names.should contain("zenc")
    names.should contain("craft-launcher")
  end

  # The Store's Craft Suite kit (crates/core/src/store.rs, KitSource::Wopkg("craft-launcher")) is
  # shown only when this recipe ships; it must stay a signed, pinned, user-side AppImage install.
  it "craft-launcher installs the signed launcher AppImage for this account" do
    r = Wopkg::Recipe.parse(File.read("#{__DIR__}/../recipes/craft-launcher/recipe.toml"))
    r.name.should eq("craft-launcher")
    r.sources.map(&.url).each(&.should(start_with("https://github.com/storytold/craft-launcher/releases/download/v#{r.version}/")))
    r.sources.map(&.file).should contain("artcraft-launcher-#{r.version}-linux-x86_64.AppImage")
    r.fetch.should be_empty
    steps = r.build.join("\n")
    steps.should contain("--list-keys 2B76741B414AAD6CD33E631213A005C0B9D7C3ED")
    steps.should contain("--verify SHA256SUMS.txt.asc SHA256SUMS.txt")
    steps.should contain("--verify artcraft-launcher-#{r.version}-linux-x86_64.AppImage.asc")
    steps.should contain("sha256sum -c -")
    steps.should_not contain("pkexec")
    steps.should_not contain("sudo")
    r.install.join("\n").should contain("$PREFIX/bin/artcraft-launcher")
  end

  it "installs build output atomically into a prefix" do
    d = File.tempname("wopkg-ri")
    Dir.mkdir_p("#{d}/out/bin")
    File.write("#{d}/out/bin/hvm", "x")
    files = Wopkg::RecipeBuilder.install("#{d}/out", "#{d}/prefix")
    files.should eq(["#{d}/prefix/bin/hvm"])
    File.read("#{d}/prefix/bin/hvm").should eq("x")
    FileUtils.rm_rf(d)
  end
end
