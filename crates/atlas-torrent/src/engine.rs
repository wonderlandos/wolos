// SPDX-License-Identifier: BSD-2-Clause
//! The torrent engine behind one module (librqbit, pinned): jobs in, rows out. Everything the
//! engine is told has been checked first: magnet links by [`crate::magnet`], `.torrent` files and
//! the metadata a magnet link fetches by [`crate::metainfo`] (bounded, confined paths), and every
//! job writes only under a fresh folder in the downloads folder ([`crate::confine::fresh_folder`]).
//!
//! Sharing is off unless the person allowed it (`Config::sharing`): the session is built with
//! uploads disabled and no listening socket, and a finished job is paused ("done, not shared").
//! Trackers and the DHT are used to find peers for the download (DHT can be turned off).
//! The session keeps its jobs in the state folder (JSON), so after the helper is killed the jobs
//! come back at the next start, check the pieces already on disk and continue from there; a
//! finished job leaves the state when the helper stops ([`Engine::forget_finished`]), so the state
//! lists only unfinished jobs and the browser starts the helper only for those (DL-SEC-08).
//!
//! What the engine may reach (DL-SEC-03): the trackers [`crate::trackers`] allows (at most 32 per
//! job, never this machine or the LAN; the `.torrent` bytes are rebuilt with only those,
//! [`metainfo::rebuild`]), and peers outside the bogon ranges (the engine's blocklist file).
use crate::confine;
use crate::magnet::Magnet;
use crate::metainfo;
use crate::proto::{ErrorCode, Job, JobState, MAX_JOBS};
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ListenerMode, ListenerOptions,
    ManagedTorrent, Session, SessionOptions, SessionPersistenceConfig, TorrentStatsState,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// Peers one job connects to at most.
pub const PEER_LIMIT: usize = 60;
/// Peers a magnet link asks for its list of files at once: each may make the engine hold up to
/// 32 MiB for the metadata it claims (librqbit `peer_info_reader`), so 8 keep that under the
/// unit's MemoryMax (DL-SEC-13).
pub const METADATA_PEER_LIMIT: usize = 8;
/// The blocklist file in the state folder.
pub const BLOCKLIST_FILE: &str = "blocklist.txt";
/// How long a restored job may take to check its pieces before the helper resumes it.
pub const RESTORE_WAIT: Duration = Duration::from_secs(120);
/// The jobs the helper paused because the browser went away (one id per line), so they continue
/// when it is back, even after the helper ended in between; a job the person paused is never in it.
pub const ABSENCE_FILE: &str = "paused-while-away.txt";
/// How long a magnet link may look for its metadata before the row says so.
pub const METADATA_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone)]
pub struct Config {
    pub downloads: PathBuf,
    pub state: PathBuf,
    pub sharing: bool,
    pub dht: bool,
    /// Trackers every job also uses (the e2e's local tracker); empty in normal use.
    pub extra_trackers: Vec<String>,
    /// Listen for incoming peers on this port (the e2e seeder); None: no listening unless sharing.
    pub listen_port: Option<u16>,
    /// Upload limit in bytes per second (the e2e seeder, to make a download last long enough).
    pub upload_bps: Option<std::num::NonZeroU32>,
    /// Allow loopback and LAN trackers and peers (tests and the e2e, `--local-peers`); otherwise the
    /// bogon blocklist is written to the state folder and given to the engine.
    pub local_peers: bool,
}

/// A magnet link waiting for its metadata (or one that failed): shown as a row too.
#[derive(Debug, Clone)]
struct Pending {
    name: String,
    error: Option<String>,
}

pub struct Engine {
    session: Arc<Session>,
    cfg: Config,
    pending: Mutex<HashMap<String, Pending>>,
    /// See [`ABSENCE_FILE`].
    away: Mutex<Vec<String>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Engine {
    pub async fn start(cfg: Config) -> Result<Arc<Engine>, String> {
        std::fs::create_dir_all(&cfg.state).map_err(|e| format!("state folder: {e}"))?;
        let blocklist_url = if cfg.local_peers {
            None
        } else {
            let f = cfg.state.join(BLOCKLIST_FILE);
            std::fs::write(&f, crate::trackers::blocklist_text())
                .map_err(|e| format!("blocklist: {e}"))?;
            Some(format!("file://{}", f.display()))
        };
        let listen = if cfg.sharing || cfg.listen_port.is_some() {
            let port = cfg.listen_port.unwrap_or(0);
            Some(ListenerOptions {
                mode: ListenerMode::TcpAndUtp,
                // only the tests' seeder (`seed`, a `tools` build: DL-SEC-17) or a sharing the person
                // allowed listens; librqbit 9.1.0's loopback-only listener is not reachable through
                // its own tracker announce, so the seeder keeps the wildcard address
                listen_addr: SocketAddr::from(([0, 0, 0, 0], port)),
                ipv4_only: true,
                ..Default::default()
            })
        } else {
            None
        };
        let opts = SessionOptions {
            dht: if cfg.dht {
                Some(librqbit::DhtSessionConfig {
                    persistence: None,
                    ..Default::default()
                })
            } else {
                None
            },
            // No fast resume: librqbit 9.1.0 keeps its piece bitfield in memory and a SIGKILL leaves the
            // stored one empty, which it then trusts (the job restarted from zero in tests/swarm.rs).
            // Without it every start checks the pieces on disk (state `checking`), so nothing is lost.
            fastresume: false,
            persistence: Some(SessionPersistenceConfig::Json {
                folder: Some(cfg.state.clone()),
            }),
            listen,
            peer_limit: Some(PEER_LIMIT),
            concurrent_init_limit: Some(2),
            runtime_worker_threads: Some(4),
            disable_upload: !cfg.sharing,
            disable_local_service_discovery: true,
            ratelimits: librqbit::limits::LimitsConfig {
                upload_bps: cfg.upload_bps,
                download_bps: None,
            },
            ipv4_only: false,
            trackers: crate::trackers::keep(
                cfg.extra_trackers.iter().map(String::as_str),
                cfg.local_peers,
            )
            .iter()
            .filter_map(|t| t.parse().ok())
            .collect(),
            blocklist_url,
            client_name_and_version: Some("Atlas".into()),
            ..Default::default()
        };
        let session = Session::new_with_opts(cfg.downloads.clone(), opts)
            .await
            .map_err(|e| format!("{e:#}"))?;
        let away = std::fs::read_to_string(cfg.state.join(ABSENCE_FILE))
            .unwrap_or_default()
            .lines()
            .filter(|l| crate::proto::valid_id(l))
            .map(str::to_string)
            .collect();
        Ok(Arc::new(Engine {
            session,
            cfg,
            pending: Mutex::new(HashMap::new()),
            away: Mutex::new(away),
        }))
    }

    pub fn sharing(&self) -> bool {
        self.cfg.sharing
    }

    fn count(&self) -> usize {
        self.session.with_torrents(|it| it.count()) + lock(&self.pending).len()
    }

    fn handle(&self, id: &str) -> Option<Arc<ManagedTorrent>> {
        let hash = librqbit_core_id(id)?;
        self.session.get(librqbit::api::TorrentIdOrHash::Hash(hash))
    }

    /// A magnet link: the row appears at once in state `metadata`; the files are checked when the
    /// metadata arrives, then the job starts.
    pub fn add_magnet(self: &Arc<Self>, m: Magnet) -> Result<String, ErrorCode> {
        if self.handle(&m.id).is_some() || lock(&self.pending).contains_key(&m.id) {
            return Ok(m.id);
        }
        if self.count() >= MAX_JOBS {
            return Err(ErrorCode::TooMany);
        }
        let name = m.name.clone().unwrap_or_else(|| "Torrent".to_string());
        lock(&self.pending).insert(m.id.clone(), Pending { name, error: None });
        let me = Arc::clone(self);
        let id = m.id.clone();
        tokio::spawn(async move {
            let r = tokio::time::timeout(METADATA_TIMEOUT, me.fetch_and_start(&m)).await;
            let err = match r {
                Ok(Ok(())) => None,
                Ok(Err(c)) => Some(c.text().to_string()),
                Err(_) => Some("No peers sent the list of files in 10 minutes".to_string()),
            };
            let mut p = lock(&me.pending);
            match err {
                None => {
                    p.remove(&m.id);
                }
                Some(e) => {
                    if let Some(x) = p.get_mut(&m.id) {
                        x.error = Some(e);
                    }
                }
            }
        });
        Ok(id)
    }

    async fn fetch_and_start(&self, m: &Magnet) -> Result<(), ErrorCode> {
        let r = self
            .session
            .add_torrent(
                AddTorrent::from_url(m.canonical()),
                Some(AddTorrentOptions {
                    list_only: true,
                    peer_limit: Some(METADATA_PEER_LIMIT),
                    ..Default::default()
                }),
            )
            .await
            .map_err(|_| ErrorCode::Engine)?;
        let lo = match r {
            AddTorrentResponse::ListOnly(lo) => lo,
            _ => return Ok(()), // already running
        };
        if !lock(&self.pending).contains_key(&m.id) {
            return Ok(()); // cancelled while looking
        }
        self.start_checked(lo.torrent_bytes.to_vec(), lo.seen_peers)
            .await
            .map(|_| ())
    }

    /// A `.torrent` file's bytes.
    pub async fn add_torrent(&self, bytes: Vec<u8>) -> Result<String, ErrorCode> {
        if self.count() >= MAX_JOBS {
            return Err(ErrorCode::TooMany);
        }
        self.start_checked(bytes, Vec::new()).await
    }

    async fn start_checked(
        &self,
        bytes: Vec<u8>,
        peers: Vec<SocketAddr>,
    ) -> Result<String, ErrorCode> {
        let sum = metainfo::check(&bytes).map_err(|e| match e {
            metainfo::MetaError::Path(_) => ErrorCode::UnsafePath,
            _ => ErrorCode::BadTorrent,
        })?;
        // only the allowed trackers reach the engine; the info (and the hash) stay the same
        let (bytes, _) =
            metainfo::rebuild(&bytes, self.cfg.local_peers).map_err(|_| ErrorCode::BadTorrent)?;
        let folder = job_folder(&self.cfg.downloads, &sum)?;
        let r = self
            .session
            .add_torrent(
                AddTorrent::from_bytes(bytes),
                Some(AddTorrentOptions {
                    overwrite: true, // a fresh folder: nothing of the person's is there to overwrite
                    output_folder: Some(folder.to_string_lossy().into_owned()),
                    initial_peers: (!peers.is_empty()).then_some(peers),
                    peer_limit: Some(PEER_LIMIT),
                    ..Default::default()
                }),
            )
            .await
            .map_err(|_| ErrorCode::Engine)?;
        let h = match r {
            AddTorrentResponse::Added(_, h) | AddTorrentResponse::AlreadyManaged(_, h) => h,
            AddTorrentResponse::ListOnly(_) => return Err(ErrorCode::Engine),
        };
        Ok(h.info_hash().as_string())
    }

    pub async fn pause(&self, id: &str) -> Result<(), ErrorCode> {
        let h = self.handle(id).ok_or(ErrorCode::NotFound)?;
        if h.is_paused() {
            return Ok(());
        }
        self.session.pause(&h).await.map_err(|_| ErrorCode::Engine)
    }

    pub async fn resume(&self, id: &str) -> Result<(), ErrorCode> {
        if let Some(p) = lock(&self.pending).get(id)
            && p.error.is_some()
        {
            return Err(ErrorCode::Engine);
        }
        let h = self.handle(id).ok_or(ErrorCode::NotFound)?;
        if h.stats().finished && !self.cfg.sharing {
            return Ok(()); // done and not shared: nothing to resume
        }
        if !h.is_paused() {
            return Ok(());
        }
        self.session
            .unpause(&h)
            .await
            .map_err(|_| ErrorCode::Engine)
    }

    pub async fn cancel(&self, id: &str, delete: bool) -> Result<(), ErrorCode> {
        if lock(&self.pending).remove(id).is_some() && self.handle(id).is_none() {
            return Ok(());
        }
        let hash = librqbit_core_id(id).ok_or(ErrorCode::NotFound)?;
        self.session
            .delete(librqbit::api::TorrentIdOrHash::Hash(hash), delete)
            .await
            .map_err(|_| ErrorCode::NotFound)
    }

    /// Every row now. A finished job is paused when sharing is off (it stops talking to peers).
    pub async fn jobs(&self) -> Vec<Job> {
        let mut out = Vec::new();
        let mut to_pause = Vec::new();
        let handles: Vec<Arc<ManagedTorrent>> = self
            .session
            .with_torrents(|it| it.map(|(_, h)| h.clone()).collect());
        {
            for h in handles {
                let s = h.stats();
                let id = h.info_hash().as_string();
                let files: Vec<String> = h
                    .with_metadata(|m| {
                        m.file_infos
                            .iter()
                            .map(|f| f.relative_filename.to_string_lossy().into_owned())
                            .collect()
                    })
                    .unwrap_or_default();
                let (peers, known, bps) = s
                    .live
                    .as_ref()
                    .map(|l| {
                        let p = &l.snapshot.peer_stats;
                        (
                            p.live,
                            p.seen,
                            (l.download_speed.mbps * 1024.0 * 1024.0) as u64,
                        )
                    })
                    .unwrap_or((0, 0, 0));
                let state = match (&s.state, s.finished) {
                    (TorrentStatsState::Error, _) => JobState::Failed,
                    (TorrentStatsState::Initializing { .. }, _) => JobState::Checking,
                    (_, true) if self.cfg.sharing && !h.is_paused() => JobState::Seeding,
                    (_, true) => JobState::Done,
                    (TorrentStatsState::Paused, false) => JobState::Paused,
                    (TorrentStatsState::Live, false) => JobState::Downloading,
                };
                if s.finished
                    && !self.cfg.sharing
                    && !h.is_paused()
                    && matches!(s.state, TorrentStatsState::Live)
                {
                    to_pause.push(Arc::clone(&h));
                }
                out.push(Job {
                    id,
                    name: h.name().unwrap_or_else(|| "Torrent".into()),
                    state,
                    have: s.progress_bytes,
                    total: s.total_bytes,
                    peers,
                    peers_known: known,
                    down_bps: if state == JobState::Downloading {
                        bps
                    } else {
                        0
                    },
                    path: top_path(h.output_folder(), &files),
                    error: s.error.clone(),
                    exts: crate::proto::exts_of(files.iter().map(String::as_str)),
                });
            }
        }
        for h in to_pause {
            let _ = self.session.pause(&h).await;
        }
        let running: Vec<String> = out.iter().map(|j| j.id.clone()).collect();
        for (id, p) in lock(&self.pending).iter() {
            if running.contains(id) {
                continue;
            }
            out.push(Job {
                id: id.clone(),
                name: p.name.clone(),
                state: if p.error.is_some() {
                    JobState::Failed
                } else {
                    JobState::Metadata
                },
                have: 0,
                total: 0,
                peers: 0,
                peers_known: 0,
                down_bps: 0,
                path: self.cfg.downloads.to_string_lossy().into_owned(),
                error: p.error.clone(),
                exts: Vec::new(),
            });
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Share an existing copy of a torrent's data (the e2e seeder, `atlas-torrent seed`; never used
    /// by the browser's helper, which only starts jobs in fresh folders).
    pub async fn seed_existing(&self, bytes: Vec<u8>, folder: &Path) -> Result<String, String> {
        metainfo::check(&bytes).map_err(|e| format!("torrent refused: {e:?}"))?;
        let r = self
            .session
            .add_torrent(
                AddTorrent::from_bytes(bytes),
                Some(AddTorrentOptions {
                    overwrite: true,
                    output_folder: Some(folder.to_string_lossy().into_owned()),
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| format!("{e:#}"))?;
        let h = r.into_handle().ok_or_else(|| "not added".to_string())?;
        Ok(h.info_hash().as_string())
    }

    pub fn local_peers(&self) -> bool {
        self.cfg.local_peers
    }

    /// True while a job needs the network: a magnet looking for its files, or a job checking or
    /// downloading (the helper ends when none does, so its DHT node and peer connections do not
    /// outlive the work: DL-SEC-08).
    pub async fn active(&self) -> bool {
        self.jobs().await.iter().any(|j| {
            matches!(
                j.state,
                JobState::Metadata | JobState::Checking | JobState::Downloading | JobState::Seeding
            )
        })
    }

    /// Pause every running job at once (the browser is gone: no new peer connections while the
    /// helper waits for it, DL-SEC-08); they are remembered in [`ABSENCE_FILE`].
    pub async fn pause_for_absence(&self) {
        let handles: Vec<Arc<ManagedTorrent>> = self
            .session
            .with_torrents(|it| it.map(|(_, h)| h.clone()).collect());
        let mut paused = Vec::new();
        for h in handles {
            if !h.is_paused() && self.session.pause(&h).await.is_ok() {
                paused.push(h.info_hash().as_string());
            }
        }
        let mut away = lock(&self.away);
        for id in paused {
            if !away.contains(&id) {
                away.push(id);
            }
        }
        let _ = std::fs::write(self.cfg.state.join(ABSENCE_FILE), away.join("\n"));
    }

    /// The browser is back: the jobs [`Engine::pause_for_absence`] paused continue.
    pub async fn resume_after_absence(&self) {
        let ids: Vec<String> = std::mem::take(&mut *lock(&self.away));
        let _ = std::fs::remove_file(self.cfg.state.join(ABSENCE_FILE));
        for id in ids {
            let Some(h) = self.handle(&id) else {
                continue;
            };
            // a job restored from the state is still checking its pieces: it pauses when that
            // ends, so it is resumed after it
            let session = Arc::clone(&self.session);
            tokio::spawn(async move {
                let _ = tokio::time::timeout(RESTORE_WAIT, h.wait_until_initialized()).await;
                if h.is_paused() && !h.stats().finished {
                    let _ = session.unpause(&h).await;
                }
            });
        }
    }

    /// Forget every finished job that is not shared (its files stay in the downloads folder): the
    /// state then lists only unfinished jobs, so the browser has no reason to start the helper for
    /// them at its next start (DL-SEC-08).
    pub async fn forget_finished(&self) {
        let done: Vec<Arc<ManagedTorrent>> = self.session.with_torrents(|it| {
            it.filter(|(_, h)| h.stats().finished)
                .map(|(_, h)| h.clone())
                .collect()
        });
        if self.cfg.sharing {
            return;
        }
        for h in done {
            let _ = self
                .session
                .delete(librqbit::api::TorrentIdOrHash::Hash(h.info_hash()), false)
                .await;
        }
    }

    pub async fn stop(&self) {
        self.forget_finished().await;
        self.session.stop().await;
    }
}

/// The folder a job writes into: always a fresh folder in the downloads folder, named after the
/// torrent (a one-file torrent's folder is named after the file without its extension), never the
/// downloads folder itself and never a hidden name (DL-SEC-05: a torrent named `.bash_profile` or
/// `.ssh` cannot land in a home folder chosen as the downloads folder).
pub fn job_folder(downloads: &Path, sum: &metainfo::Summary) -> Result<PathBuf, ErrorCode> {
    if sum.name.starts_with('.') {
        return Err(ErrorCode::UnsafePath);
    }
    let single = sum.files.len() == 1 && sum.files[0].0 == [sum.name.clone()];
    let folder_name = match sum.name.rsplit_once('.') {
        Some((stem, _)) if single && !stem.is_empty() => stem,
        _ => sum.name.as_str(),
    };
    confine::fresh_folder(downloads, folder_name).map_err(|_| ErrorCode::UnsafePath)
}

/// What "Open" and "Show in Files" point at: the file of a one-file torrent, otherwise the job's
/// folder.
pub fn top_path(output: &Path, files: &[String]) -> String {
    match files {
        [one] if confine::component(one.as_bytes()).is_ok() => {
            output.join(one).to_string_lossy().into_owned()
        }
        _ => output.to_string_lossy().into_owned(),
    }
}

fn librqbit_core_id(id: &str) -> Option<librqbit::dht::Id20> {
    if !crate::proto::valid_id(id) {
        return None;
    }
    <librqbit::dht::Id20 as std::str::FromStr>::from_str(id).ok()
}
