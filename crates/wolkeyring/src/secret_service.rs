// SPDX-License-Identifier: BSD-2-Clause
//! The Secret Service subset on the session bus (ADR 0054 section 8): what libsecret's
//! `secret-tool`, Chromium and NetworkManager's agent use. One collection, `login`; the `plain`
//! session only (the session bus is private to one user and the bus daemon sees the message
//! either way); no prompts (a request that needs a person is refused, never granted); the caller
//! is the process the bus names, pinned through `/proc`.
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use zbus::message::Header;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, ObjectServer, fdo, interface};

use crate::daemon::{Daemon, Ident};
use crate::item::{AppId, Class, Meta};
use crate::{Error, io};

pub const BUS_NAME: &str = "org.freedesktop.secrets";
/// A second name only this daemon takes, so `wol keyring migrate` can tell it from gnome-keyring.
pub const OWN_NAME: &str = "org.wolos.Keyring";
pub const ROOT: &str = "/org/freedesktop/secrets";
pub const COLLECTION: &str = "/org/freedesktop/secrets/collection/login";
pub const ALIAS: &str = "/org/freedesktop/secrets/aliases/default";
const SESSION_DIR: &str = "/org/freedesktop/secrets/session";
const NONE: &str = "/";
/// Most open sessions, most registered items.
const MAX_SESSIONS: usize = 64;
const MAX_REGISTERED: usize = 70_000;
const LABEL: &str = "org.freedesktop.Secret.Item.Label";
const ATTRIBUTES: &str = "org.freedesktop.Secret.Item.Attributes";

type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

/// The errors of the specification, plus the bus's own.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.Secret.Error")]
pub enum SecretError {
    #[zbus(error)]
    ZBus(zbus::Error),
    IsLocked(String),
    NoSession(String),
    NoSuchObject(String),
    NotSupported(String),
}

impl From<Error> for SecretError {
    fn from(e: Error) -> Self {
        let m = e.to_string();
        match e {
            Error::Locked => SecretError::IsLocked(m),
            Error::NotFound => SecretError::NoSuchObject(m),
            Error::Denied => {
                SecretError::ZBus(zbus::Error::FDO(Box::new(fdo::Error::AccessDenied(m))))
            }
            other => SecretError::ZBus(zbus::Error::FDO(Box::new(fdo::Error::Failed(
                other.to_string(),
            )))),
        }
    }
}

/// The object path of item `id`.
pub fn item_path(id: &str) -> OwnedObjectPath {
    ObjectPath::try_from(format!("{COLLECTION}/{id}"))
        .map(Into::into)
        .unwrap_or_else(|_| none())
}

/// The item id of an object path under the collection, or None.
pub fn item_id(path: &str) -> Option<String> {
    let id = path.strip_prefix(COLLECTION)?.strip_prefix('/')?;
    (id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())).then(|| id.to_string())
}

fn prop(e: Error) -> fdo::Error {
    fdo::Error::Failed(e.to_string())
}

fn none() -> OwnedObjectPath {
    ObjectPath::from_static_str_unchecked(NONE).into()
}

fn path(s: &str) -> OwnedObjectPath {
    ObjectPath::try_from(s.to_string())
        .map(Into::into)
        .unwrap_or_else(|_| none())
}

struct Shared {
    d: Arc<Daemon>,
    sessions: Mutex<HashSet<String>>,
    registered: Mutex<HashSet<String>>,
    next: AtomicU64,
}

impl Shared {
    fn has_session(&self, p: &str) -> bool {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(p)
    }

    /// Makes the item objects for `ids` exist on the bus (they are made when first named).
    async fn register(self: &Arc<Self>, server: &ObjectServer, ids: &[String]) {
        for id in ids {
            let fresh = {
                let mut r = self.registered.lock().unwrap_or_else(|e| e.into_inner());
                r.len() < MAX_REGISTERED && r.insert(id.clone())
            };
            if fresh {
                let _ = server
                    .at(
                        item_path(id),
                        ItemObj {
                            sh: self.clone(),
                            id: id.clone(),
                        },
                    )
                    .await;
            }
        }
    }

    /// (unlocked, locked) item paths for a search.
    fn split(&self, metas: &[Meta]) -> (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) {
        let full = self.d.status().map(|(u, _)| u).unwrap_or(false);
        let (mut open, mut shut) = (Vec::new(), Vec::new());
        for m in metas {
            if full || m.class == Class::AfterFirstUnlock {
                open.push(item_path(&m.id));
            } else {
                shut.push(item_path(&m.id));
            }
        }
        (open, shut)
    }
}

/// The caller the bus names for `hdr`, pinned and identified; None when the bus cannot say or the
/// process is another user's. The pid comes from the bus, is pinned through `/proc`, and the bus
/// is asked again: a connection that still has this pid after the pin is the process that was
/// pinned (a pid is not reused while its connection is open). Nothing here waits for the keyring's
/// lock.
async fn caller(sh: &Shared, conn: &Connection, hdr: &Header<'_>) -> Option<Ident> {
    let sender = hdr.sender()?.clone();
    let proxy = fdo::DBusProxy::new(conn).await.ok()?;
    let c = proxy
        .get_connection_credentials(sender.clone().into())
        .await
        .ok()?;
    let uid = c.unix_user_id()?;
    if uid != shared::sys::euid() {
        return None;
    }
    let pid = c.process_id()?;
    let p = io::Pinned::pid(i32::try_from(pid).ok()?, uid)?;
    let again = proxy.get_connection_credentials(sender.into()).await.ok()?;
    if again.process_id() != Some(pid) {
        return None;
    }
    sh.d.identify(p)
}

/// Runs `f` for the identified caller. The process must still be the program that was identified
/// before the work and after it; a result produced for a process that changed (executed another
/// program) in between is dropped, not delivered. The window that remains is the one between this
/// check and the bus sending the reply (ADR 0054 section 4).
fn run<T>(
    who: &Option<Ident>,
    f: impl FnOnce(Option<&AppId>) -> Result<T, Error>,
) -> Result<T, Error> {
    if who.as_ref().is_some_and(|i| !i.unchanged()) {
        return Err(Error::Denied);
    }
    let r = f(who.as_ref().map(|i| &i.app));
    if who.as_ref().is_some_and(|i| !i.unchanged()) {
        return Err(Error::Denied);
    }
    r
}

fn attrs_of(v: Option<&OwnedValue>) -> Option<std::collections::BTreeMap<String, String>> {
    let m: HashMap<String, String> = v?.try_clone().ok()?.try_into().ok()?;
    Some(m.into_iter().collect())
}

fn text_of(v: Option<&OwnedValue>) -> Option<String> {
    String::try_from(v?.try_clone().ok()?).ok()
}

struct Svc(Arc<Shared>);

#[interface(name = "org.freedesktop.Secret.Service")]
impl Svc {
    async fn open_session(
        &self,
        algorithm: String,
        _input: Value<'_>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<(OwnedValue, OwnedObjectPath), SecretError> {
        if algorithm != "plain" {
            return Err(SecretError::NotSupported(
                "only the plain session is offered".into(),
            ));
        }
        let p = format!(
            "{SESSION_DIR}/s{}",
            self.0.next.fetch_add(1, Ordering::SeqCst)
        );
        {
            let mut s = self.0.sessions.lock().unwrap_or_else(|e| e.into_inner());
            if s.len() >= MAX_SESSIONS {
                return Err(SecretError::NotSupported("too many open sessions".into()));
            }
            s.insert(p.clone());
        }
        let _ = server
            .at(
                path(&p),
                SessionObj {
                    sh: self.0.clone(),
                    path: p.clone(),
                },
            )
            .await;
        let out = OwnedValue::try_from(Value::from("")).map_err(|e| SecretError::ZBus(e.into()))?;
        Ok((out, path(&p)))
    }

    async fn create_collection(
        &self,
        _properties: HashMap<String, OwnedValue>,
        alias: String,
    ) -> Result<(OwnedObjectPath, OwnedObjectPath), SecretError> {
        if alias == "default" || alias == "login" {
            return Ok((path(COLLECTION), none()));
        }
        Err(SecretError::NotSupported(
            "WolOS keeps one collection, login".into(),
        ))
    }

    async fn search_items(
        &self,
        attributes: HashMap<String, String>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<(Vec<OwnedObjectPath>, Vec<OwnedObjectPath>), SecretError> {
        let q = attributes.into_iter().collect();
        let metas = match self.0.d.search(&q) {
            Ok(m) => m,
            Err(Error::Locked) | Err(Error::NotFound) => return Ok((vec![], vec![])),
            Err(e) => return Err(e.into()),
        };
        let ids: Vec<String> = metas.iter().map(|m| m.id.clone()).collect();
        self.0.register(server, &ids).await;
        Ok(self.0.split(&metas))
    }

    /// Nothing here can prompt; objects are reported unlocked only when the keyring is.
    async fn unlock(
        &self,
        objects: Vec<OwnedObjectPath>,
    ) -> Result<(Vec<OwnedObjectPath>, OwnedObjectPath), SecretError> {
        let open = self.0.d.status().map(|(u, _)| u).unwrap_or(false);
        Ok((if open { objects } else { vec![] }, none()))
    }

    async fn lock(
        &self,
        objects: Vec<OwnedObjectPath>,
    ) -> Result<(Vec<OwnedObjectPath>, OwnedObjectPath), SecretError> {
        self.0.d.lock("secret service")?;
        Ok((objects, none()))
    }

    async fn get_secrets(
        &self,
        items: Vec<OwnedObjectPath>,
        session: OwnedObjectPath,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
    ) -> Result<HashMap<OwnedObjectPath, Secret>, SecretError> {
        if !self.0.has_session(session.as_str()) {
            return Err(SecretError::NoSession("open a session first".into()));
        }
        let who = caller(&self.0, conn, &hdr).await;
        let mut out = HashMap::new();
        for p in items {
            let Some(id) = item_id(p.as_str()) else {
                continue;
            };
            match run(&who, |a| self.0.d.get(a, &id)) {
                Ok((s, ct)) => {
                    out.insert(p, (session.clone(), vec![], s.to_vec(), ct));
                }
                Err(Error::Locked) => {
                    return Err(SecretError::IsLocked("the keyring is locked".into()));
                }
                Err(_) => {}
            }
        }
        Ok(out)
    }

    async fn read_alias(&self, name: String) -> OwnedObjectPath {
        if name == "default" || name == "login" {
            path(COLLECTION)
        } else {
            none()
        }
    }

    async fn set_alias(
        &self,
        _name: String,
        _collection: OwnedObjectPath,
    ) -> Result<(), SecretError> {
        Err(SecretError::NotSupported("aliases are fixed".into()))
    }

    #[zbus(property)]
    async fn collections(&self) -> Vec<OwnedObjectPath> {
        vec![path(COLLECTION)]
    }
}

struct Coll(Arc<Shared>);

#[interface(name = "org.freedesktop.Secret.Collection")]
impl Coll {
    async fn delete(&self) -> Result<OwnedObjectPath, SecretError> {
        Err(SecretError::NotSupported(
            "the login collection stays".into(),
        ))
    }

    async fn search_items(
        &self,
        attributes: HashMap<String, String>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<Vec<OwnedObjectPath>, SecretError> {
        let q = attributes.into_iter().collect();
        let metas = match self.0.d.search(&q) {
            Ok(m) => m,
            Err(Error::Locked) | Err(Error::NotFound) => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        let ids: Vec<String> = metas.iter().map(|m| m.id.clone()).collect();
        self.0.register(server, &ids).await;
        Ok(ids.iter().map(|i| item_path(i)).collect())
    }

    async fn create_item(
        &self,
        properties: HashMap<String, OwnedValue>,
        secret: Secret,
        replace: bool,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<(OwnedObjectPath, OwnedObjectPath), SecretError> {
        if !self.0.has_session(secret.0.as_str()) {
            return Err(SecretError::NoSession("open a session first".into()));
        }
        let label = text_of(properties.get(LABEL)).unwrap_or_default();
        let attrs = attrs_of(properties.get(ATTRIBUTES)).unwrap_or_default();
        if !replace
            && self
                .0
                .d
                .search(&attrs)
                .is_ok_and(|v| v.iter().any(|m| m.attrs == attrs))
        {
            return Err(SecretError::NotSupported(
                "an item with these attributes exists; ask to replace it".into(),
            ));
        }
        let who = caller(&self.0, conn, &hdr).await;
        let id = run(&who, |a| {
            self.0
                .d
                .set(a, &label, attrs, &secret.2, Class::WhenUnlocked)
        })?;
        self.0.register(server, std::slice::from_ref(&id)).await;
        Ok((item_path(&id), none()))
    }

    #[zbus(property)]
    async fn items(&self, #[zbus(object_server)] server: &ObjectServer) -> Vec<OwnedObjectPath> {
        let metas = self.0.d.search(&Default::default()).unwrap_or_default();
        let ids: Vec<String> = metas.iter().map(|m| m.id.clone()).collect();
        self.0.register(server, &ids).await;
        ids.iter().map(|i| item_path(i)).collect()
    }

    #[zbus(property)]
    async fn label(&self) -> String {
        "Login".into()
    }

    #[zbus(property)]
    async fn locked(&self) -> bool {
        !self.0.d.status().map(|(u, _)| u).unwrap_or(false)
    }

    #[zbus(property)]
    async fn created(&self) -> u64 {
        0
    }

    #[zbus(property)]
    async fn modified(&self) -> u64 {
        io::now()
    }
}

struct ItemObj {
    sh: Arc<Shared>,
    id: String,
}

#[interface(name = "org.freedesktop.Secret.Item")]
impl ItemObj {
    async fn delete(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<OwnedObjectPath, SecretError> {
        let who = caller(&self.sh, conn, &hdr).await;
        run(&who, |a| self.sh.d.delete(a, &self.id))?;
        self.sh
            .registered
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
        let _ = server.remove::<ItemObj, _>(item_path(&self.id)).await;
        Ok(none())
    }

    async fn get_secret(
        &self,
        session: OwnedObjectPath,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
    ) -> Result<Secret, SecretError> {
        if !self.sh.has_session(session.as_str()) {
            return Err(SecretError::NoSession("open a session first".into()));
        }
        let who = caller(&self.sh, conn, &hdr).await;
        let (s, ct) = run(&who, |a| self.sh.d.get(a, &self.id))?;
        Ok((session, vec![], s.to_vec(), ct))
    }

    async fn set_secret(
        &self,
        secret: Secret,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
    ) -> Result<(), SecretError> {
        if !self.sh.has_session(secret.0.as_str()) {
            return Err(SecretError::NoSession("open a session first".into()));
        }
        let who = caller(&self.sh, conn, &hdr).await;
        run(&who, |a| self.sh.d.replace(a, &self.id, &secret.2))?;
        Ok(())
    }

    #[zbus(property)]
    async fn label(&self) -> fdo::Result<String> {
        Ok(self.sh.d.meta(&self.id).map_err(prop)?.label)
    }

    #[zbus(property)]
    async fn attributes(&self) -> fdo::Result<HashMap<String, String>> {
        Ok(self
            .sh
            .d
            .meta(&self.id)
            .map_err(prop)?
            .attrs
            .into_iter()
            .collect())
    }

    #[zbus(property)]
    async fn locked(&self) -> bool {
        let full = self.sh.d.status().map(|(u, _)| u).unwrap_or(false);
        !full
            && self
                .sh
                .d
                .meta(&self.id)
                .map(|m| m.class == Class::WhenUnlocked)
                .unwrap_or(true)
    }

    #[zbus(property)]
    async fn created(&self) -> fdo::Result<u64> {
        Ok(self.sh.d.meta(&self.id).map_err(prop)?.created)
    }

    #[zbus(property)]
    async fn modified(&self) -> fdo::Result<u64> {
        Ok(self.sh.d.meta(&self.id).map_err(prop)?.modified)
    }
}

struct SessionObj {
    sh: Arc<Shared>,
    path: String,
}

#[interface(name = "org.freedesktop.Secret.Session")]
impl SessionObj {
    async fn close(&self, #[zbus(object_server)] server: &ObjectServer) {
        self.sh
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.path);
        let _ = server.remove::<SessionObj, _>(path(&self.path)).await;
    }
}

/// Takes the bus name and serves the subset until the connection is dropped. Fails when another
/// process (gnome-keyring) owns `org.freedesktop.secrets`.
pub async fn serve(d: Arc<Daemon>) -> zbus::Result<Connection> {
    let sh = Arc::new(Shared {
        d,
        sessions: Mutex::new(HashSet::new()),
        registered: Mutex::new(HashSet::new()),
        next: AtomicU64::new(1),
    });
    zbus::connection::Builder::session()?
        .name(BUS_NAME)?
        .name(OWN_NAME)?
        .serve_at(ROOT, Svc(sh.clone()))?
        .serve_at(COLLECTION, Coll(sh.clone()))?
        .serve_at(ALIAS, Coll(sh))?
        .build()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_paths_round_trip() {
        let id = "00112233445566778899aabbccddeeff";
        let p = item_path(id);
        assert_eq!(
            p.as_str(),
            "/org/freedesktop/secrets/collection/login/00112233445566778899aabbccddeeff"
        );
        assert_eq!(item_id(p.as_str()).as_deref(), Some(id));
        assert_eq!(
            item_id("/org/freedesktop/secrets/collection/login/zz"),
            None
        );
        assert_eq!(
            item_id("/org/freedesktop/secrets/collection/other/00112233445566778899aabbccddeeff"),
            None
        );
    }
}
