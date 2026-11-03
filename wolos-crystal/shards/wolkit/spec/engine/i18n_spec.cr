# SPDX-License-Identifier: BSD-2-Clause
# The locale system through libwolkit (ADR 0045): tables made by `wol-i18n compile` from
# spec/fixtures/i18n/demo are read by key name, language and plural count, English fills what a
# language lacks, and `t` in a view uses them. (crates/wolkit/tests/catalog.rs keeps the .wtx files
# in spec/fixtures/i18n/tables equal to what the compiler makes of the .tsv files.)
require "./engine_helper_spec"
require "../../src/wolkit/ffi/catalog"

private TABLES = File.join(__DIR__, "..", "fixtures", "i18n", "tables")

describe Wolkit::FFI::Catalog do
  it "loads a directory of tables and answers by key, language and count" do
    catalog = Wolkit::FFI::Catalog.load_dir(TABLES, "demo")
    catalog.count.should eq 4
    title = catalog.find("sound.title").not_nil!
    catalog.text(title, LibWolkit::WOLKIT_LANG_EN).should eq "Sound"
    catalog.text(title, LibWolkit::WOLKIT_LANG_DE).should eq "Ton"
    louder = catalog.find("sound.louder").not_nil!
    catalog.text(louder, LibWolkit::WOLKIT_LANG_DE).should eq "Lets volume sliders pass 100%."
    files = catalog.find("files.count").not_nil!
    catalog.plural(files, 1_u64, LibWolkit::WOLKIT_LANG_DE).should eq "{n} Datei"
    catalog.plural(files, 3_u64, LibWolkit::WOLKIT_LANG_DE).should eq "{n} Dateien"
    catalog.find("no.such.key").should be_nil
    catalog.close
  end

  it "loads tables held in memory" do
    tables = ["en", "de"].map { |l| File.open(File.join(TABLES, "demo.#{l}.wtx"), &.getb_to_end) }
    catalog = Wolkit::FFI::Catalog.load(tables)
    catalog.text(catalog.find("sound.title").not_nil!, LibWolkit::WOLKIT_LANG_DE).should eq "Ton"
    catalog.close
  end

  it "raises with a sentence when the English table is missing" do
    error = expect_raises(Wol::Error) { Wolkit::FFI::Catalog.load_dir(TABLES, "other") }
    error.message.to_s.should contain "English text"
  end
end

describe Wolkit::FFI::Language do
  it "maps tags and locales to ids like Wolkit::I18n does" do
    ["pt-BR", "pt_PT.UTF-8", "zh_CN.UTF-8", "xx", "ko_KR.UTF-8", "it_IT.UTF-8", "zh_TW"].each do |name|
      Wolkit::FFI::Language.parse(name).should eq Wolkit::I18n.parse(name)
    end
    Wolkit::FFI::Language.tag(7).should eq "zh-Hans"
    Wolkit::FFI::Language.tag(40).should be_nil
  end
end

describe Wolkit::I18n do
  after_each { Wolkit::I18n.language = "en" }

  it "uses the catalog for t in a view" do
    Wolkit::I18n.load(TABLES, "demo")
    Wolkit::I18n.t("sound.title").should eq "Sound"
    Wolkit::I18n.language = "de"
    Wolkit::I18n.t("sound.title").should eq "Ton"
    Wolkit::I18n.t("files.count", n: 2).should eq "2 Dateien"
    Wolkit::I18n.t("net.join", name: "Rabbit Hole").should eq "Join “Rabbit Hole”."
    Wolkit::I18n.t("missing.key").should eq "missing.key"
  end
end
