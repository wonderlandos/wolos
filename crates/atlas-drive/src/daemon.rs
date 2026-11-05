// SPDX-License-Identifier: GPL-2.0-or-later
//! Daemon state shared by every connection: the browser (started on demand), the state directory,
//! counters for `status`, the tracing and recording handles, and the recorded verbs for `codegen`.
use crate::browser::{Browser, LaunchConfig};
use crate::page::Page;
use crate::util::{Res, Ring, err};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

pub struct Daemon {
    pub defaults: Mutex<LaunchConfig>,
    browser: Mutex<Option<Browser>>,
    pub state_dir: PathBuf,
    pub started: Instant,
    pub shots: AtomicU64,
    pub verbs_run: AtomicU64,
    pub trace: Mutex<Option<crate::investigate::TraceState>>,
    pub recorder: Mutex<Option<crate::record::Recorder>>,
    pub script: Mutex<Ring<Value>>,
    pub routes: Mutex<Vec<crate::route::Route>>,
    pub auto_launch: bool,
}

impl Daemon {
    pub fn new(defaults: LaunchConfig, state_dir: PathBuf) -> Arc<Daemon> {
        let _ = std::fs::create_dir_all(&state_dir);
        Arc::new(Daemon {
            defaults: Mutex::new(defaults),
            browser: Mutex::new(None),
            state_dir,
            started: Instant::now(),
            shots: AtomicU64::new(0),
            verbs_run: AtomicU64::new(0),
            trace: Mutex::new(None),
            recorder: Mutex::new(None),
            script: Mutex::new(Ring::new(500)),
            routes: Mutex::new(vec![]),
            auto_launch: true,
        })
    }

    fn lock_browser(&self) -> MutexGuard<'_, Option<Browser>> {
        self.browser.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The running browser; starts one with the defaults on first use.
    pub fn browser(&self) -> Res<Browser> {
        let mut g = self.lock_browser();
        if let Some(b) = g.as_ref() {
            if b.is_alive() {
                return Ok(b.clone());
            }
            b.close();
            *g = None;
        }
        if !self.auto_launch {
            return err("no_browser", "no browser is running", "run launch");
        }
        let c = {
            let d = self.defaults.lock().unwrap_or_else(PoisonError::into_inner);
            LaunchConfig {
                binary: d.binary.clone(),
                headless: d.headless,
                profile: d.profile.clone(),
                extra_args: d.extra_args.clone(),
                download_dir: d
                    .download_dir
                    .clone()
                    .or_else(|| Some(self.state_dir.join("downloads"))),
                no_sandbox: d.no_sandbox,
                disable_gpu: d.disable_gpu,
                url: d.url.clone(),
                log: d.log.clone(),
            }
        };
        let b = Browser::launch(c)?;
        *g = Some(b.clone());
        Ok(b)
    }

    pub fn launch(&self, c: LaunchConfig) -> Res<Browser> {
        let mut g = self.lock_browser();
        if let Some(b) = g.take() {
            b.close();
        }
        let mut c = c;
        if c.download_dir.is_none() {
            c.download_dir = Some(self.state_dir.join("downloads"));
        }
        let b = Browser::launch(c)?;
        *g = Some(b.clone());
        Ok(b)
    }

    /// Use an already wired browser (tests, and a launcher that holds the pipe itself).
    pub fn attach(&self, b: Browser) {
        *self.lock_browser() = Some(b);
    }

    pub fn running(&self) -> Option<Browser> {
        self.lock_browser()
            .as_ref()
            .filter(|b| b.is_alive())
            .cloned()
    }

    pub fn close_browser(&self) {
        if let Some(b) = self.lock_browser().take() {
            b.close();
        }
    }

    /// The tab a verb acts on: `params.tab` (index) or the current one.
    pub fn page(&self, params: &Value) -> Res<(Browser, Arc<Page>)> {
        let b = self.browser()?;
        let p = match params.get("tab").and_then(Value::as_u64) {
            Some(i) => b.select(i as usize)?,
            None => b.wait_first_page(crate::util::WAIT_10S)?,
        };
        Ok((b, p))
    }

    pub fn next_shot(&self) -> u64 {
        self.shots.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn remember(&self, verb: &str, params: &Value) {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"verb": verb, "params": params}));
    }
}

/// `$XDG_RUNTIME_DIR/atlas` (0700), the home of the socket, shots and downloads.
pub fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) if !d.is_empty() => PathBuf::from(d).join("atlas"),
        _ => PathBuf::from(format!("/tmp/atlas-{}", shared::sys::euid())),
    }
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join("drive.sock")
}
