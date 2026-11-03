# SPDX-License-Identifier: BSD-2-Clause
# Budgets 3 and 12 of plan-wolkit section 4.1, on the Crystal side: the effect flush is fast, nothing is
# allocated at idle, and an interaction allocates at most 8 KB. Measured with GC.stats.total_bytes (the
# bytes the program asked the collector for), so they do not depend on when a collection runs.
require "./spec_helper"
require "./dsl_app_spec"

# Bytes allocated while the block runs.
def allocated(& : ->) : UInt64
  GC.collect
  before = GC.stats.total_bytes
  yield
  GC.stats.total_bytes - before
end

describe "the budgets" do
  after_each { Wolkit::Test.reset }

  it "allocates nothing at idle (budget 12)" do
    Wolkit::Test.record(SpecTodos.new, keep: false)
    # Warm up so buffers have their final size.
    5.times { Wolkit::Test.settle }
    bytes = allocated do
      10_000.times do
        Wolkit::Test.settle
        Wolkit::Batch.flush
      end
    end
    bytes.should eq 0
  end

  it "allocates at most 8 KB per interaction (budget 12)" do
    app = SpecTodos.new
    Wolkit::Test.record(app, keep: false)
    texts = ["milk", "milk and eggs", "milk", ""]
    # Warm up: the first changes grow the writer's buffers.
    40.times { |i| app.draft.value = texts[i % 4]; Wolkit::Test.settle }
    count = 2000
    bytes = allocated do
      count.times do |i|
        app.draft.value = texts[i % 4]
        Wolkit::Test.settle
      end
    end
    (bytes // count).should be <= 8192
  end

  it "flushes 20 dependent effects in under half a millisecond (budget 3)" do
    scope = Wolkit::Scope.new(nil)
    source = Wolkit::Signal(Int32).new(0)
    sink = 0
    scope.run { 20.times { Wolkit.effect { sink += source.value } } }
    flushes = 2000
    started = Time.instant
    flushes.times { |i| source.value = i + 1 }
    average = (Time.instant - started).total_milliseconds / flushes
    average.should be < 0.5
    scope.dispose
  end

  it "stays flat in memory over many row changes" do
    app = SpecTodos.new
    Wolkit::Test.record(app, keep: false)
    # Add and remove rows: ids and scopes come back, so the live part count returns to where it was.
    mount = Wolkit::Runtime.windows.first.mount
    base = mount.live
    200.times do |i|
      app.items << "item #{i}"
      Wolkit::Test.settle
    end
    mount.live.should be > base
    app.items.clear
    Wolkit::Test.settle
    mount.live.should eq base
  end

  it "says what the limit is when a window has too many parts (plan section 3.7)" do
    mount = Wolkit::Mount.new(->(_bytes : Bytes) { })
    scope = Wolkit::Scope.new(nil)
    error = nil
    scope.run do
      begin
        70_000.times { mount.create(Wolkit::Wire::Kind::BOX) }
      rescue ex : Wolkit::LimitError
        error = ex
      end
    end
    error.try(&.message).should eq "This window has 65,537 parts; the limit is 65,536. Use `list` for long lists."
    scope.dispose
  end
end
