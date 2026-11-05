// SPDX-License-Identifier: GPL-2.0-or-later
// Doctor's development answers (over kit/fixtures.js). doctor.check is `wol doctor --json`
// recorded on a build box (fixtures/doctor-check.json); doctor.fix is the answer a fix that
// worked gives (fixtures/doctor-fix.json, written by hand in the shape `wol doctor fix --json`
// prints; fixtures/doctor-fix-no-admind.json is the recorded answer without wol-admind).
wolkitFixtures({
 "app": {
  "id": "org.wolos.webapp.doctor",
  "version": "0.1.0"
 },
 "permissions": [
  "doctor.read",
  "doctor.fix"
 ],
 "calls": {
  "doctor.check": {
   "checked": 1793749137,
   "checks": [
    {
     "area": "sound",
     "detail": "The sound service is stopped, so nothing plays.",
     "fix": {
      "command": "systemctl --user start pipewire.service pipewire-pulse.service",
      "needs_admin": false,
      "title": "Start it again"
     },
     "id": "sound.pipewire",
     "status": "fail",
     "title": "Sound service"
    },
    {
     "area": "sound",
     "detail": "The part that finds speakers and microphones is stopped, so volume and device choices do nothing.",
     "fix": {
      "command": "systemctl --user start wireplumber.service",
      "needs_admin": false,
      "title": "Start it again"
     },
     "id": "sound.wireplumber",
     "status": "fail",
     "title": "Speakers and microphones"
    },
    {
     "area": "sound",
     "detail": "Sound gets the first turn on the processor, so it stays smooth when the computer is busy.",
     "id": "sound.realtime",
     "status": "ok",
     "title": "Smooth sound when busy"
    },
    {
     "area": "sound",
     "detail": "Part of the WolOS sound tuning is missing or was changed.",
     "fix": {
      "command": "wol admin pkg-install wolos-system",
      "needs_admin": true,
      "title": "Put it back"
     },
     "id": "sound.config",
     "status": "fail",
     "title": "WolOS sound tuning"
    },
    {
     "area": "sound",
     "detail": "No microphone is connected. Plug one in or pair one to talk and record.",
     "id": "sound.input-level",
     "status": "warn",
     "title": "Microphone level"
    },
    {
     "area": "sound",
     "detail": "Sound timing adjusts itself to each app.",
     "id": "sound.clock",
     "status": "ok",
     "title": "Sound timing"
    },
    {
     "area": "sound",
     "detail": "No sound dropouts since the devices started.",
     "id": "sound.xruns",
     "status": "ok",
     "title": "Dropouts"
    },
    {
     "area": "sound",
     "detail": "Background noise removal is not installed, so its switch in Sound settings does nothing.",
     "fix": {
      "command": "wol admin pkg-install noise-suppression-for-voice",
      "needs_admin": true,
      "title": "Install it"
     },
     "id": "sound.rnnoise",
     "status": "warn",
     "title": "Background noise removal"
    },
    {
     "area": "sound",
     "detail": "No recording app is installed.",
     "id": "sound.obs",
     "status": "skipped",
     "title": "Recording and streaming"
    },
    {
     "area": "sound",
     "detail": "Apps cannot ask to record your screen, so screen capture shows nothing.",
     "fix": {
      "command": "wol admin pkg-install xdg-desktop-portal-hyprland",
      "needs_admin": true,
      "title": "Install it"
     },
     "id": "sound.portal",
     "status": "warn",
     "title": "Screen recording"
    },
    {
     "area": "sound",
     "detail": "No sound card or camera was found.",
     "id": "sound.devices",
     "status": "skipped",
     "title": "Sound devices"
    },
    {
     "area": "games",
     "detail": "You run your own kernel. WolOS keeps it as it is.",
     "id": "games.base",
     "status": "skipped",
     "title": "WolOS for games"
    },
    {
     "area": "games",
     "detail": "The package source that carries Steam and its 32-bit parts is off, so Steam cannot be installed.",
     "id": "games.multilib",
     "status": "fail",
     "title": "Game package source"
    },
    {
     "area": "games",
     "detail": "Steam is not installed.",
     "fix": {
      "command": "wol admin pkg-install steam",
      "needs_admin": true,
      "title": "Install Steam"
     },
     "id": "games.steam",
     "status": "fail",
     "title": "Steam"
    },
    {
     "area": "games",
     "detail": "The graphics driver is running.",
     "id": "games.drm",
     "status": "ok",
     "title": "Graphics driver"
    },
    {
     "area": "games",
     "detail": "The 32-bit graphics parts Steam needs are missing, so Steam cannot open.",
     "fix": {
      "command": "wol admin pkg-install vulkan-icd-loader; wol admin pkg-install lib32-vulkan-icd-loader; wol admin pkg-install lib32-mesa",
      "needs_admin": true,
      "title": "Install them"
     },
     "id": "games.lib32-gl",
     "status": "fail",
     "title": "Graphics for Steam"
    },
    {
     "area": "games",
     "detail": "A graphics driver for modern games is installed.",
     "id": "games.vulkan-icd",
     "status": "ok",
     "title": "Driver for modern games"
    },
    {
     "area": "games",
     "detail": "The graphics test tool is missing, so WolOS cannot see which card games use.",
     "fix": {
      "command": "wol admin pkg-install vulkan-tools",
      "needs_admin": true,
      "title": "Install it"
     },
     "id": "games.vulkan",
     "status": "warn",
     "title": "Graphics card for games"
    },
    {
     "area": "games",
     "detail": "Windows games use the fastest way to wait.",
     "id": "games.futex-waitv",
     "status": "ok",
     "title": "Fast waiting for Windows games"
    },
    {
     "area": "games",
     "detail": "Windows games use the standard timing, which works well.",
     "id": "games.ntsync",
     "status": "skipped",
     "title": "Windows timing"
    },
    {
     "area": "games",
     "detail": "Not installed. Steam games do not need it.",
     "fix": {
      "command": "wol admin pkg-install wine",
      "needs_admin": true,
      "title": "Install it"
     },
     "id": "games.wine",
     "status": "skipped",
     "title": "Windows apps outside Steam"
    },
    {
     "area": "updates",
     "detail": "WolOS looks for updates every day. The first look has not happened yet.",
     "id": "updates.checked",
     "status": "warn",
     "title": "Looking for updates"
    },
    {
     "area": "updates",
     "detail": "Shown after the first look for updates.",
     "id": "updates.waiting",
     "status": "skipped",
     "title": "Updates"
    },
    {
     "area": "security",
     "detail": "The firewall is off, so other computers on the network can reach this one.",
     "fix": {
      "command": "wol admin firewall on",
      "needs_admin": true,
      "title": "Turn on the firewall"
     },
     "id": "security.firewall",
     "status": "fail",
     "title": "Firewall"
    },
    {
     "area": "security",
     "detail": "Nobody can sign in to this computer from another one.",
     "id": "security.remote-login",
     "status": "ok",
     "title": "Remote Login"
    },
    {
     "area": "security",
     "detail": "A program on this computer can send the system log over the network.",
     "id": "security.system-log",
     "status": "warn",
     "title": "System log"
    },
    {
     "area": "security",
     "detail": "2 protections in the system core are weaker than WolOS sets them. A restart puts them back.",
     "id": "security.protections",
     "status": "warn",
     "title": "Core protections"
    },
    {
     "area": "security",
     "detail": "OpenSSL and GnuTLS offer the post-quantum group first, TLS 1.2 or newer, modern ciphers only.",
     "id": "security.tls-policy",
     "status": "ok",
     "title": "System TLS policy"
    },
    {
     "area": "security",
     "detail": "Offers the post-quantum group to every site; Encrypted Client Hello and http-to-https upgrades are on (set by policy, measured in the browser, not here).",
     "id": "security.browser-tls",
     "status": "ok",
     "title": "Atlas browser"
    },
    {
     "area": "security",
     "detail": "The malware check is off. Turn it on in Settings > Privacy & Security.",
     "id": "security.malware",
     "status": "warn",
     "title": "Malware check"
    },
    {
     "area": "security",
     "detail": "No harmful files were found.",
     "id": "security.stopped",
     "status": "ok",
     "title": "Harmful files"
    },
    {
     "area": "storage",
     "detail": "The startup disk is almost full: 2.2 GB free of 415 GB. Apps and updates can fail until you make room.",
     "id": "storage.space",
     "status": "fail",
     "title": "Free space"
    },
    {
     "area": "storage",
     "detail": "Your files are not locked. Anyone who takes the disk can read them.",
     "id": "storage.encryption",
     "status": "warn",
     "title": "Disk lock"
    },
    {
     "area": "storage",
     "detail": "This disk does not keep restore points.",
     "id": "storage.restore-points",
     "status": "skipped",
     "title": "Restore points"
    },
    {
     "area": "network",
     "detail": "Connected through a cable.",
     "id": "network.connection",
     "status": "ok",
     "title": "Connection"
    },
    {
     "area": "network",
     "detail": "Websites are found by name.",
     "id": "network.names",
     "status": "ok",
     "title": "Finding websites"
    },
    {
     "area": "network",
     "detail": "Every Wi-Fi, cable and Bluetooth device has its driver.",
     "id": "network.devices",
     "status": "ok",
     "title": "Network devices"
    },
    {
     "area": "power",
     "detail": "This computer runs from the wall and has no battery.",
     "id": "power.battery",
     "status": "skipped",
     "title": "Battery"
    },
    {
     "area": "power",
     "detail": "This computer cannot sleep, so it stays on when you step away.",
     "id": "power.sleep",
     "status": "warn",
     "title": "Sleep"
    },
    {
     "area": "display",
     "detail": "Screens are checked from the desktop.",
     "id": "display.screens",
     "status": "skipped",
     "title": "Screens"
    },
    {
     "area": "display",
     "detail": "Device 1111 has no driver of its own, so it works only at a basic level.",
     "id": "display.devices",
     "status": "warn",
     "title": "Graphics"
    }
   ],
   "ready": false
  },
  "doctor.fix": {
   "check": {
    "area": "security",
    "detail": "The firewall is on. Nothing comes in unless you ask for it.",
    "id": "security.firewall",
    "status": "ok",
    "title": "Firewall"
   },
   "id": "security.firewall",
   "ok": true,
   "ran": true
  }
 }
});
