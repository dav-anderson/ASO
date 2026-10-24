//! aso: runs one Service Point's own services from one settings file.
//!
//! air_server (behind nginx), fsp-mint and ecash-wallet are separate programs;
//! aso installs them, writes their settings from `aso.toml`, starts them in
//! order, watches them, upgrades them and redeems the ESP the service earns.
//! It links none of the family's crates: it runs the programs and reads what
//! they print.

pub use air_minterface::{exec, logs, schedule, template};

pub mod ask;
pub mod audit;
pub mod binary;
pub mod cli;
pub mod commands;
pub mod config;
pub mod ctl;
pub mod edit;
pub mod files;
pub mod fsp;
pub mod generate;
pub mod httpc;
pub mod install;
pub mod interface;
pub mod outcome;
pub mod program;
pub mod redeem;
pub mod service;
pub mod state;
pub mod status;
pub mod versions;
pub mod wallet;
