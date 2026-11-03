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
end
