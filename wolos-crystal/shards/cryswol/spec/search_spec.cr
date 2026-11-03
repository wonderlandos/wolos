# SPDX-License-Identifier: BSD-2-Clause
# A Search provider against the fake wold scripted by spec/fixtures/search.json: wold sends a query
# after search.provide, and the person opens row "note-1" after the reply.
require "./spec_helper"

describe Wol::SearchProvider do
  it "answers a query with rows and gets the opened key" do
    Wol::Spec.with_fake("search") do |ctx, fake|
      asked = [] of String
      opened = [] of String
      kept = [] of Wol::SearchReply
      provider = Wol::Search.provide(ctx, on_open: ->(key : String) { opened << key; nil }) do |reply, text|
        asked << text
        reply.add("note-1", "Shopping list", "Note")
        reply.add("note-2", "Ideas")
        kept << reply
      end
      GC.collect
      Wol::Spec.run_until(ctx) { !opened.empty? }.should be_true
      asked.should eq ["term"]
      opened.should eq ["note-1"]
      expect_raises(Wol::UsageError) { kept.first.add("late", "Too late") }
      reply = fake.calls.find! { |c| c["call"] == "search.reply" }
      reply["results"].as_a.map(&.["key"].as_s).should eq ["note-1", "note-2"]
      reply["results"][0]["detail"].as_s.should eq "Note"
      provider.close
      ctx.rooted_count.should eq 0
    end
  end

  it "reports a raising block to on_error and keeps running" do
    Wol::Spec.with_fake("search") do |ctx, _fake|
      errors = Wol::Spec.capture_errors do |seen|
        Wol::Search.provide(ctx) { |_reply, _text| raise "boom" }
        Wol::Spec.run_until(ctx, 1.second) { !seen.empty? }
      end
      errors.map(&.message).should eq ["boom"]
    end
  end
end
