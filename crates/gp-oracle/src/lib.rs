//! The Mesen oracle (Milestone 1, step 5): turning a Mesen 2 capture into a
//! recording, comparing two recordings field by field, and the triage of
//! what differs. docs/oracle.md describes the whole procedure.
//!
//! Mesen is GPL-3.0 and is never linked: `gp oracle record` runs it in its
//! own Docker image and reads the text file its Lua script writes
//! ([`capture`]). This crate only parses that file.

pub mod capture;
pub mod diff;
pub mod mesen;
pub mod states;
pub mod triage;
