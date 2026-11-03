# SPDX-License-Identifier: BSD-2-Clause
# hello: the high-layer counterpart of crates/wol-abi/bindings/crystal/hello.cr. It prints the
# same four lines (version, OS, a posted notification, one theme event), so
# crates/wol-abi/tests/crystal_shard.rs checks it with the same fake wold as the C program.
require "../src/cryswol"

Wol::Context.open("org.example.hello") do |ctx|
  puts "version: #{Wol.version_string}"
  puts "os: #{Wol::System.info(ctx).os_name}"
  Wol::Notification.post(ctx, "Hello", "from hello")
  puts "posted"
  seen = false
  ctx.subscribe("theme") do |event|
    puts "theme: #{event.json}"
    seen = true
    Wol::Loop.stop(ctx)
  end
  Wol::Loop.run(ctx, 10.seconds)
  abort "no theme event arrived" unless seen
end
