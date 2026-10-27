//! wol-admind (root, wold job org.wol.admin): runs only the admin allow-list. One client at a time.
use api::admind::{Caller, RealOs, handle, session_of};
use std::io::{Read, Write};

fn main() {
    // SAFETY: plain syscall.
    if unsafe { libc::getuid() } != 0 {
        eprintln!("wol-admind: must run as root");
        std::process::exit(shared::errors::ErrorCode::Denied.exit());
    }
    let listener = api::server::bind(std::path::Path::new(shared::admin::SOCKET), 0o666)
        .expect("bind admin socket"); // anyone may connect; peer credentials decide
    let mut cache = wol_core::admin::AuthCache::default();
    eprintln!("wol-admind: listening on {}", shared::admin::SOCKET);
    let mut buf = vec![0u8; shared::admin::MAX_REQUEST + 1];
    for mut conn in listener.incoming().flatten() {
        let caller = shared::sys::peer(&conn).map(|p| Caller {
            uid: p.uid,
            session: session_of(p.pid),
        });
        let _ = conn.set_read_timeout(Some(std::time::Duration::from_secs(5)));
        let mut n = 0;
        while n < buf.len() {
            match conn.read(&mut buf[n..]) {
                Ok(0) | Err(_) => break,
                Ok(r) => n += r,
            }
        }
        let reply = handle(&buf[..n], caller, &mut cache, &RealOs);
        buf.fill(0); // password never outlives the request
        let _ = writeln!(conn, "{}", reply.line());
    }
}
