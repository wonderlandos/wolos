#!/usr/bin/rsh
# SPDX-License-Identifier: GPL-2.0-or-later
# deploy: update a checkout, build it, copy the result, restart the service if it runs.
# Usage: deploy.rsh [DESTINATION]      (the destination may contain spaces: it stays one argument)
dest = ARGV[0] || "/srv/app"
jobs = 4
mkdir -p src/app build
cd src/app
git pull --ff-only && make -j#{jobs}
rsync -a build/ #{dest}/
if systemctl is-active --quiet app
  systemctl --user restart app
  puts "restarted app"
else
  puts "app was not running"
end
puts "deployed to #{dest}"
