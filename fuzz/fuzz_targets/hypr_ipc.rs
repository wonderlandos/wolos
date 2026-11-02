// SPDX-License-Identifier: GPL-2.0-or-later
//! Hyprland IPC replies and event lines as the shell reads them (`shell::hypr`; audit B10):
//! hyprctl JSON and socket2 events are input from another process, so never panic.
#![no_main]
use libfuzzer_sys::fuzz_target;
use shell::hypr;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = hypr::parse_event(&text);
    let _ = hypr::parse_clients(&text);
    let _ = hypr::parse_active(&text);
    let _ = hypr::parse_active_client(&text);
    let _ = hypr::parse_layers(&text);
    let _ = hypr::parse_zone(&text);
});
