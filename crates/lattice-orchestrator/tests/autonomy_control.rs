use lattice_orchestrator::{
    AutonomyDecision, AutonomyDecisionReason, AutonomyIntent, AutonomyIntentVersion,
    ModelRecommendation, TaskKind, VerificationRecommendation, classify_autonomy,
};
use lattice_task_domain::{RiskClass, TaskState};

fn intent(kind: TaskKind, risk: RiskClass) -> AutonomyIntent {
    AutonomyIntent {
        version: AutonomyIntentVersion::V1,
        kind,
        risk,
        execution_preapproved: true,
        requires_new_authority: false,
        irreversible_or_high_risk: false,
    }
}

#[test]
fn authorized_feature_selects_only_existing_governed_writer_and_focused_checks() {
    let receipt = classify_autonomy(intent(TaskKind::Feature, RiskClass::R1), TaskState::Draft);
    assert_eq!(
        receipt.decision,
        AutonomyDecision::Proceed {
            model: ModelRecommendation::GovernedCodexWriter,
            verification: VerificationRecommendation::FocusedChecks,
            reason: AutonomyDecisionReason::RoutineAuthorized
        }
    );
}

#[test]
fn r2_configuration_selects_build_and_focused_checks_without_a_model_claim() {
    let receipt = classify_autonomy(
        intent(TaskKind::Configuration, RiskClass::R2),
        TaskState::Preparing,
    );
    assert_eq!(
        receipt.decision,
        AutonomyDecision::Proceed {
            model: ModelRecommendation::NoModel,
            verification: VerificationRecommendation::BuildAndFocusedChecks,
            reason: AutonomyDecisionReason::RoutineAuthorized
        }
    );
}

#[test]
fn missing_decision_new_authority_and_high_risk_escalate() {
    let mut decision = intent(TaskKind::Feature, RiskClass::R0);
    decision.execution_preapproved = false;
    assert_eq!(
        classify_autonomy(decision, TaskState::Draft).decision,
        AutonomyDecision::AskUser {
            reason: AutonomyDecisionReason::NewUserDecision
        }
    );
    let mut authority = intent(TaskKind::Feature, RiskClass::R0);
    authority.requires_new_authority = true;
    assert_eq!(
        classify_autonomy(authority, TaskState::Draft).decision,
        AutonomyDecision::AskUser {
            reason: AutonomyDecisionReason::NewAuthority
        }
    );
    assert_eq!(
        classify_autonomy(intent(TaskKind::Research, RiskClass::R3), TaskState::Draft).decision,
        AutonomyDecision::AskUser {
            reason: AutonomyDecisionReason::HighRiskOrIrreversible
        }
    );
}
