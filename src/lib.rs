pub mod agent;
pub mod bus;
pub mod channels;
pub mod config;
pub mod cron;
pub mod health;
pub mod heartbeat;
pub mod memory;
pub mod pairing;
pub mod providers;
pub mod service;
pub mod session;
pub mod skills;
pub mod tools;
pub mod utils;
pub mod webui;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
