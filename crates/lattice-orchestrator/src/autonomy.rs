//! Versioned, pure autonomy-control recommendations.
//!
//! This module recommends a bounded next step. It neither invokes a model nor
//! changes task state; Policy, Task Ledger, Writer Lease, and composition keep
//! authority for those effects.

use lattice_task_domain::{RiskClass, TaskState};

/// The only supported interpretation version for this minimal control slice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyIntentVersion {
    V1,
}

/// Coarse task categories accepted by the local rule set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskKind {
    Feature,
    BugFix,
    Configuration,
    Research,
}

/// Immutable, caller-reviewed task intent supplied to the pure classifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AutonomyIntent {
    pub version: AutonomyIntentVersion,
    pub kind: TaskKind,
    pub risk: RiskClass,
    /// Whether the requested local execution boundary is already authorized.
    pub execution_preapproved: bool,
    /// Whether an effect needs authority not represented by the current task.
    pub requires_new_authority: bool,
    /// Whether the requested effect is high-risk or difficult to reverse.
    pub irreversible_or_high_risk: bool,
}

/// A recommendation, not a provider selection or model invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelRecommendation {
    GovernedCodexWriter,
    NoModel,
}

/// The minimum verification category recommended before ordinary progression.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationRecommendation {
    FocusedChecks,
    BuildAndFocusedChecks,
    ReadOnlyEvidence,
}

/// Why autonomous progression stopped or can continue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyDecisionReason {
    RoutineAuthorized,
    NewUserDecision,
    NewAuthority,
    HighRiskOrIrreversible,
}

/// Closed, explainable control recommendation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyDecision {
    Proceed {
        model: ModelRecommendation,
        verification: VerificationRecommendation,
        reason: AutonomyDecisionReason,
    },
    AskUser {
        reason: AutonomyDecisionReason,
    },
}

/// Non-durable status receipt; callers must bind it to the existing Ledger to persist it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AutonomyReceipt {
    pub version: AutonomyIntentVersion,
    pub observed_state: TaskState,
    pub decision: AutonomyDecision,
}

/// Classifies an already-understood intent with fail-closed priority.
///
/// It cannot select an unavailable model, scheduler, remote service, or a new
/// authority: the only writer recommendation is the existing governed Codex path.
#[must_use]
pub const fn classify_autonomy(
    intent: AutonomyIntent,
    observed_state: TaskState,
) -> AutonomyReceipt {
    let decision = if intent.requires_new_authority {
        AutonomyDecision::AskUser {
            reason: AutonomyDecisionReason::NewAuthority,
        }
    } else if intent.irreversible_or_high_risk || matches!(intent.risk, RiskClass::R3) {
        AutonomyDecision::AskUser {
            reason: AutonomyDecisionReason::HighRiskOrIrreversible,
        }
    } else if !intent.execution_preapproved {
        AutonomyDecision::AskUser {
            reason: AutonomyDecisionReason::NewUserDecision,
        }
    } else {
        let model = match intent.kind {
            TaskKind::Feature | TaskKind::BugFix => ModelRecommendation::GovernedCodexWriter,
            TaskKind::Configuration | TaskKind::Research => ModelRecommendation::NoModel,
        };
        let verification = match intent.kind {
            TaskKind::Research => VerificationRecommendation::ReadOnlyEvidence,
            TaskKind::Configuration | TaskKind::Feature | TaskKind::BugFix
                if matches!(intent.risk, RiskClass::R2) =>
            {
                VerificationRecommendation::BuildAndFocusedChecks
            }
            TaskKind::Configuration | TaskKind::Feature | TaskKind::BugFix => {
                VerificationRecommendation::FocusedChecks
            }
        };
        AutonomyDecision::Proceed {
            model,
            verification,
            reason: AutonomyDecisionReason::RoutineAuthorized,
        }
    };
    AutonomyReceipt {
        version: intent.version,
        observed_state,
        decision,
    }
}
