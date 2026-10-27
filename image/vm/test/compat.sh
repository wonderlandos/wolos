#!/bin/sh
# Linuxulator + Proton end to end through wol-run, on the FreeBSD build VM (WolKernel).
# Host (Linux): builds static Linux fixtures (kernel/patches/tests) and a CRT-free Windows console exe.
# VM: wraps each in an App.wol bundle and runs it with wol-run; prints output + exit code.
#   uname.wol  Linux CLI from linux_base (/compat/linux/usr/bin/uname)
#   futex.wol  futex_waitv(2) conformance + ping-pong (WolKernel patches)
#   regs.wol   Linux syscall register preservation (WolKernel patches)
#   win.wol    targets linux (entry missing) then proton -> must fall back to Proton, exit 7
#   gears.wol  Linux GUI (linux-rl9-glx-utils glxgears) on Xwayland in headless sway (test/headless.sh)
# Usage: compat.sh (build VM over ssh) | compat.sh --iso out.iso (fixtures + compat-vm.sh for the installed disk)
# Needs host: gcc (static glibc), clang, lld-link, llvm-dlltool. VM: wine-proton, linux-rl9-glx-utils, sway, grim.
set -eu
REPO=$(cd "$(dirname "$0")/../../.." && pwd); SSH=$REPO/image/vm/ssh.sh
B=$(mktemp -d); trap 'rm -rf "$B"' EXIT
gcc -O2 -static -pthread -o "$B/futex_waitv" "$REPO/kernel/patches/tests/futex_waitv.c"
gcc -O2 -static -o "$B/syscall_regs" "$REPO/kernel/patches/tests/syscall_regs.c"
printf 'LIBRARY kernel32.dll\nEXPORTS\nGetStdHandle\nWriteFile\nExitProcess\n' > "$B/k32.def"
llvm-dlltool -m i386:x86-64 -d "$B/k32.def" -l "$B/kernel32.lib"
clang --target=x86_64-pc-windows-msvc -O2 -c "$REPO/image/vm/test/fixtures/win-hello.c" -o "$B/hello.obj"
lld-link /subsystem:console /entry:mainCRTStartup /nodefaultlib /out:"$B/hello.exe" "$B/hello.obj" "$B/kernel32.lib" >/dev/null
cp "$REPO/image/vm/test/headless.sh" "$B/"
cp "$REPO/image/vm/test/compat-vm.sh" "$B/"
if [ "${1:-}" = --iso ]; then   # for the installed disk: attach as -cdrom, mount_cd9660 /dev/cd0 /mnt; sh /mnt/compat-vm.sh /mnt
  xorriso -as mkisofs -quiet -R -o "$2" "$B"; echo "compat: $2"; exit 0
fi
tar -C "$B" -cf - . | $SSH 'rm -rf /work/fx && mkdir -p /work/fx && tar -C /work/fx -xf -'
$SSH 'env ASSUME_ALWAYS_YES=yes pkg install -y wine-proton linux-rl9-glx-utils sway grim >/dev/null 2>&1; R=/work/target/release/wol-run sh /work/fx/compat-vm.sh /work/fx'
