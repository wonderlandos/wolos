//! Games: Steam and wine facts and the checks behind `wol games doctor`
//! (archicture/modules/games.md). Steam is the native Arch package and manages Proton itself;
//! wine (or umu-launcher) is only for Windows programs outside Steam.
pub mod doctor;
pub mod notice;
pub mod steam;
pub mod wine;
