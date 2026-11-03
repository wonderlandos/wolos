# SPDX-License-Identifier: BSD-2-Clause
# The locale system for apps (ADR 0045) without the engine: `t` reads the language signal, falls
# back to English, fills placeholders and plurals, and names a missing key instead of raising.
require "./spec_helper"

private def demo : Wolkit::I18n::Table
  Wolkit::I18n::Table.new({
    "en" => {
      "sound.title"       => "Sound",
      "sound.louder"      => "Lets volume sliders pass 100%.",
      "files.count#one"   => "{n} file",
      "files.count#other" => "{n} files",
      "net.join"          => "Join “{name}”.",
    },
    "de" => {
      "sound.title"       => "Ton",
      "files.count#one"   => "{n} Datei",
      "files.count#other" => "{n} Dateien",
    },
  })
end

describe Wolkit::I18n do
  before_each do
    Wolkit::I18n.use(demo)
    Wolkit::I18n.language = "en"
  end

  it "answers in the chosen language and falls back to English" do
    Wolkit::I18n.t("sound.title").should eq "Sound"
    Wolkit::I18n.language = "de"
    Wolkit::I18n.t("sound.title").should eq "Ton"
    Wolkit::I18n.t("sound.louder").should eq "Lets volume sliders pass 100%."
  end

  it "fills placeholders and picks plural forms" do
    Wolkit::I18n.t("net.join", name: "Rabbit Hole").should eq "Join “Rabbit Hole”."
    Wolkit::I18n.t("files.count", n: 1).should eq "1 file"
    Wolkit::I18n.t("files.count", n: 3).should eq "3 files"
    Wolkit::I18n.language = "de"
    Wolkit::I18n.t("files.count", n: 3).should eq "3 Dateien"
  end

  it "names a missing key and counts it, never raising" do
    before = Wolkit::I18n.missing
    Wolkit::I18n.t("no.such.key").should eq "no.such.key"
    Wolkit::I18n.missing.should eq before + 1
  end

  it "keeps the String of a label: a second read is the same object" do
    a = Wolkit::I18n.t("sound.title")
    Wolkit::I18n.t("sound.title").object_id.should eq a.object_id
  end

  it "maps POSIX locales and tags to languages like libwolkit" do
    Wolkit::I18n.parse("pt_BR.UTF-8").should eq 1
    Wolkit::I18n.parse("pt_PT.UTF-8").should eq 1
    Wolkit::I18n.parse("zh_CN.UTF-8").should eq 7
    Wolkit::I18n.parse("zh_TW.UTF-8").should be_nil
    Wolkit::I18n.parse("fil_PH.UTF-8").should eq 9
    Wolkit::I18n.parse("C.UTF-8").should eq 0
    Wolkit::I18n.parse("zh-Hans").should eq 7
    Wolkit::I18n.parse("it_IT.UTF-8").should be_nil
    Wolkit::I18n.language = "it_IT.UTF-8"
    Wolkit::I18n.language.should eq "en"
  end

  it "relabels an effect when the language changes" do
    scope = Wolkit::Scope.new(nil)
    labels = [] of String
    scope.run { Wolkit.effect { labels << Wolkit::I18n.t("sound.title") } }
    Wolkit::I18n.language = "de"
    labels.should eq ["Sound", "Ton"]
  end

  it "gives views a t helper" do
    view = I18nView.new
    view.label.should eq "Sound"
    Wolkit::I18n.language = "de"
    view.label.should eq "Ton"
    view.count(2).should eq "2 Dateien"
  end
end

class I18nView
  include Wolkit::DSL

  def label : String
    t("sound.title")
  end

  def count(n : Int32) : String
    t("files.count", n: n)
  end
end
