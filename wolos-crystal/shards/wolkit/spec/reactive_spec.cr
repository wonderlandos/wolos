# SPDX-License-Identifier: BSD-2-Clause
# Specs for the reactive core: signals, effects, computeds, batches, scopes and lists.
require "./spec_helper"

# The macros live in Wolkit::DSL, which every view includes.
class SignalMacros
  include Wolkit::DSL

  getter string = signal("")
  getter int = signal(1)
  getter float = signal(1.5)
  getter flag = signal(false)
  getter maybe = signal(nil, of: String?)
  getter items = list(String)
end

describe Wolkit::Signal do
  it "runs an effect once at creation and again when the value changes" do
    scope = Wolkit::Scope.new(nil)
    runs = [] of Int32
    count = Wolkit::Signal(Int32).new(1)
    scope.run { Wolkit.effect { runs << count.value } }
    count.value = 2
    count.value = 2
    runs.should eq [1, 2]
    scope.dispose
  end

  it "forwards unknown methods through a tracked read" do
    scope = Wolkit::Scope.new(nil)
    draft = Wolkit::Signal(String).new("")
    blank = [] of Bool
    scope.run { Wolkit.effect { blank << draft.blank? } }
    draft.value = "milk"
    blank.should eq [true, false]
    scope.dispose
  end

  it "drops an effect that read nothing" do
    scope = Wolkit::Scope.new(nil)
    effect = scope.run { Wolkit.effect { 1 + 1 } }
    effect.disposed?.should be_true
    scope.dispose
  end

  it "infers the type of a literal signal at class level" do
    made = SignalMacros.new
    made.string.should be_a(Wolkit::Signal(String))
    made.int.should be_a(Wolkit::Signal(Int32))
    made.float.should be_a(Wolkit::Signal(Float64))
    made.flag.should be_a(Wolkit::Signal(Bool))
    made.maybe.should be_a(Wolkit::Signal(String?))
    made.items.should be_a(Wolkit::List(String))
  end

  it "does not record a dependency for peek and untracked" do
    scope = Wolkit::Scope.new(nil)
    a = Wolkit::Signal(Int32).new(1)
    b = Wolkit::Signal(Int32).new(1)
    runs = 0
    scope.run do
      Wolkit.effect do
        a.value
        b.peek
        Wolkit.untracked { b.value }
        runs += 1
      end
    end
    b.value = 2
    runs.should eq 1
    a.value = 2
    runs.should eq 2
    scope.dispose
  end
end

describe Wolkit::Batch do
  it "runs a dependent effect once per batch and in creation order" do
    scope = Wolkit::Scope.new(nil)
    a = Wolkit::Signal(Int32).new(0)
    b = Wolkit::Signal(Int32).new(0)
    log = [] of String
    scope.run do
      Wolkit.effect { log << "first #{a.value + b.value}" }
      Wolkit.effect { log << "second #{b.value}" }
    end
    log.clear
    Wolkit.batch do
      b.value = 1
      a.value = 1
      log.should be_empty
    end
    log.should eq ["first 2", "second 1"]
    scope.dispose
  end

  it "flushes at once outside a batch" do
    scope = Wolkit::Scope.new(nil)
    a = Wolkit::Signal(Int32).new(0)
    seen = 0
    scope.run { Wolkit.effect { seen = a.value } }
    a.value = 5
    seen.should eq 5
    scope.dispose
  end

  it "runs effects that other effects queue in the same flush" do
    scope = Wolkit::Scope.new(nil)
    a = Wolkit::Signal(Int32).new(0)
    b = Wolkit::Signal(Int32).new(0)
    seen = 0
    scope.run do
      Wolkit.effect { b.value = a.value * 2 }
      Wolkit.effect { seen = b.value }
    end
    a.value = 4
    seen.should eq 8
    scope.dispose
  end

  it "names a reactive cycle" do
    caught = [] of String
    previous = Wolkit.on_error
    Wolkit.on_error { |ex| caught << ex.message.to_s }
    scope = Wolkit::Scope.new(nil)
    a = Wolkit::Signal(Int32).new(0)
    b = Wolkit::Signal(Int32).new(0)
    scope.run do
      Wolkit.effect { b.value = a.value + 1 }
      Wolkit.effect { a.value = b.value + 1 }
    end
    caught.any?(&.includes?("reactive cycle")).should be_true
    scope.dispose
    Wolkit.on_error(&previous)
  end
end

describe Wolkit::Computed do
  it "is lazy, cached and recomputed after a change" do
    scope = Wolkit::Scope.new(nil)
    a = Wolkit::Signal(Int32).new(2)
    computes = 0
    double = Wolkit.computed do
      computes += 1
      a.value * 2
    end
    computes.should eq 0
    double.value.should eq 4
    double.value.should eq 4
    computes.should eq 1
    a.value = 3
    computes.should eq 1
    double.value.should eq 6
    computes.should eq 2
    scope.dispose
  end

  it "queues the effects that read it" do
    scope = Wolkit::Scope.new(nil)
    a = Wolkit::Signal(Int32).new(1)
    plus = Wolkit.computed { a.value + 10 }
    seen = [] of Int32
    scope.run { Wolkit.effect { seen << plus.value } }
    a.value = 2
    seen.should eq [11, 12]
    scope.dispose
  end
end

describe Wolkit::Scope do
  it "disposes effects and runs cleanups newest first" do
    log = [] of String
    scope = Wolkit::Scope.new(nil)
    a = Wolkit::Signal(Int32).new(0)
    scope.run do
      Wolkit.effect do
        a.value
        Wolkit.on_cleanup { log << "effect cleanup" }
      end
      Wolkit.on_cleanup { log << "scope cleanup 1" }
      Wolkit.on_cleanup { log << "scope cleanup 2" }
    end
    scope.dispose
    log.should eq ["scope cleanup 2", "scope cleanup 1", "effect cleanup"]
    a.observer_count.should eq 0
  end

  it "runs an effect cleanup before every re-run" do
    log = [] of String
    scope = Wolkit::Scope.new(nil)
    a = Wolkit::Signal(Int32).new(0)
    scope.run do
      Wolkit.effect do
        n = a.value
        Wolkit.on_cleanup { log << "cleanup #{n}" }
      end
    end
    a.value = 1
    log.should eq ["cleanup 0"]
    scope.dispose
    log.should eq ["cleanup 0", "cleanup 1"]
  end

  it "disposes child scopes with the parent" do
    parent = Wolkit::Scope.new(nil)
    child = Wolkit::Scope.new(parent)
    parent.dispose
    child.disposed?.should be_true
  end

  it "reports an exception in an effect and keeps going" do
    caught = [] of String
    previous = Wolkit.on_error
    Wolkit.on_error { |ex| caught << ex.message.to_s }
    scope = Wolkit::Scope.new(nil)
    a = Wolkit::Signal(Int32).new(0)
    ran = 0
    scope.run do
      Wolkit.effect do
        a.value
        raise "boom" if a.peek == 1
      end
      Wolkit.effect { ran = a.value }
    end
    a.value = 1
    caught.should eq ["boom"]
    ran.should eq 1
    scope.dispose
    Wolkit.on_error(&previous)
  end
end

describe Wolkit::List do
  it "tracks size and records changes for a cursor" do
    scope = Wolkit::Scope.new(nil)
    items = Wolkit::List(String).new
    cursor = items.open_cursor
    sizes = [] of Int32
    scope.run { Wolkit.effect { sizes << items.size } }
    items << "a"
    items << "b"
    items.delete("a")
    items.move(0, 0)
    sizes.should eq [0, 1, 2, 1]
    cursor.changes.map { |c| {c.kind, c.index} }.should eq [{Wolkit::ListChangeKind::Insert, 0}, {Wolkit::ListChangeKind::Insert, 1}, {Wolkit::ListChangeKind::Remove, 0}]
    scope.dispose
  end

  it "turns a replace into one reset" do
    items = Wolkit::List(Int32).new([1, 2, 3])
    cursor = items.open_cursor
    items << 4
    items.replace([9])
    cursor.changes.map(&.kind).should eq [Wolkit::ListChangeKind::Reset]
    items.to_a.should eq [9]
  end
end
