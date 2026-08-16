mod managed;
mod runner;
#[cfg(test)]
mod runner_tests;

pub use managed::{ManagedProcessSnapshot, ManagedProcessStore, format_managed_process};
pub(crate) use runner::cap_text;
pub(crate) use runner::scrub_env;
pub use runner::{
    ProcessInput, ProcessResult, assert_process_allowed, format_process_result, run_process,
};
