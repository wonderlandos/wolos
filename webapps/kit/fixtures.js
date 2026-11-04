// SPDX-License-Identifier: BSD-2-Clause
// Recorded answers for development mode (ADR 0048), in the shape of each `wol` verb's --json output.
// The argument is strict JSON (crates/webengine/src/bridge/mock.rs reads it too). An app adds its
// own in webapps/<id>/fixtures.js with the same call; later files win key by key.
wolkitFixtures({
  "app": {"version": "0.13.0", "locale": "en", "theme": "light", "accent": "violet"},
  "calls": {
    "about": {"name": "WolOS 0.13", "base": "WolOS base 0.13", "kernel": "WolKernel 7.0", "memory": "16 GB", "processor": "8-core processor"},
    "sys": [
      {"device": "/dev/nvme0n1p2", "mount": "/", "total": 512110190592, "used": 187904819200, "available": 324205371392},
      {"device": "/dev/nvme0n1p1", "mount": "/boot", "total": 1073741824, "used": 134217728, "available": 939524096}
    ],
    "hw.status": {"devices": [{"id": "gpu0", "kind": "graphics", "name": "Integrated graphics", "status": "ok", "driver": "in use"}], "missing": []},
    "power.status": {"battery": null, "charge_limit": null, "cpu": "balanced", "lid": null, "mode": "auto", "sleep": true, "source": "ac", "wifi_power_save": []},
    "battery": {"present": false},
    "boot.status": {"slot": "a", "next": "a", "secure_boot": true, "measured": true},
    "modules.status": {"modules": [{"id": "printing", "installed": false, "enabled": false}]},
    "update.status": {"checked": 1762210800, "packages": 3, "security": 1, "release": null},
    "update.check": {"upgrades": [{"name": "wolos-system", "from": "0.13.0-1", "to": "0.13.0-2"}]},
    "update.install": {"applied": 3, "restart": false},
    "security.status": {
      "firewall": true, "firewall_running": true, "remote_login": false, "system_log_private": true,
      "hardening": [{"key": "kernel.kptr_restrict", "now": "2", "want": "2"}, {"key": "kernel.yama.ptrace_scope", "now": "1", "want": "1"}],
      "malware": {"on": true, "blocked": [], "signatures_age_days": 1, "signatures_built": 1762124400, "signatures_version": 27012}
    },
    "firewall.status": {"on": true, "running": true},
    "network": {
      "online": true, "gateway": "192.168.1.1", "dns": ["192.168.1.1"],
      "interfaces": [{"name": "wlan0", "kind": "Wi-Fi", "up": true, "ipv4": ["192.168.1.24/24"], "ipv6": [], "mac": "02:00:00:00:00:01"}],
      "wifi": {"on": true, "network": "Home", "signal": 72}, "bluetooth": {"on": true, "connected": 1}
    },
    "wifi.get": {"on": true, "network": "Home", "signal": 72},
    "bluetooth.get": {"on": true, "connected": 1},
    "audio.get": {"volume": 45, "muted": false, "output": "Speakers", "input": "Built-in microphone"},
    "audio.doctor": {"ready": false, "checks": [
      {"id": "service", "status": "ok", "detail": "The sound service is running", "fix": ""},
      {"id": "devices", "status": "ok", "detail": "Speakers and a microphone are ready", "fix": ""},
      {"id": "realtime", "status": "ok", "detail": "Sound gets priority when the computer is busy", "fix": ""},
      {"id": "input-level", "status": "warn", "detail": "The microphone is turned up above 0 dB, so loud sounds clip", "fix": "Lower the microphone level in Settings > Sound"},
      {"id": "noise", "status": "warn", "detail": "Reduce background noise has nothing to load", "fix": "sudo pacman -S noise-suppression-for-voice"},
      {"id": "xruns", "status": "ok", "detail": "No dropouts", "fix": ""}
    ]},
    "audio.volume": {"volume": 80, "muted": false},
    "audio.mute": {"volume": 45, "muted": true},
    "audio.unmute": {"volume": 45, "muted": false},
    "audio.reset": {"dry_run": true, "plan": ["Forget saved volumes", "Restart the sound service"]},
    "games.doctor": {"checks": [
      {"id": "base", "status": "info", "detail": "WolOS 0.13 with WolKernel 7.0"},
      {"id": "drm", "status": "ok", "detail": "The graphics driver is loaded"},
      {"id": "lib32-gl", "status": "fail", "detail": "32-bit graphics libraries are missing; older games need them", "fix": "sudo pacman -S --needed lib32-mesa lib32-vulkan-icd-loader"},
      {"id": "limits", "status": "ok", "detail": "Games may open enough files"}
    ]},
    "doctor.check": {"checked": 1762210800, "checks": [
      {"id": "sound", "area": "sound", "title": "Sound", "status": "ok", "detail": "Speakers and a microphone are ready"},
      {"id": "updates", "area": "system", "title": "Updates", "status": "warn", "detail": "3 updates are ready",
       "fix": {"title": "Install updates", "command": "wol admin pkg-upgrade", "needs_admin": true}}
    ]},
    "doctor.fix": {"id": "updates", "ran": true, "ok": true, "check": {"id": "updates", "area": "system", "title": "Updates", "status": "ok", "detail": "Up to date"}},
    "notify": {"sent": true}
  }
});
