//! Wolfram Task Subsystem
//!
//! Implements the hierarchical Job -> Process -> Thread container model.

#![allow(dead_code)]
#![allow(unused_imports)]

pub mod job;
pub mod process;
pub mod thread;

pub use job::{Job, JobState};
pub use process::{Process, ProcessState};
pub use thread::{Thread, ThreadState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskError {
    JobKilled,
    ProcessDead,
    OutOfMemory,
    QuotaExceeded,
}
