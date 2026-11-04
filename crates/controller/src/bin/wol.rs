// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol`: WolOS console. Thin main; logic in `controller::{cli, run}`.
fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = match controller::cli::parse(&argv) {
        Ok(inv) => controller::run::execute(&inv, &mut controller::run::RealEnv),
        Err(e) => {
            // A word no command has: a near spelling is in `e`; else the command that means it.
            let hint = match &e {
                controller::cli::ParseError::Unknown {
                    word,
                    suggestion: None,
                } => {
                    controller::helpfind::unknown_first_word_hint(&argv, word, wolsem::installed())
                }
                _ => None,
            };
            match hint {
                Some(h) => eprintln!("wol: {e}; {h}\nTry 'wol help'."),
                None => eprintln!("wol: {e}\n{}", controller::cli::help_hint(&argv)),
            }
            shared::errors::ErrorCode::Usage.exit()
        }
    };
    std::process::exit(code);
}
