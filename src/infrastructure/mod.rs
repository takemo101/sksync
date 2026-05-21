//! Infrastructure adapters.
//!
//! Filesystem, symlink, and JSON I/O implementations belong here.

#![allow(dead_code)]

pub mod builtin_agents;
pub mod fs;
pub mod git;
pub mod hash;
pub mod install;
pub mod json;
