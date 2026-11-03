# SPDX-License-Identifier: BSD-2-Clause
# Background services against the fake wold scripted by spec/fixtures/services.json.
require "./spec_helper"

describe Wol::Service do
  it "starts, reads the status and stops" do
    Wol::Spec.with_fake("services") do |ctx, fake|
      Wol::Service.start(ctx, "sync")
      status = Wol::Service.status(ctx, "sync")
      status["state"].as_s.should eq "running"
      status["starts"].as_i?.should eq 1
      Wol::Service.stop(ctx, "sync")
      fake.call_tags.should eq ["service.start", "service.status", "service.stop"]
      fake.calls.map(&.["name"].as_s).uniq.should eq ["sync"]
      expect_raises(Wol::PermissionDenied) { Wol::Service.start(ctx, "sync") }
    end
  end

  it "refuses a bad name before asking wold" do
    Wol::Spec.with_fake("services") do |ctx, fake|
      expect_raises(Wol::InvalidArgument) { Wol::Service.start(ctx, "no/slash") }
      fake.call_tags.should be_empty
    end
  end

  it "tells a service process from the app" do
    Wol::Service.current(["--wol-service=sync"]).should eq "sync"
    Wol::Service.current([] of String).should be_nil
    Wol::Service.current(["--wol-service=sync", "x"]).should be_nil
  end
end
