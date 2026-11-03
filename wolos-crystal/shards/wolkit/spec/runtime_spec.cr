# SPDX-License-Identifier: BSD-2-Clause
# Specs for timers, background resources, persisted signals and the UI queue.
require "./spec_helper"

# Run posted work until `done` is true or two seconds pass.
def pump_until(& : -> Bool) : Nil
  deadline = Time.instant + 2.seconds
  until yield
    Wolkit::UI.run_posted
    sleep 5.milliseconds
    raise "timed out" if Time.instant > deadline
  end
end

describe Wolkit::Timer do
  it "runs after the span, once" do
    scope = Wolkit::Scope.new(nil)
    count = 0
    scope.run { Wolkit.after(10.milliseconds) { count += 1 } }
    sleep 60.milliseconds
    count.should eq 1
    scope.dispose
  end

  it "repeats until the scope is disposed" do
    scope = Wolkit::Scope.new(nil)
    count = 0
    scope.run { Wolkit.every(10.milliseconds) { count += 1 } }
    sleep 80.milliseconds
    scope.dispose
    seen = count
    seen.should be >= 3
    sleep 50.milliseconds
    count.should eq seen
  end

  it "does not run a block whose scope ended first" do
    scope = Wolkit::Scope.new(nil)
    ran = false
    scope.run { Wolkit.after(30.milliseconds) { ran = true } }
    scope.dispose
    sleep 80.milliseconds
    ran.should be_false
  end
end

describe Wolkit::Resource do
  it "fetches in the background and drops a stale result" do
    scope = Wolkit::Scope.new(nil)
    query = Wolkit::Signal(String).new("a")
    resource = scope.run do
      Wolkit::Resource(String).new(query) do |q|
        sleep(q == "slow" ? 150.milliseconds : 5.milliseconds)
        q.upcase
      end
    end
    resource.loading?.should be_true
    pump_until { resource.value == "A" }
    resource.loading?.should be_false
    query.value = "slow"
    query.value = "b"
    pump_until { resource.value == "B" }
    sleep 200.milliseconds
    Wolkit::UI.run_posted
    resource.value.should eq "B"
    scope.dispose
  end

  it "keeps the exception of a failed fetch" do
    scope = Wolkit::Scope.new(nil)
    source = Wolkit::Signal(Int32).new(1)
    resource = scope.run { Wolkit::Resource(Int32).new(source) { |_n| raise "no network" } }
    pump_until { !resource.loading? }
    resource.error.try(&.message).should eq "no network"
    resource.value.should be_nil
    scope.dispose
  end
end

describe Wolkit::Persisted do
  it "restores a stored value and saves a change" do
    store = Wolkit::MemoryStore.new
    Wolkit::Store.current = store
    store.write("width", "640")
    scope = Wolkit::Scope.new(nil)
    width = scope.run { Wolkit::Persisted(Int32).new("width", 420) }
    width.value.should eq 640
    width.value = 800
    store.read("width").should eq "800"
    scope.dispose
    Wolkit::Store.current = Wolkit::MemoryStore.new
  end

  it "falls back to the default for text that no longer fits the type" do
    store = Wolkit::MemoryStore.new
    Wolkit::Store.current = store
    store.write("width", "\"wide\"")
    scope = Wolkit::Scope.new(nil)
    scope.run { Wolkit::Persisted(Int32).new("width", 420) }.value.should eq 420
    scope.dispose
    Wolkit::Store.current = Wolkit::MemoryStore.new
  end

  it "shows the store's sentence and keeps working in memory when the store refuses" do
    store = RefusingStore.new
    Wolkit::Store.current = store
    scope = Wolkit::Scope.new(nil)
    name = scope.run { Wolkit::Persisted(String).new("name", "x") }
    name.value = "y"
    name.value.should eq "y"
    name.notice.should eq "Allow this app to keep its settings in System Settings."
    scope.dispose
    Wolkit::Store.current = Wolkit::MemoryStore.new
  end
end

class RefusingStore < Wolkit::Store
  def read(key : String) : String?
    nil
  end

  def write(key : String, value : String) : String?
    "Allow this app to keep its settings in System Settings."
  end
end

describe Wolkit::UI do
  it "runs posted work in a batch" do
    scope = Wolkit::Scope.new(nil)
    a = Wolkit::Signal(Int32).new(0)
    runs = 0
    scope.run { Wolkit.effect { a.value; runs += 1 } }
    a.set_later(5)
    Wolkit::UI.run_posted.should eq 1
    a.value.should eq 5
    runs.should eq 2
    scope.dispose
  end
end
