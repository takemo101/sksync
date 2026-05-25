//! Application layer orchestration.
//!
//! This module coordinates domain logic and infrastructure ports as the CLI MVP grows.

#![allow(dead_code)]

pub mod add;
pub mod apply;
pub mod bundle;
pub mod check;
pub mod config;
pub mod discovery;
pub mod init;
pub mod list;
pub mod outdated;
pub mod plan;
pub mod ports;
pub mod source;
pub mod update;
