//! Scouts (milestone 8b decisions 12 to 15): read-only headless sessions that read a
//! repository and report once through `submit_scout_report`. The onboarding scout is
//! one; milestone 9's area scouts reuse all of it.
//!
//! `contract`, `spec`, `report` and `machine` are pure; `service` drives the sessions
//! through the window manager.

pub mod contract;
pub mod machine;
pub mod planner;
pub mod report;
pub mod service;
pub mod spec;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_machine;
#[cfg(test)]
mod tests_planner;
