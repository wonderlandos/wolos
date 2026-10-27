#!/bin/sh
# Write WolOS rc.conf defaults into root $1 (live root or installed chroot "/").
set -eu
R=${1:-/}
for kv in dbus_enable=YES seatd_enable=YES linux_enable=YES \
          wolos_firstboot_enable=YES wold_enable=YES \
          ntpd_enable=YES ntpd_sync_on_start=YES sendmail_enable=NONE \
          clear_tmp_enable=YES cupsd_enable=YES avahi_daemon_enable=YES autofs_enable=YES \
          pf_enable=YES pf_rules=/usr/local/share/wolos/pf.conf syslogd_flags=-ss; do
  sysrc -f "$R/etc/rc.conf" "$kv" >/dev/null
done
# powerd on/off: rc.conf.d/powerd from `wol hw boot` (only with a cpufreq(4) driver, no Speed Shift).
sysrc -f "$R/etc/rc.conf" powerd_flags="-a hiadaptive" >/dev/null
# USB sticks under /media, writable for WolOS accounts (/etc/autofs/special_wolmedia).
grep -q '^/media' "$R/etc/auto_master" || printf '/media\t\t-wolmedia\t-nosuid,noatime\n' >> "$R/etc/auto_master"
# GPU kld_list is set by wolos_firstboot after pciconf detection.
