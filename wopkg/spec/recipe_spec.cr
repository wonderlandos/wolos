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
      recipe.sub(SUM, "abc")                                   => /64 hex/,
      recipe.sub("https://static", "http://static")            => /HTTPS only/,
      recipe(fetch: %(["curl https://evil"]))                  => /fetch steps may only run/,
      recipe(fetch: %(["cargo fetch --locked; curl x"]))       => /one command/,
      recipe.sub(%("target/release/hvm"), %("../../.bashrc"))  => /inside the build/,
      recipe.sub(%(name = "hvm"), %(name = "HVM!"))            => /name is lowercase/,
      recipe("file = \"../x\"")                                => /bad source file/,
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
