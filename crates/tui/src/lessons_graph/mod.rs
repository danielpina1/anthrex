//! Milestone 9.15's knowledge graph page (decisions 44 to 49): its pure parts. The
//! layout (task M9.15.14) places every node in the unit square once per
//! `LessonsGraph` reply; navigation and the camera (`nav`) and the search (`search`)
//! are task M9.15.15's. Nothing here performs I/O (AGENTS.md rule 5).

pub mod layout;
pub mod nav;
pub mod search;

#[cfg(test)]
pub(crate) mod fixture;
