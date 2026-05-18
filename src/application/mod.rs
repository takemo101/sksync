//! Application layer orchestration.
//!
//! This module coordinates domain logic and infrastructure ports as the CLI MVP grows.

#![allow(dead_code)]

pub mod apply;
pub mod check;
pub mod config;
pub mod init;
pub mod list;
pub mod plan;
pub mod ports;
