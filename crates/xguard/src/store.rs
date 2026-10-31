// SPDX-License-Identifier: GPL-2.0-or-later
//! Verdicts remembered across restarts, so a clean file is never read twice: keyed by file
//! identity (no read at all) and by content (a copy of a known file needs only its hash).
//! One sparse file of fixed slots, memory-mapped: only slots in use cost memory.
use memmap2::MmapMut;
use sha2::{Digest, Sha256};
use std::path::Path;

/// A file as the kernel sees it now. Any write changes `ctime`, and no program can set it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileId {
    pub dev: u64,
    pub ino: u64,
    /// Inode generation: tells a reused inode number apart.
    pub generation: u32,
    pub size: u64,
    pub ctime: i64,
    pub ctime_ns: u32,
}

impl FileId {
    fn key(&self) -> [u8; 16] {
        let mut h = Sha256::new();
        h.update(b"id");
        h.update(self.dev.to_le_bytes());
        h.update(self.ino.to_le_bytes());
        h.update(self.generation.to_le_bytes());
        h.update(self.size.to_le_bytes());
        h.update(self.ctime.to_le_bytes());
        h.update(self.ctime_ns.to_le_bytes());
        trim(&h.finalize())
    }

    /// Of an open file.
    pub fn of(f: &std::fs::File) -> std::io::Result<Self> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::MetadataExt;
        let m = f.metadata()?;
        let mut generation: libc::c_int = 0;
        // FS_IOC_GETVERSION. Unsupported filesystems leave it 0, which only weakens inode reuse.
        // SAFETY: the ioctl writes one int to the given address.
        unsafe { libc::ioctl(f.as_raw_fd(), 0x8008_7601u64 as _, &mut generation) };
        Ok(Self {
            dev: m.dev(),
            ino: m.ino(),
            generation: generation as u32,
            size: m.size(),
            ctime: m.ctime(),
            ctime_ns: m.ctime_nsec() as u32,
        })
    }
}

fn trim(d: &[u8]) -> [u8; 16] {
    let mut k = [0; 16];
    k.copy_from_slice(&d[..16]);
    k
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    Clean = 1,
    Found = 2,
    /// The person chose Open Anyway for this content.
    Allowed = 3,
}

/// What is remembered about a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub state: State,
    /// Signature version that gave this verdict.
    pub rules: u32,
    /// The rule sets ran too, not only hashes and patterns.
    pub deep: bool,
    /// First 16 bytes of the content's SHA-256.
    pub sha: [u8; 16],
}

const SLOTS: usize = 1 << 16;
const SLOT: usize = 48;
const PROBES: usize = 8;

/// Cache file of [`SLOTS`] slots: `key[16] state rules[4] deep sha[16] pad`.
pub struct Store {
    map: MmapMut,
}

impl Store {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        f.set_len((SLOTS * SLOT) as u64)?;
        // SAFETY: root-owned file in a root-only folder, written only through this mapping.
        Ok(Self {
            map: unsafe { MmapMut::map_mut(&f)? },
        })
    }

    fn start(key: &[u8; 16]) -> usize {
        u32::from_le_bytes([key[0], key[1], key[2], key[3]]) as usize % SLOTS
    }

    fn read(&self, key: &[u8; 16]) -> Option<Entry> {
        let s = Self::start(key);
        (0..PROBES).find_map(|i| {
            let at = (s + i) % SLOTS * SLOT;
            let r = &self.map[at..at + SLOT];
            (&r[..16] == key).then_some(())?;
            Some(Entry {
                state: match r[16] {
                    1 => State::Clean,
                    2 => State::Found,
                    3 => State::Allowed,
                    _ => return None,
                },
                rules: u32::from_le_bytes([r[17], r[18], r[19], r[20]]),
                deep: r[21] != 0,
                sha: trim(&r[22..38]),
            })
        })
    }

    fn write(&mut self, key: &[u8; 16], e: Entry) {
        let s = Self::start(key);
        // The same key, else a free slot, else the last probed: a lost verdict only means a rescan.
        let free = (0..PROBES)
            .map(|i| (s + i) % SLOTS * SLOT)
            .find(|at| {
                let r = &self.map[*at..*at + 16];
                r == key || r == [0; 16]
            })
            .unwrap_or((s + PROBES - 1) % SLOTS * SLOT);
        let r = &mut self.map[free..free + SLOT];
        r[16] = 0; // not valid until complete
        r[..16].copy_from_slice(key);
        r[17..21].copy_from_slice(&e.rules.to_le_bytes());
        r[21] = u8::from(e.deep);
        r[22..38].copy_from_slice(&e.sha);
        r[16] = e.state as u8;
    }

    /// By identity: no read of the file.
    pub fn by_id(&self, id: &FileId) -> Option<Entry> {
        self.read(&id.key())
    }

    /// By another key for the same content (a kernel-held digest).
    pub fn by_key(&self, key: &[u8; 16]) -> Option<Entry> {
        self.read(key)
    }

    pub fn alias(&mut self, key: &[u8; 16], e: Entry) {
        self.write(key, e);
    }

    /// By content.
    pub fn by_sha(&self, sha: &[u8]) -> Option<Entry> {
        self.read(&trim(sha))
    }

    /// Remember for this identity and this content. The person's Open Anyway is never overwritten
    /// by a later scan of the same content.
    pub fn put(&mut self, id: &FileId, e: Entry) {
        let e = match self.read(&e.sha) {
            Some(old) if old.state == State::Allowed => Entry {
                state: State::Allowed,
                ..e
            },
            _ => e,
        };
        self.write(&id.key(), e);
        self.write(&e.sha.clone(), e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> FileId {
        FileId {
            dev: 1,
            ino: n,
            generation: 2,
            size: 3,
            ctime: 4,
            ctime_ns: 5,
        }
    }
    fn tmp(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("xguard-store-{tag}-{}", std::process::id()))
    }
    fn entry(state: State, sha: u8) -> Entry {
        Entry {
            state,
            rules: 9,
            deep: true,
            sha: [sha; 16],
        }
    }

    #[test]
    fn verdicts_survive_a_restart_by_identity_and_by_content() {
        let p = tmp("restart");
        let mut s = Store::open(&p).unwrap();
        assert_eq!(s.by_id(&id(1)), None);
        s.put(&id(1), entry(State::Clean, 7));
        drop(s);
        let s = Store::open(&p).unwrap();
        assert_eq!(s.by_id(&id(1)), Some(entry(State::Clean, 7)));
        assert_eq!(s.by_sha(&[7; 32]), Some(entry(State::Clean, 7)));
        assert_eq!(s.by_id(&id(2)), None);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn a_changed_file_is_a_new_file() {
        let p = tmp("changed");
        let mut s = Store::open(&p).unwrap();
        s.put(&id(1), entry(State::Clean, 7));
        for change in [
            FileId {
                ctime_ns: 6,
                ..id(1)
            },
            FileId { size: 9, ..id(1) },
            FileId {
                generation: 3,
                ..id(1)
            },
        ] {
            assert_eq!(s.by_id(&change), None);
        }
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn open_anyway_outlives_a_rescan() {
        let p = tmp("allow");
        let mut s = Store::open(&p).unwrap();
        s.put(&id(1), entry(State::Allowed, 7));
        s.put(&id(2), entry(State::Found, 7));
        assert_eq!(s.by_id(&id(2)).unwrap().state, State::Allowed);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn real_file_identity_moves_with_the_content() {
        let p = tmp("real");
        std::fs::write(&p, "one").unwrap();
        let a = FileId::of(&std::fs::File::open(&p).unwrap()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "two").unwrap();
        let b = FileId::of(&std::fs::File::open(&p).unwrap()).unwrap();
        assert_ne!(a, b);
        std::fs::remove_file(&p).ok();
    }
}
