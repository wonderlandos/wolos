#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Final touches on the guest root: mkguest.sh ROOT REPO. The test user, session and test units, the
# generator that loads this run's files from the second disk, WolOS's current limits as the base.
set -e
R=$1 REPO=$2
arch-chroot "$R" useradd -m -u 1000 -s /bin/bash wol 2>/dev/null || true
arch-chroot "$R" passwd -d root >/dev/null
arch-chroot "$R" loginctl enable-linger wol 2>/dev/null || mkdir -p "$R/var/lib/systemd/linger"
mkdir -p "$R/var/lib/systemd/linger"; touch "$R/var/lib/systemd/linger/wol"
echo wolos-guest > "$R/etc/hostname"
cat > "$R/etc/fstab" <<'EOF'
/dev/vda /home btrfs subvol=/@home,compress=zstd 0 0
EOF
cp -r "$REPO"/image/files/usr/lib/sysctl.d/. "$R/usr/lib/sysctl.d/"
cp "$REPO"/image/files/usr/lib/tmpfiles.d/70-wolos-performance.conf "$R/usr/lib/tmpfiles.d/"
cp "$REPO"/image/files/usr/lib/systemd/zram-generator.conf "$R/usr/lib/systemd/"
mkdir -p "$R/usr/lib/systemd/system.conf.d" "$R/usr/lib/systemd/user.conf.d"
cp "$REPO"/image/files/usr/lib/systemd/system.conf.d/70-wolos-limits.conf "$R/usr/lib/systemd/system.conf.d/"
cp "$REPO"/image/files/usr/lib/systemd/user.conf.d/70-wolos-limits.conf "$R/usr/lib/systemd/user.conf.d/"
cat > "$R/usr/lib/systemd/system-generators/kbomb" <<'EOF'
#!/bin/sh
# Second disk: a tar with this run's test files, and after/ with the limits under test.
mkdir -p /run/kbomb && tar xf /dev/vdb -C /run/kbomb 2>/dev/null
: > /run/kbomb-guest
if grep -q 'kbmode=after' /proc/cmdline && [ -d /run/kbomb/after ]; then
	cp -a /run/kbomb/after/. /run/
fi
EOF
chmod +x "$R/usr/lib/systemd/system-generators/kbomb"
cat > "$R/usr/lib/systemd/system/kbomb.service" <<'EOF'
[Unit]
Description=limits test run
After=multi-user.target
[Service]
ExecStart=/run/kbomb/t/guest-run.sh
EOF
cat > "$R/usr/lib/systemd/system/kbomb-session.service" <<'EOF'
[Unit]
Description=stand-in for the desktop session
After=systemd-logind.service user-runtime-dir@1000.service
[Service]
User=wol
PAMName=login
Environment=XDG_SESSION_CLASS=user XDG_SESSION_TYPE=wayland XDG_SEAT=seat0
ExecStart=/bin/sh -c 'exec /run/kbomb/t/comp C > /dev/ttyS1'
EOF
ln -sf /usr/lib/systemd/system/kbomb.service "$R/etc/systemd/system/multi-user.target.wants/kbomb.service"
