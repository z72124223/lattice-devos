//! Pure, deterministic LATTICE Policy Engine V2.

mod checks;
mod decimal;
mod decision;
mod evaluate;
mod matrix;
mod types;
pub mod v1_compat;

pub use decision::{DecisionKind, DecisionStage, PolicyDecision, PolicyEvidence, PolicyReason};
pub use evaluate::evaluate;
pub use types::*;

/// Public Policy Engine contract version.
pub const POLICY_CONTRACT_VERSION: u16 = 2;
