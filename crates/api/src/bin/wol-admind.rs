// SPDX-License-Identifier: GPL-2.0-or-later
//! wol-admind (root, wold job org.wol.admin): runs only the admin allow-list. Each connection is
//! read on its own thread within a deadline after the ADR 0033 hello; commands run one at a time
//! per lane, and no refusal delay or password check holds another account
//! (`api::admind::serve`, audit S4-7, api-security 1+2).
use api::admind::RealOs;

static OS: RealOs = RealOs;

fn main() {
    if shared::sys::euid() != 0 {
        eprintln!("wol-admind: must run as root");
        std::process::exit(shared::errors::ErrorCode::Denied.exit());
    }
    // S4-2: signals are taken on one thread; a hang-up or a broken pipe never ends the daemon.
    // Blocked before any thread exists, so every thread inherits the mask.
    api::os::block_signals();
    let signals = std::thread::Builder::new()
        .name("admind-signals".into())
        .spawn(|| {
            loop {
                if matches!(api::os::wait_signal(), libc::SIGTERM | libc::SIGINT) {
                    std::process::exit(0);
                }
            }
        });
    if signals.is_err() {
        eprintln!("wol-admind: could not start the signal thread");
        std::process::exit(shared::errors::ErrorCode::Failed.exit());
    }
    let listener = api::server::bind(std::path::Path::new(shared::admin::SOCKET), 0o666)
        .expect("bind admin socket"); // anyone may connect; peer credentials decide
    eprintln!("wol-admind: listening on {}", shared::admin::SOCKET);
    api::admind::serve(&listener, &OS);
}
