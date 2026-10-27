//! `wol`: WolOS console. Thin main; logic in `controller::{cli, run}`.
fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = match controller::cli::parse(&argv) {
        Ok(inv) => controller::run::execute(&inv, &mut controller::run::RealEnv),
        Err(e) => {
            eprintln!("wol: {e}\nTry 'wol help'.");
            shared::errors::ErrorCode::Usage.exit()
        }
    };
    std::process::exit(code);
}
