//! Shared test harness and multi-machine rigs for `humatad` integration tests
//! (Opt #2: compiled once instead of 216x).

#![allow(dead_code)]

#[path = "mod.rs"]
pub mod common;
pub use common::*;

pub mod away;
pub mod away_machine;
pub mod mcp_client;
pub mod power_cut;
pub mod pr_github;
pub mod remote_rig;
pub mod shared;
pub mod trust_box;
pub mod upgrade;
pub mod upgrade_rig;
