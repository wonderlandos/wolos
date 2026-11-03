# SPDX-License-Identifier: BSD-2-Clause
# Signals belong to the UI context. In a dev build a write from another thread raises a sentence that
# names `set_later`; a release build skips the check (this spec has nothing to test then).
require "./spec_helper"

{% if flag?(:dev) %}
  describe "the UI context rule" do
    it "refuses a signal write from another thread and says what to do (plan section 3.7)" do
      Wolkit::UI.claim
      signal = Wolkit::Signal(Int32).new(0)
      done = Channel(Exception?).new
      pool = Fiber::ExecutionContext::Isolated.new("spec-writer") do
        done.send(Wolkit.capture { signal.value = 5 }.as?(Exception))
      end
      error = done.receive
      pool.class.should eq Fiber::ExecutionContext::Isolated
      error.should be_a(Wolkit::UsageError)
      error.try(&.message).to_s.should contain("was called off the UI context. Use `set_later`")
      signal.peek.should eq 0
      signal.set_later(7)
      Wolkit::UI.run_posted
      signal.peek.should eq 7
      Wolkit::UI.release
    end
  end
{% end %}
