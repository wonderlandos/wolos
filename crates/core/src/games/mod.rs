//! Games: native wine-proton facts, the `WolOS Proton` Steam compatibility tool, and the checks
//! behind `wol games doctor` (archicture/modules/games.md). Windows games run on wine-proton, a
//! native FreeBSD build; only the Steam client itself (Linux-only) uses the Linux ABI.
pub mod doctor;
pub mod notice;
pub mod steam;
pub mod wine;
