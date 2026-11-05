// SPDX-License-Identifier: BSD-2-Clause
//! `wol keyring migrate` (ADR 0054 section 7): read every item of the default collection from
//! whatever owns `org.freedesktop.secrets` on the session bus (gnome-keyring, before this keyring
//! takes the name) over the Secret Service, so they can be written into the new store. Nothing is
//! changed or removed on the old side.
use std::collections::HashMap;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zeroize::Zeroizing;

use crate::{Error, Result};

/// One item as the old service has it.
pub struct Found {
    pub label: String,
    pub attrs: std::collections::BTreeMap<String, String>,
    pub secret: Zeroizing<Vec<u8>>,
    pub content_type: String,
}

const BUS: &str = "org.freedesktop.secrets";
/// Most items read in one migration.
pub const MAX_ITEMS: usize = 65_536;

fn bus(e: zbus::Error) -> Error {
    Error::Io(format!("secret service: {e}"))
}

/// Every item of the default collection; an error when the collection is locked (unlock it first:
/// the old service is unlocked at sign-in) or when this keyring's own daemon owns the name.
pub fn read_all() -> Result<Vec<Found>> {
    let conn = Connection::session().map_err(|e| Error::Io(format!("session bus: {e}")))?;
    let dbus = zbus::blocking::fdo::DBusProxy::new(&conn).map_err(bus)?;
    dbus.get_name_owner(BUS.try_into().map_err(|_| Error::Malformed("bus name"))?)
        .map_err(|_| {
            Error::Io("nothing owns org.freedesktop.secrets: no old keyring to read".into())
        })?;
    // This keyring's own daemon also owns a name of its own (it cannot be told apart by its
    // executable: a daemon that is not dumpable hides /proc/<pid>/exe from everyone).
    if dbus
        .name_has_owner(
            crate::secret_service::OWN_NAME
                .try_into()
                .map_err(|_| Error::Malformed("bus name"))?,
        )
        .unwrap_or(false)
    {
        return Err(Error::Policy(
            "wol-keyringd already owns the Secret Service; there is nothing to migrate",
        ));
    }
    let svc = Proxy::new(
        &conn,
        BUS,
        "/org/freedesktop/secrets",
        "org.freedesktop.Secret.Service",
    )
    .map_err(bus)?;
    let (_, session): (OwnedValue, OwnedObjectPath) = svc
        .call("OpenSession", &("plain", Value::from("")))
        .map_err(bus)?;
    let coll_path: OwnedObjectPath = svc.call("ReadAlias", &("default",)).map_err(bus)?;
    if coll_path.as_str() == "/" {
        return Err(Error::NotFound);
    }
    let coll = Proxy::new(
        &conn,
        BUS,
        coll_path.as_str(),
        "org.freedesktop.Secret.Collection",
    )
    .map_err(bus)?;
    if coll.get_property::<bool>("Locked").map_err(bus)? {
        return Err(Error::Locked);
    }
    let items: Vec<OwnedObjectPath> = coll.get_property("Items").map_err(bus)?;
    if items.len() > MAX_ITEMS {
        return Err(Error::Limit("item count"));
    }
    let mut out = Vec::with_capacity(items.len());
    for p in items {
        let it = Proxy::new(&conn, BUS, p.as_str(), "org.freedesktop.Secret.Item").map_err(bus)?;
        let label: String = it.get_property("Label").map_err(bus)?;
        let attrs: HashMap<String, String> = it.get_property("Attributes").map_err(bus)?;
        let (_, _, value, content_type): (OwnedObjectPath, Vec<u8>, Vec<u8>, String) =
            it.call("GetSecret", &(session.clone(),)).map_err(bus)?;
        out.push(Found {
            label,
            attrs: attrs.into_iter().collect(),
            secret: Zeroizing::new(value),
            content_type,
        });
    }
    Ok(out)
}
