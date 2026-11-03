# SPDX-License-Identifier: BSD-2-Clause
# The shard-side benchmark of plan-wolkit section 4.6: sample app 1 (todos) on the headless engine,
# measured from Crystal. It reports, as median and p95 over 30 runs: cold start to first frame,
# the time from a text change to the patch applied (budget 1, shard side), allocation per interaction
# (budget 12), a full collection at a 16 MB and a 64 MB heap (budget 13), and the encode cost of a
# patch of 100 ops. The engine's own numbers (frame time, damage) come from `wolkit_engine_stats_json`
# and the compositor's `wolstats`; this file measures what Crystal adds.
require "json"
require "../../../wolos-crystal/shards/wolkit/src/wolkit/test"
require "../../../wolos-crystal/examples/todos/src/todos"

RUNS = (ENV["WOLKIT_BENCH_RUNS"]? || "30").to_i

def percentile(values : Array(Float64), fraction : Float64) : Float64
  sorted = values.sort
  sorted[((sorted.size - 1) * fraction).to_i]
end

def summary(values : Array(Float64)) : NamedTuple(median: Float64, p95: Float64)
  {median: percentile(values, 0.5).round(3), p95: percentile(values, 0.95).round(3)}
end

# Milliseconds from now until the block ends.
def timed(& : ->) : Float64
  started = Time.instant
  yield
  (Time.instant - started).total_milliseconds
end

# A heap of `megabytes` of closures and strings that a UI app would hold, for the collector.
class Ballast
  getter rows = [] of Array(Proc(Nil))

  # Bytes the collector holds that are not free (after a collection: what is live).
  private def live_bytes : UInt64
    stats = GC.stats
    stats.heap_size - stats.free_bytes
  end

  def initialize(megabytes : Int32)
    target = megabytes.to_u64 * 1024 * 1024
    while live_bytes < target
      mark = "row #{@rows.size}"
      @rows << Array.new(4) { -> { mark.size; nil } }
    end
  end
end

start_ms = [] of Float64
interaction_us = [] of Float64
bytes_per_interaction = [] of Float64
collect16_ms = [] of Float64
collect64_ms = [] of Float64
encode_us = [] of Float64

RUNS.times do
  started = Time.instant
  Wolkit::Test.run(Todos) do |t|
    start_ms << (Time.instant - started).total_milliseconds
    t.type "warm", into: "What needs doing?"
    texts = ["a", "ab", "abc", ""]
    window = t.window
    samples = [] of Float64
    GC.collect
    before = GC.stats.total_bytes
    200.times do |i|
      window.inject_text(texts[i % 3])
      elapsed = timed { t.settle }
      samples << elapsed * 1000
    end
    bytes_per_interaction << (GC.stats.total_bytes - before) / 200.0
    interaction_us << percentile(samples, 0.5)
  end

  GC.collect
  small = Ballast.new(16)
  collect16_ms << timed { GC.collect }
  large = Ballast.new(64)
  collect64_ms << timed { GC.collect }
  small.rows.clear
  large.rows.clear
  GC.collect

  writer = Wolkit::PatchWriter.new
  encoded = timed do
    100.times do |i|
      writer.create((i + 1).to_u32, Wolkit::Wire::Kind::BOX)
      writer.set_f32((i + 1).to_u32, Wolkit::Wire::Prop::GAP, 12.0_f32)
    end
    writer.finish(1_u32)
  end
  encode_us << encoded * 1000 / 2
end

puts({
  runs:                      RUNS,
  crystal:                   Crystal::VERSION,
  start_to_first_frame_ms:   summary(start_ms),
  text_change_to_applied_us: summary(interaction_us),
  bytes_per_interaction:     summary(bytes_per_interaction),
  full_collect_16mb_ms:      summary(collect16_ms),
  full_collect_64mb_ms:      summary(collect64_ms),
  encode_100_ops_us:         summary(encode_us),
}.to_json)
