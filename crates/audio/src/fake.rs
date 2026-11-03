// SPDX-License-Identifier: GPL-2.0-or-later
//! Fake-registry test mode: a recording backend and a fixture of the audit's machine (a codec
//! microphone at 100% = +30 dB, an HDMI output, Bluetooth headphones). The tests below are the
//! contract of the service; the pipewire backend only has to deliver the same events.
use crate::model::{Command, Event, NodeInfo};
use crate::service::{Backend, Service};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use wol_core::audio::DeviceKind;

/// Records every command; answers `Run` from `replies` (`argv joined by spaces` -> stdout);
/// a `Run` whose line starts with the first part of a `fails` pair returns its second part as
/// the error (a module that does not load, a plugin that is not installed).
#[derive(Default)]
pub struct FakeBackend {
    pub commands: Rc<RefCell<Vec<Command>>>,
    pub replies: Vec<(String, String)>,
    pub fails: Vec<(String, String)>,
    pub files: Vec<(PathBuf, String)>,
}

impl FakeBackend {
    pub fn taken(&self) -> Vec<Command> {
        std::mem::take(&mut *self.commands.borrow_mut())
    }
    /// The `Run` argv lines, joined by spaces.
    pub fn runs(&self) -> Vec<String> {
        self.commands
            .borrow()
            .iter()
            .filter_map(|c| match c {
                Command::Run(a) => Some(a.join(" ")),
                _ => None,
            })
            .collect()
    }
}

impl Backend for FakeBackend {
    fn execute(&mut self, cmd: &Command) -> Result<String, String> {
        self.commands.borrow_mut().push(cmd.clone());
        match cmd {
            Command::Run(argv) => {
                let line = argv.join(" ");
                if let Some(f) = self.fails.iter().find(|f| line.starts_with(&f.0)) {
                    return Err(f.1.clone());
                }
                Ok(self
                    .replies
                    .iter()
                    .find(|r| r.0 == line)
                    .map(|r| r.1.clone())
                    .unwrap_or_default())
            }
            Command::WriteFile(p, t) => {
                self.files.retain(|f| &f.0 != p);
                self.files.push((p.clone(), t.clone()));
                Ok(String::new())
            }
            Command::AppendLine(p, l) => {
                match self.files.iter_mut().find(|f| &f.0 == p) {
                    Some(f) => {
                        f.1.push_str(l);
                        f.1.push('\n');
                    }
                    None => self.files.push((p.clone(), format!("{l}\n"))),
                }
                Ok(String::new())
            }
            Command::RemovePath(p) => {
                self.files.retain(|f| !f.0.starts_with(p));
                Ok(String::new())
            }
            _ => Ok(String::new()),
        }
    }
    fn read_file(&self, p: &Path) -> Option<String> {
        self.files.iter().find(|f| f.0 == p).map(|f| f.1.clone())
    }
}

/// 0 dB of a microphone whose mixer goes up to +30 dB.
pub const MIC_BASE: f32 = 0.031_622_78;

/// The audit's machine as registry events, in the order PipeWire sends them.
pub fn fixture() -> Vec<Event> {
    vec![
        Event::Node(NodeInfo {
            id: 45,
            node_name: "alsa_output.pci-0000_00_1f.3.hdmi-stereo".into(),
            description: "HDMI Audio".into(),
            kind: DeviceKind::Output,
            device_id: Some(40),
            profile: Some("output:hdmi-stereo".into()),
            ..Default::default()
        }),
        Event::Props {
            id: 45,
            channel_volumes: vec![0.064, 0.064],
            volume_base: 1.0,
            muted: false,
        },
        Event::Node(NodeInfo {
            id: 46,
            node_name: "alsa_input.pci-0000_00_1f.3.analog-stereo".into(),
            description: "Built-in Audio Analog Stereo".into(),
            kind: DeviceKind::Input,
            device_id: Some(40),
            ..Default::default()
        }),
        Event::Props {
            id: 46,
            channel_volumes: vec![1.0, 1.0],
            volume_base: MIC_BASE,
            muted: false,
        },
        Event::Node(NodeInfo {
            id: 47,
            node_name: "bluez_output.AA_BB_CC_DD_EE_FF.1".into(),
            description: "WH-1000XM4".into(),
            kind: DeviceKind::Output,
            device_id: Some(60),
            address: Some("AA:BB:CC:DD:EE:FF".into()),
            codec: Some("ldac".into()),
            profile: Some("a2dp-sink".into()),
            route_device: None,
        }),
        Event::Props {
            id: 47,
            channel_volumes: vec![0.3, 0.3],
            volume_base: 1.0,
            muted: false,
        },
        Event::Default {
            kind: DeviceKind::Output,
            node_name: "alsa_output.pci-0000_00_1f.3.hdmi-stereo".into(),
        },
        Event::Default {
            kind: DeviceKind::Input,
            node_name: "alsa_input.pci-0000_00_1f.3.analog-stereo".into(),
        },
    ]
}

/// A service over the fixture with a fresh (never snapped) home.
pub fn service() -> Service<FakeBackend> {
    let mut s = Service::new(FakeBackend::default(), PathBuf::from("/home/t"));
    for ev in fixture() {
        s.handle(ev);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::METER_PROPS;
    use wol_core::audio::{
        AGC_SOURCE, APPMIX_SINK, Action, MICMIX_SOURCE, PRO_AUDIO_CONF, RNNOISE_SOURCE,
        SNAPPED_FILE, linear_for, percent_of,
    };

    const MIC: &str = "alsa_input.pci-0000_00_1f.3.analog-stereo";

    #[test]
    fn a_microphone_at_hardware_max_is_snapped_to_zero_db_once() {
        let s = service();
        let cmds = s.backend().taken();
        assert_eq!(
            cmds[0],
            Command::SetVolume {
                id: 46,
                linear: MIC_BASE
            },
            "the +30 dB mic goes to 0 dB"
        );
        assert_eq!(
            cmds[1],
            Command::AppendLine(PathBuf::from("/home/t").join(SNAPPED_FILE), MIC.into())
        );
        assert_eq!(
            cmds.len(),
            2,
            "outputs and the headphones are left alone: {cmds:?}"
        );
        // Same machine again, the user has since chosen 100% on purpose: nothing happens.
        let mut b = FakeBackend::default();
        b.files.push((
            PathBuf::from("/home/t").join(SNAPPED_FILE),
            format!("{MIC}\n"),
        ));
        let mut s = Service::new(b, PathBuf::from("/home/t"));
        for ev in fixture() {
            s.handle(ev);
        }
        assert!(s.backend().taken().is_empty());
    }

    #[test]
    fn snapshot_lists_outputs_then_inputs_with_defaults_and_codec() {
        let s = service();
        let snap = s.snapshot();
        let names: Vec<&str> = snap.devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            ["HDMI Audio", "WH-1000XM4", "Built-in Audio Analog Stereo"]
        );
        assert!(snap.default_output().is_some_and(|d| d.id == 45));
        let mic = snap.default_input().unwrap();
        assert_eq!((mic.id, mic.volume, mic.volume_base), (46, 1.0, MIC_BASE));
        assert_eq!(
            percent_of(mic.volume, mic.volume_base, false),
            100,
            "shown capped at 0 dB"
        );
        assert_eq!(snap.devices[1].codec.as_deref(), Some("ldac"));
        assert_eq!(
            snap.devices[0].profile.as_deref(),
            Some("output:hdmi-stereo")
        );
        assert!(!snap.auto_input_level && !snap.noise_suppression && !snap.boost);
    }

    #[test]
    fn mic_slider_stops_at_zero_db_unless_boost_is_allowed() {
        let mut s = service();
        s.backend().taken();
        s.act(&Action::MicVolume(50)).unwrap();
        s.act(&Action::MicVolume(140)).unwrap();
        let c = s.backend().taken();
        let lin = |c: &Command| match c {
            Command::SetVolume { id: 46, linear } => *linear,
            other => panic!("{other:?}"),
        };
        assert!(
            (lin(&c[0]) - MIC_BASE * 0.125).abs() < 1e-7,
            "50% is -18 dB"
        );
        assert!(
            (lin(&c[1]) - MIC_BASE).abs() < 1e-7,
            "140% is capped to 0 dB"
        );
        s.set_boost(true);
        s.act(&Action::MicVolume(140)).unwrap();
        let c = s.backend().taken();
        assert!(
            lin(&c[0]) > MIC_BASE && lin(&c[0]) < 0.1,
            "boost: above 0 dB, below max"
        );
        assert!(s.snapshot().boost);
        s.act(&Action::MicMuted(true)).unwrap();
        assert_eq!(
            s.backend().taken()[0],
            Command::SetMute {
                id: 46,
                muted: true
            }
        );
    }

    #[test]
    fn choosing_an_input_writes_the_default_metadata() {
        let mut s = service();
        s.handle(Event::Node(NodeInfo {
            id: 80,
            node_name: "alsa_input.usb-Yeti".into(),
            description: "Yeti".into(),
            kind: DeviceKind::Input,
            ..Default::default()
        }));
        s.backend().taken();
        s.act(&Action::Input(80)).unwrap();
        assert_eq!(
            s.backend().taken()[0],
            Command::SetDefault {
                kind: DeviceKind::Input,
                node_name: "alsa_input.usb-Yeti".into()
            }
        );
        assert_eq!(s.act(&Action::Input(45)), Err("not an input".into()));
        assert_eq!(s.act(&Action::Input(999)), Err("that input is gone".into()));
        s.handle(Event::Default {
            kind: DeviceKind::Input,
            node_name: "alsa_input.usb-Yeti".into(),
        });
        assert_eq!(s.snapshot().default_input().unwrap().id, 80);
        s.handle(Event::Removed(80));
        assert!(s.snapshot().default_input().is_none());
    }

    #[test]
    fn input_filters_are_pulse_modules_loaded_on_demand_and_torn_down() {
        let mut s = service();
        s.backend().taken();
        s.act(&Action::AutoInputLevel(true)).unwrap();
        let runs = s.backend().runs();
        assert_eq!(runs[0], "pactl list short modules", "old filters first");
        assert!(
            runs[1].starts_with("pactl load-module module-echo-cancel"),
            "{runs:?}"
        );
        assert!(runs[1].contains(&format!("source_master={MIC}")));
        let cmds = s.backend().taken();
        assert!(
            !cmds.iter().any(|c| matches!(c, Command::SetDefault { .. })),
            "the default waits until the filter's node exists: {cmds:?}"
        );
        // The AGC source shows up and becomes the default; the user adds noise suppression:
        // the chain is rebuilt mic -> AGC -> RNNoise, with the old AGC module unloaded.
        s.handle(Event::Node(NodeInfo {
            id: 90,
            node_name: AGC_SOURCE.into(),
            description: "WolOS Microphone".into(),
            kind: DeviceKind::Input,
            ..Default::default()
        }));
        assert_eq!(
            s.backend().taken(),
            [Command::SetDefault {
                kind: DeviceKind::Input,
                node_name: AGC_SOURCE.into()
            }]
        );
        s.handle(Event::Props {
            id: 90,
            channel_volumes: vec![1.0],
            volume_base: 1.0,
            muted: false,
        });
        s.handle(Event::Default {
            kind: DeviceKind::Input,
            node_name: AGC_SOURCE.into(),
        });
        assert!(
            s.backend().taken().is_empty(),
            "virtual devices are never snapped"
        );
        s.backend_mut().replies.push((
            "pactl list short modules".into(),
            format!(
                "536870912\tmodule-echo-cancel\tsource_name={AGC_SOURCE} source_master={MIC}\n"
            ),
        ));
        s.act(&Action::NoiseSuppression(true)).unwrap();
        let runs = s.backend().runs();
        assert_eq!(runs[1], "pactl unload-module 536870912");
        assert!(
            runs[2].contains("module-echo-cancel")
                && runs[2].contains(&format!("source_master={MIC}"))
        );
        assert!(
            runs[3].contains("module-ladspa-source")
                && runs[3].contains(&format!("master={AGC_SOURCE}"))
        );
        s.backend().taken();
        // The old AGC node (id 90) still reporting is not the new one; the RNNoise node is.
        s.handle(Event::Node(NodeInfo {
            id: 90,
            node_name: AGC_SOURCE.into(),
            description: "WolOS Microphone".into(),
            kind: DeviceKind::Input,
            ..Default::default()
        }));
        assert!(s.backend().taken().is_empty());
        s.handle(Event::Node(NodeInfo {
            id: 91,
            node_name: RNNOISE_SOURCE.into(),
            description: "WolOS Microphone (noise reduced)".into(),
            kind: DeviceKind::Input,
            ..Default::default()
        }));
        assert_eq!(
            s.backend().taken(),
            [Command::SetDefault {
                kind: DeviceKind::Input,
                node_name: RNNOISE_SOURCE.into()
            }]
        );
        assert!(s.snapshot().auto_input_level && s.snapshot().noise_suppression);
        // AGC off with noise suppression still on: RNNoise is rebuilt straight on the mic.
        s.act(&Action::AutoInputLevel(false)).unwrap();
        let runs = s.backend().runs();
        assert!(
            runs.iter().any(
                |r| r.contains("module-ladspa-source") && r.contains(&format!("master={MIC} "))
            ),
            "{runs:?}"
        );
        assert!(
            runs.iter()
                .all(|r| !r.contains("load-module module-echo-cancel")),
            "{runs:?}"
        );
        s.backend().taken();
        // Both off: the real microphone is the default again, nothing loaded.
        s.act(&Action::NoiseSuppression(false)).unwrap();
        let runs = s.backend().runs();
        assert!(
            runs.iter().all(|r| !r.contains("pactl load-module")),
            "{runs:?}"
        );
        assert_eq!(
            s.backend().taken().last().unwrap(),
            &Command::SetDefault {
                kind: DeviceKind::Input,
                node_name: MIC.into()
            }
        );
    }

    #[test]
    fn appmix_builds_the_obs_sink_and_the_mic_plus_app_mix_source() {
        let mut s = service();
        s.backend().taken();
        s.appmix(true).unwrap();
        let runs = s.backend().runs();
        assert_eq!(runs.len(), 6);
        assert!(runs[1].contains(&format!("sink_name={APPMIX_SINK}")));
        assert!(
            runs[3].contains("media.class=Audio/Source/Virtual") && runs[3].contains(MICMIX_SOURCE)
        );
        assert!(runs[5].contains(&format!("source={MIC}")));
        s.backend().taken();
        s.backend_mut().replies.push((
            "pactl list short modules".into(),
            format!("1\tmodule-null-sink\tsink_name={APPMIX_SINK}\n2\tmodule-loopback\tsource={APPMIX_SINK}.monitor\n"),
        ));
        s.appmix(false).unwrap();
        assert_eq!(
            s.backend().runs(),
            [
                "pactl list short modules",
                "pactl unload-module 1",
                "pactl unload-module 2"
            ]
        );
    }

    #[test]
    fn bluetooth_codec_goes_to_the_device_object() {
        let mut s = service();
        s.backend().taken();
        s.act(&Action::BtCodec("AA:BB:CC:DD:EE:FF".into(), "aac".into()))
            .unwrap();
        assert_eq!(
            s.backend().runs(),
            ["pw-cli set-param 60 Props { bluetoothAudioCodec: \"aac\" }"]
        );
        assert!(
            s.act(&Action::BtCodec("00:00".into(), "aac".into()))
                .is_err()
        );
        assert!(
            s.act(&Action::BtCodec(
                "AA:BB:CC:DD:EE:FF".into(),
                "aac\" }".into()
            ))
            .is_err()
        );
    }

    #[test]
    fn pro_audio_writes_one_user_file_and_applies_live() {
        let mut s = service();
        s.backend().taken();
        s.act(&Action::ProAudio {
            rate: 96000,
            quantum: 128,
        })
        .unwrap();
        let c = s.backend().taken();
        let file = PathBuf::from("/home/t").join(PRO_AUDIO_CONF);
        match &c[0] {
            Command::WriteFile(p, t) => {
                assert_eq!(p, &file);
                assert!(
                    t.contains("default.clock.rate = 96000")
                        && t.contains("default.clock.quantum = 128")
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            c[1],
            Command::Run(wol_core::audio::force_argv("clock.force-rate", 96000))
        );
        assert_eq!(
            c[2],
            Command::Run(wol_core::audio::force_argv("clock.force-quantum", 128))
        );
        assert_eq!(
            (s.snapshot().pro.rate, s.snapshot().pro.quantum),
            (96000, 128)
        );
        // Back to automatic: the file goes, the forces clear.
        s.act(&Action::ProAudio {
            rate: 0,
            quantum: 0,
        })
        .unwrap();
        let c = s.backend().taken();
        assert_eq!(c[0], Command::RemovePath(file.clone()));
        assert_eq!(
            c[2],
            Command::Run(wol_core::audio::force_argv("clock.force-quantum", 0))
        );
        assert!(
            s.act(&Action::ProAudio {
                rate: 12345,
                quantum: 0
            })
            .is_err()
        );
        assert!(
            s.act(&Action::ProAudio {
                rate: 0,
                quantum: 100
            })
            .is_err()
        );
        // The file is read back at start.
        let mut b = FakeBackend::default();
        b.files
            .push((file, wol_core::audio::pro_audio_conf(48000, 64)));
        let s = Service::new(b, PathBuf::from("/home/t"));
        assert_eq!(
            (s.snapshot().pro.rate, s.snapshot().pro.quantum),
            (48000, 64)
        );
    }

    #[test]
    fn meter_settings_and_xruns_flow_into_the_snapshot() {
        let mut s = service();
        assert!(s.handle(Event::Peak {
            device: 46,
            peak: 0.7,
            dt: 0.0
        }));
        assert_eq!(
            (s.snapshot().meter.device, s.snapshot().meter.hold),
            (46, 0.7)
        );
        assert!(s.handle(Event::Settings {
            rate: 0,
            quantum: 256
        }));
        assert!(
            !s.handle(Event::Settings {
                rate: 0,
                quantum: 256
            }),
            "unchanged"
        );
        assert!(s.handle(Event::Xruns(3)));
        assert_eq!((s.snapshot().pro.quantum, s.snapshot().pro.xruns), (256, 3));
        assert!(!s.handle(Event::Props {
            id: 999,
            channel_volumes: vec![],
            volume_base: 1.0,
            muted: false
        }));
    }

    #[test]
    fn reset_tears_everything_down_and_restarts_the_services() {
        let mut s = service();
        s.act(&Action::AutoInputLevel(true)).unwrap();
        s.backend().taken();
        s.act(&Action::ResetAudio).unwrap();
        let c = s.backend().taken();
        let home = PathBuf::from("/home/t");
        assert!(c.contains(&Command::RemovePath(home.join(".local/state/wireplumber"))));
        assert!(c.contains(&Command::RemovePath(home.join(PRO_AUDIO_CONF))));
        assert!(c.contains(&Command::RemovePath(home.join(SNAPPED_FILE))));
        let runs: Vec<String> = c
            .iter()
            .filter_map(|c| match c {
                Command::Run(a) => Some(a.join(" ")),
                _ => None,
            })
            .collect();
        assert_eq!(runs[0], "pactl list short modules");
        assert!(runs.contains(&"wpctl clear-default".to_string()));
        assert!(
            runs.last()
                .unwrap()
                .starts_with("systemctl --user restart pipewire.service")
        );
        let snap = s.snapshot();
        assert!(!snap.auto_input_level && snap.pro == Default::default());
        // Once reset, the mic snaps again when PipeWire comes back.
        s.handle(Event::Props {
            id: 46,
            channel_volumes: vec![1.0, 1.0],
            volume_base: MIC_BASE,
            muted: false,
        });
        assert_eq!(
            s.backend().taken()[0],
            Command::SetVolume {
                id: 46,
                linear: MIC_BASE
            }
        );
    }

    fn yeti() -> Event {
        Event::Node(NodeInfo {
            id: 80,
            node_name: "alsa_input.usb-Yeti".into(),
            description: "Yeti".into(),
            kind: DeviceKind::Input,
            ..Default::default()
        })
    }

    #[test]
    fn a_node_with_a_device_route_is_set_through_the_route() {
        // The ALSA shape: the mic node names its card (device.id 40) and its profile device
        // (card.profile.device 1); the card reports route 7 for that profile device.
        let mut s = Service::new(FakeBackend::default(), PathBuf::from("/home/t"));
        s.handle(Event::Route {
            device: 40,
            index: 7,
            route_device: 1,
        });
        s.handle(Event::Node(NodeInfo {
            id: 46,
            node_name: MIC.into(),
            description: "Built-in Audio Analog Stereo".into(),
            kind: DeviceKind::Input,
            device_id: Some(40),
            route_device: Some(1),
            ..Default::default()
        }));
        s.handle(Event::Props {
            id: 46,
            channel_volumes: vec![1.0, 1.0],
            volume_base: MIC_BASE,
            muted: false,
        });
        s.handle(Event::Default {
            kind: DeviceKind::Input,
            node_name: MIC.into(),
        });
        let route = |v: Option<Vec<f32>>, m: Option<bool>| Command::SetRoute {
            device: 40,
            index: 7,
            route_device: 1,
            channel_volumes: v,
            mute: m,
        };
        assert_eq!(
            s.backend().taken()[0],
            route(Some(vec![MIC_BASE, MIC_BASE]), None),
            "the snap to 0 dB moves the hardware mixer"
        );
        s.act(&Action::MicVolume(100)).unwrap();
        s.act(&Action::MicMuted(true)).unwrap();
        assert_eq!(
            s.backend().taken(),
            [
                route(Some(vec![MIC_BASE, MIC_BASE]), None),
                route(None, Some(true))
            ]
        );
        // The card switches the port: a new route index for the same profile device.
        s.handle(Event::Route {
            device: 40,
            index: 9,
            route_device: 1,
        });
        s.act(&Action::MicMuted(false)).unwrap();
        assert!(matches!(
            s.backend().taken()[0],
            Command::SetRoute { index: 9, .. }
        ));
        // The card goes away: its routes go with it, the node falls back to its Props.
        s.handle(Event::Removed(40));
        s.act(&Action::MicMuted(true)).unwrap();
        assert_eq!(
            s.backend().taken()[0],
            Command::SetMute {
                id: 46,
                muted: true
            }
        );
    }

    #[test]
    fn a_node_without_a_route_keeps_its_own_props() {
        // The fixture's nodes carry device.id but no card.profile.device (virtual, or the card
        // has not reported its routes): node Props, as before.
        let mut s = service();
        s.handle(Event::Route {
            device: 40,
            index: 7,
            route_device: 1,
        });
        s.backend().taken();
        s.act(&Action::MicVolume(100)).unwrap();
        assert_eq!(
            s.backend().taken()[0],
            Command::SetVolume {
                id: 46,
                linear: MIC_BASE
            }
        );
    }

    #[test]
    fn a_filter_that_does_not_load_is_reported_and_the_default_stays() {
        let mut s = service();
        s.backend_mut().fails.push((
            "pactl load-module module-ladspa-source".into(),
            "Failure: Module initialization failed".into(),
        ));
        s.backend().taken();
        let err = s.act(&Action::NoiseSuppression(true)).unwrap_err();
        assert!(err.contains("noise-suppression-for-voice"), "{err}");
        let snap = s.snapshot();
        assert!(!snap.noise_suppression, "the switch goes back off");
        assert_eq!(snap.error.as_deref(), Some(err.as_str()));
        let cmds = s.backend().taken();
        assert!(
            !cmds.iter().any(|c| matches!(c, Command::SetDefault { .. })),
            "the default stays on the real microphone: {cmds:?}"
        );
        // AGC loads, RNNoise does not: the AGC chain still works and becomes the default.
        s.act(&Action::AutoInputLevel(true)).unwrap();
        assert!(
            s.snapshot().error.is_none(),
            "a working action clears the notice"
        );
        let err = s.act(&Action::NoiseSuppression(true)).unwrap_err();
        assert!(err.contains("Reduce background noise"));
        let snap = s.snapshot();
        assert!(snap.auto_input_level && !snap.noise_suppression);
        s.backend().taken();
        s.handle(Event::Node(NodeInfo {
            id: 90,
            node_name: AGC_SOURCE.into(),
            kind: DeviceKind::Input,
            ..Default::default()
        }));
        assert_eq!(
            s.backend().taken(),
            [Command::SetDefault {
                kind: DeviceKind::Input,
                node_name: AGC_SOURCE.into()
            }]
        );
        // A filter source that never appears never becomes the default.
        let mut s = service();
        s.act(&Action::AutoInputLevel(true)).unwrap();
        s.backend().taken();
        s.handle(yeti());
        assert!(s.backend().taken().is_empty());
    }

    #[test]
    fn echo_cancel_that_does_not_load_switches_automatic_input_level_off() {
        let mut s = service();
        s.backend_mut().fails.push((
            "pactl load-module module-echo-cancel".into(),
            "Failure: Invalid argument".into(),
        ));
        s.backend().taken();
        let err = s.act(&Action::AutoInputLevel(true)).unwrap_err();
        assert!(err.starts_with("Automatic input level could not start"));
        assert!(
            err.ends_with("Invalid argument"),
            "the cause is kept: {err}"
        );
        assert!(!s.snapshot().auto_input_level);
    }

    #[test]
    fn picking_another_microphone_moves_the_filters_to_it() {
        let mut s = service();
        s.handle(yeti());
        s.act(&Action::AutoInputLevel(true)).unwrap();
        s.backend().taken();
        s.act(&Action::Input(80)).unwrap();
        let runs = s.backend().runs();
        assert!(
            runs.iter().any(|r| r.contains("module-echo-cancel")
                && r.contains("source_master=alsa_input.usb-Yeti")),
            "{runs:?}"
        );
        let cmds = s.backend().taken();
        assert!(
            !cmds.iter().any(|c| matches!(
                c,
                Command::SetDefault { node_name, .. } if node_name == "alsa_input.usb-Yeti"
            )),
            "no bypass to the raw microphone: {cmds:?}"
        );
        s.handle(Event::Node(NodeInfo {
            id: 92,
            node_name: AGC_SOURCE.into(),
            kind: DeviceKind::Input,
            ..Default::default()
        }));
        assert_eq!(
            s.backend().taken(),
            [Command::SetDefault {
                kind: DeviceKind::Input,
                node_name: AGC_SOURCE.into()
            }]
        );
        assert!(s.snapshot().auto_input_level);
    }

    #[test]
    fn unplugging_the_filtered_microphone_moves_or_stops_the_filters() {
        let mut s = service();
        s.handle(yeti());
        s.act(&Action::Input(80)).unwrap();
        s.act(&Action::AutoInputLevel(true)).unwrap();
        s.backend().taken();
        // The Yeti goes: the filters move to the built-in microphone.
        assert!(s.handle(Event::Removed(80)));
        let runs = s.backend().runs();
        assert!(
            runs.iter()
                .any(|r| r.contains("module-echo-cancel")
                    && r.contains(&format!("source_master={MIC}"))),
            "{runs:?}"
        );
        assert!(s.snapshot().auto_input_level);
        s.backend().taken();
        // And then the last one: everything off, with a notice.
        assert!(s.handle(Event::Removed(46)));
        let snap = s.snapshot();
        assert!(!snap.auto_input_level && !snap.noise_suppression);
        assert!(
            snap.error
                .as_deref()
                .is_some_and(|e| e.contains("unplugged"))
        );
        assert!(
            s.backend()
                .runs()
                .iter()
                .all(|r| !r.contains("load-module"))
        );
    }

    #[test]
    fn a_reset_forgets_the_filtered_microphone() {
        let mut s = service();
        s.handle(yeti());
        s.act(&Action::Input(80)).unwrap();
        s.act(&Action::ResetAudio).unwrap();
        s.handle(Event::Removed(80));
        s.backend().taken();
        // After the reset the filters take the current default, not the vanished Yeti.
        s.act(&Action::AutoInputLevel(true)).unwrap();
        assert!(
            s.backend()
                .runs()
                .iter()
                .any(|r| r.contains(&format!("source_master={MIC}")))
        );
    }

    /// A6-4: a PipeWire restart takes the pipewire-pulse modules with it; the switches stay on in
    /// the service, so the filters are loaded again once the microphone is back.
    #[test]
    fn filters_come_back_after_pipewire_restarts() {
        let mut s = service();
        s.act(&Action::AutoInputLevel(true)).unwrap();
        s.act(&Action::NoiseSuppression(true)).unwrap();
        s.backend().taken();
        s.set_connected(false);
        s.set_connected(true);
        assert!(
            s.backend().runs().is_empty(),
            "nothing to load into before the registry is back"
        );
        // The registry sends the graph again; the filters wait for their microphone.
        let mut events = fixture().into_iter();
        let before_mic = events.by_ref().take(2);
        for ev in before_mic {
            s.handle(ev);
        }
        assert!(
            s.backend()
                .runs()
                .iter()
                .all(|r| !r.contains("load-module")),
            "other nodes do not trigger it"
        );
        for ev in events {
            s.handle(ev);
        }
        let runs = s.backend().runs();
        assert!(
            runs.iter()
                .any(|r| r.contains("module-echo-cancel")
                    && r.contains(&format!("source_master={MIC}"))),
            "{runs:?}"
        );
        assert!(
            runs.iter().any(|r| r.contains("module-ladspa-source")),
            "{runs:?}"
        );
        let snap = s.snapshot();
        assert!(snap.auto_input_level && snap.noise_suppression && snap.error.is_none());
        // Once: a second description of the same microphone does not reload anything.
        s.backend().taken();
        s.handle(Event::Node(NodeInfo {
            id: 46,
            node_name: MIC.into(),
            description: "Built-in Audio Analog Stereo (renamed)".into(),
            kind: DeviceKind::Input,
            ..Default::default()
        }));
        assert!(s.backend().runs().is_empty());
    }

    #[test]
    fn a_restart_with_the_filters_off_loads_nothing_and_a_reset_cancels_the_wait() {
        let mut s = service();
        s.set_connected(false);
        s.set_connected(true);
        s.backend().taken();
        for ev in fixture() {
            s.handle(ev);
        }
        assert!(
            s.backend()
                .runs()
                .iter()
                .all(|r| !r.contains("load-module"))
        );

        s.act(&Action::AutoInputLevel(true)).unwrap();
        s.set_connected(false);
        s.set_connected(true);
        s.act(&Action::ResetAudio).unwrap();
        s.backend().taken();
        for ev in fixture() {
            s.handle(ev);
        }
        assert!(
            s.backend()
                .runs()
                .iter()
                .all(|r| !r.contains("load-module")),
            "a reset forgot the filters"
        );
    }

    /// A6-2: the output slider is the 0 dB scale, converted by the service.
    #[test]
    fn the_output_slider_sets_the_gain_from_zero_db_not_from_the_maximum() {
        let mut s = Service::new(FakeBackend::default(), PathBuf::from("/home/t"));
        let sink = "alsa_output.usb-dac";
        s.handle(Event::Node(NodeInfo {
            id: 70,
            node_name: sink.into(),
            description: "DAC".into(),
            kind: DeviceKind::Output,
            device_id: Some(50),
            route_device: Some(2),
            ..Default::default()
        }));
        s.handle(Event::Props {
            id: 70,
            channel_volumes: vec![0.125, 0.125],
            volume_base: 0.125,
            muted: false,
        });
        assert_eq!(
            s.act(&Action::OutputVolume(50)),
            Err("no output device".into()),
            "nothing is the default yet"
        );
        s.handle(Event::Default {
            kind: DeviceKind::Output,
            node_name: sink.into(),
        });
        let lin = |c: &Command| match c {
            Command::SetVolume { id: 70, linear } => *linear,
            other => panic!("{other:?}"),
        };
        s.backend().taken();
        s.act(&Action::OutputVolume(100)).unwrap();
        s.act(&Action::OutputVolume(180)).unwrap();
        let c = s.backend().taken();
        assert!(
            (lin(&c[0]) - 0.125).abs() < 1e-7,
            "100 is 0 dB, not 1.0: {c:?}"
        );
        assert!(
            (lin(&c[1]) - 0.125).abs() < 1e-7,
            "capped without Allow louder"
        );
        s.set_boost(true);
        s.act(&Action::OutputVolume(180)).unwrap();
        let c = s.backend().taken();
        assert!(lin(&c[0]) > 0.125 && lin(&c[0]) <= 1.0, "{c:?}");
        // What was set reads back as the same slider position (no jump after the drag).
        s.handle(Event::Props {
            id: 70,
            channel_volumes: vec![linear_for(60, 0.125); 2],
            volume_base: 0.125,
            muted: false,
        });
        let d = s.snapshot().default_output().cloned().unwrap();
        assert_eq!(percent_of(d.volume, d.volume_base, false), 60);
        // With the card's route known, it goes to the hardware mixer like a microphone does.
        s.handle(Event::Route {
            device: 50,
            index: 3,
            route_device: 2,
        });
        s.backend().taken();
        s.act(&Action::OutputVolume(100)).unwrap();
        assert_eq!(
            s.backend().taken()[0],
            Command::SetRoute {
                device: 50,
                index: 3,
                route_device: 2,
                channel_volumes: Some(vec![0.125, 0.125]),
                mute: None,
            }
        );
    }

    /// A6-7: `Props` before the card's `Route` must not set the node's own volume and call the
    /// microphone done; the snap waits for the route and then moves the hardware mixer.
    #[test]
    fn a_microphone_waits_for_its_route_before_the_snap() {
        let mut s = Service::new(FakeBackend::default(), PathBuf::from("/home/t"));
        s.handle(Event::Node(NodeInfo {
            id: 46,
            node_name: MIC.into(),
            description: "Built-in Audio Analog Stereo".into(),
            kind: DeviceKind::Input,
            device_id: Some(40),
            route_device: Some(1),
            ..Default::default()
        }));
        s.handle(Event::Props {
            id: 46,
            channel_volumes: vec![1.0, 1.0],
            volume_base: MIC_BASE,
            muted: false,
        });
        assert!(
            s.backend().taken().is_empty(),
            "no node SetVolume, nothing remembered"
        );
        // Another card's route is not this one's.
        s.handle(Event::Route {
            device: 99,
            index: 1,
            route_device: 1,
        });
        assert!(s.backend().taken().is_empty());
        s.handle(Event::Route {
            device: 40,
            index: 7,
            route_device: 1,
        });
        let cmds = s.backend().taken();
        assert_eq!(
            cmds[0],
            Command::SetRoute {
                device: 40,
                index: 7,
                route_device: 1,
                channel_volumes: Some(vec![MIC_BASE, MIC_BASE]),
                mute: None,
            }
        );
        assert_eq!(
            cmds[1],
            Command::AppendLine(PathBuf::from("/home/t").join(SNAPPED_FILE), MIC.into())
        );
        // Snapped once: a later route change (another port) leaves the user's choice alone.
        s.handle(Event::Route {
            device: 40,
            index: 9,
            route_device: 1,
        });
        assert!(s.backend().taken().is_empty());
    }

    /// A6-5: the level meter is not a recorder as far as the Bluetooth autoswitch can tell.
    #[test]
    fn the_meter_stream_carries_a_link_group_so_headsets_stay_in_music_mode() {
        let get = |k: &str| METER_PROPS.iter().find(|p| p.0 == k).map(|p| p.1);
        assert_eq!(get("node.name"), Some("wolos-meter"));
        assert!(
            get("node.link-group").is_some_and(|g| !g.is_empty()),
            "WirePlumber's autoswitch skips capture streams with a node.link-group"
        );
        assert_eq!(get("media.category"), Some("Capture"));
    }

    #[test]
    fn a_dropped_connection_empties_the_list_until_pipewire_is_back() {
        let mut s = service();
        s.act(&Action::ProAudio {
            rate: 0,
            quantum: 256,
        })
        .unwrap();
        s.set_connected(false);
        let snap = s.snapshot();
        assert!(!snap.connected && snap.devices.is_empty());
        assert_eq!(snap.pro.quantum, 256, "the saved Pro Audio state is kept");
        s.set_connected(true);
        for ev in fixture() {
            s.handle(ev);
        }
        let snap = s.snapshot();
        assert!(snap.connected && snap.devices.len() == 3);
    }

    #[test]
    fn reconnects_back_off_one_two_four_then_eight_seconds() {
        let t0 = std::time::Instant::now();
        let mut b = crate::service::Backoff::default();
        assert!(b.due(t0), "the first try is immediate");
        let waits: Vec<u64> = (0..5).map(|_| b.failed(t0).as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 8]);
        assert!(!b.due(t0 + std::time::Duration::from_secs(7)));
        assert!(b.due(t0 + std::time::Duration::from_secs(8)));
        b.reset();
        assert_eq!(b.failed(t0).as_secs(), 1);
    }

    /// R2C-14: a backend that panics stops the service with a word to the UI, not silently.
    #[test]
    fn a_panic_in_the_service_is_reported_and_ends_the_run() {
        use crate::service::{Ended, Input, SERVICE_STOPPED, serve_guarded};
        use std::sync::mpsc;
        struct Panics;
        impl Backend for Panics {
            fn execute(&mut self, _: &Command) -> Result<String, String> {
                panic!("a bug in a backend call");
            }
            fn read_file(&self, _: &Path) -> Option<String> {
                None
            }
        }
        let (tx_in, rx_in) = mpsc::channel();
        let (tx_snap, rx_snap) = mpsc::channel();
        for ev in fixture() {
            tx_in.send(Input::Event(ev)).unwrap();
        }
        let mut s = Service::new(Panics, PathBuf::from("/home/t"));
        let ended = serve_guarded(
            &mut s,
            &rx_in,
            &tx_snap,
            std::time::Duration::from_millis(10),
            &mut |_| {},
        );
        assert_eq!(ended, Ended::Panicked);
        let last = rx_snap.try_iter().last().unwrap();
        assert!(!last.connected);
        assert_eq!(last.error.as_deref(), Some(SERVICE_STOPPED));
        // A clean stop is not a panic.
        let (tx_in, rx_in) = mpsc::channel::<Input>();
        drop(tx_in);
        let mut s = Service::new(FakeBackend::default(), PathBuf::from("/home/t"));
        let ended = serve_guarded(
            &mut s,
            &rx_in,
            &tx_snap,
            std::time::Duration::from_millis(10),
            &mut |_| {},
        );
        assert_eq!(ended, Ended::Quit);
    }

    #[test]
    fn the_service_thread_serves_requests_events_and_the_xrun_poll() {
        use crate::service::{Input, Request, serve};
        use std::sync::mpsc;
        let (tx_in, rx_in) = mpsc::channel();
        let (tx_snap, rx_snap) = mpsc::channel();
        let (tx_meter, rx_meter) = mpsc::channel();
        let t = std::thread::spawn(move || {
            let mut b = FakeBackend::default();
            b.replies.push((
                "pw-top -b -n 1".into(),
                "S   ID  QUANT   RATE    WAIT    BUSY   W/Q   B/Q  ERR FORMAT NAME\n\
                 R   46   1024  48000   0.1us   0.1us  0.00  0.00    5 F32LE mic\n"
                    .into(),
            ));
            let mut s = Service::new(b, PathBuf::from("/home/t"));
            serve(
                &mut s,
                &rx_in,
                &tx_snap,
                std::time::Duration::from_millis(10),
                &mut |on| {
                    let _ = tx_meter.send(on);
                },
            );
        });
        for ev in fixture() {
            tx_in.send(Input::Event(ev)).unwrap();
        }
        tx_in
            .send(Input::Event(Event::Peak {
                device: 0,
                peak: 0.5,
                dt: 0.0,
            }))
            .unwrap();
        tx_in
            .send(Input::Ui(Request::Act(Action::Input(999))))
            .unwrap();
        tx_in.send(Input::Ui(Request::Meter(true))).unwrap();
        let wait = std::time::Duration::from_secs(5);
        assert_eq!(rx_meter.recv_timeout(wait), Ok(true));
        let mut last = None;
        while let Ok(snap) = rx_snap.recv_timeout(wait) {
            let done = snap.pro.xruns == 5;
            last = Some(snap);
            if done {
                break;
            }
        }
        let snap = last.unwrap();
        assert_eq!(snap.pro.xruns, 5, "pw-top ran on the service thread");
        assert_eq!(
            snap.meter.device, 46,
            "the peak is named after the default input"
        );
        assert_eq!(snap.error.as_deref(), Some("that input is gone"));
        tx_in.send(Input::Connected(false)).unwrap();
        tx_in.send(Input::Ui(Request::Quit)).unwrap();
        t.join().unwrap();
        let after: Vec<_> = rx_snap.try_iter().collect();
        assert!(
            after
                .last()
                .is_some_and(|s| !s.connected && s.devices.is_empty())
        );
    }
}
