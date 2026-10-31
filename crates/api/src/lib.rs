//! wold (job supervisor, control socket, event bus, QoS) and wol-admind. Mains are thin.
pub mod admind;
pub mod bus;
pub mod control;
pub mod jobfiles;
pub mod os;
pub mod portal;
pub mod packages;
pub mod qos;
pub mod reaper;
pub mod server;
pub mod supervisor;
