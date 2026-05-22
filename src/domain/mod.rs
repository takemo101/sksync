//! Domain model and business rules.
//!
//! Keep this module independent from CLI, TUI, filesystem, and JSON I/O concerns.

#![allow(dead_code)]

pub mod agent;
pub mod link_plan;
pub mod lockfile;
pub mod removal;
pub mod scope;
pub mod skill;
pub mod skill_manifest;
pub mod source;
pub mod target;
