//! Agent adapters (trait `AgentAdapter`). Each adapter stays under ~100 lines and keeps
//! its vendor paths in one constant block.

pub mod claude;
pub mod generic;
