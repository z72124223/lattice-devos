//! Normal LATTICE composition surface for the pure coordination gate.

use lattice_orchestrator::{
    CompletionReport, CoordinationRound, WorkItem, decide_coordination_round,
};

/// Evaluates one data-only coordination round.
///
/// A returned dispatch candidate grants no execution authority. Callers may
/// pass it only into the existing governed LATTICE execution workflow.
#[must_use]
pub fn evaluate_coordination_round(
    work: &[WorkItem],
    completions: &[CompletionReport],
) -> CoordinationRound {
    decide_coordination_round(work, completions)
}
