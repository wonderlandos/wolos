//! `wol` console, `wol-login`, `wol-authcheck`: pure logic here, thin mains in `src/bin/`.
pub mod approve;
pub mod authcheck;
pub mod catalog;
pub mod cli;
pub mod games;
pub mod hw;
pub mod login;
pub mod mcp;
pub mod packages;
pub mod pam;
pub mod power;
pub mod run;
pub mod security;
pub mod verbs;
pub mod widgets;
