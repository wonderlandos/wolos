# SPDX-License-Identifier: BSD-2-Clause
# Services without a desktop: with no wold running every call comes back as a sentence, never an
# exception, and the panels give the sentence to their block on the UI context.
require "./engine_helper_spec"

describe Wolkit::Services do
  it "returns the sentence when the service is not there" do
    notice = Wolkit::Services.notify("Hello", "from a spec")
    notice.should be_a(String)
    notice.to_s.should_not be_empty
    Wolkit::Services.copy("text").should be_a(String)
    Wolkit::Services.paste.notice.to_s.should_not be_empty
    Wolkit::Services.close
  end

  it "gives a panel's refusal to its block" do
    seen = nil
    Wolkit::Services.open_file("Pick") { |outcome| seen = outcome }
    deadline = Time.instant + 3.seconds
    while seen.nil? && Time.instant < deadline
      sleep 10.milliseconds
    end
    outcome = seen.as(Wolkit::Outcome(Wol::OpenFile))
    outcome.ok?.should be_false
    outcome.notice.to_s.should_not be_empty
  end

  it "gives the sentence for a player, a recent file, a colour and a print when the service is not there" do
    player = Wolkit::Services.publish_media { |_p, _key, _by| }
    player.ok?.should be_false
    player.notice.to_s.should_not be_empty
    Wolkit::Services.recent("/home/a/report.pdf").to_s.should_not be_empty
    Wolkit::Services.recent("report.pdf").to_s.should_not be_empty
    color = nil
    Wolkit::Services.pick_color { |outcome| color = outcome }
    printed = nil
    Wolkit::Services.print_file("/home/a/report.pdf") { |outcome| printed = outcome }
    deadline = Time.instant + 3.seconds
    while (color.nil? || printed.nil?) && Time.instant < deadline
      sleep 10.milliseconds
    end
    color.as(Wolkit::Outcome(UInt32)).notice.to_s.should_not be_empty
    printed.as(Wolkit::Outcome(String)).notice.to_s.should_not be_empty
    Wolkit::Services.close
  end

  it "keeps a persisted signal in memory and shows the sentence when the preferences are refused" do
    Wolkit::Store.current = Wolkit::PrefsStore.new
    scope = Wolkit::Scope.new(nil)
    name = scope.run { Wolkit::Persisted(String).new("name", "x") }
    name.value = "y"
    name.value.should eq "y"
    name.notice.to_s.should_not be_empty
    scope.dispose
    Wolkit::Store.current = Wolkit::MemoryStore.new
    Wolkit::Services.close
  end
end
