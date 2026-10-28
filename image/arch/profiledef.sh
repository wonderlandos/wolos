#!/usr/bin/env bash
# shellcheck disable=SC2034
# WolOS live + installer ISO (mkarchiso, releng-based). Staged and run by image/build.sh.
iso_name="wolos"
iso_label="WOLOS_$(date --date="@${SOURCE_DATE_EPOCH:-$(date +%s)}" +%Y%m)"
iso_publisher="WolOS <https://github.com/wonderlandos/wolos>"
iso_application="WolOS Live and Installer"
iso_version="$(date --date="@${SOURCE_DATE_EPOCH:-$(date +%s)}" +%Y.%m.%d)"
install_dir="wolos"
buildmodes=('iso')
bootmodes=('uefi.systemd-boot')
pacman_conf="pacman.conf"
airootfs_image_type="squashfs"
airootfs_image_tool_options=('-comp' 'zstd' '-Xcompression-level' '15' '-b' '1M')
file_permissions=(
  ["/etc/shadow"]="0:0:400"
  ["/etc/sudoers.d/wolos-live"]="0:0:440"
)
