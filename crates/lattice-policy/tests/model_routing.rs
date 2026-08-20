use lattice_policy::{
    ActualModelRoute, ModelCapability, ModelEvidenceFreshness, ModelEvidenceState,
    ModelGovernanceProfile, ModelProviderRole, ModelReasoningEffort, ModelRouteDecision,
    ModelRouteReason, ModelRouteRequest, escalate_model_route, resolve_model_route,
    verify_actual_model_route,
};

#[test]
fn same_task_set_routes_across_all_five_governance_profiles() {
    let catalog = catalog();

    for (profile, expected_model, expected_effort) in [
        (
            ModelGovernanceProfile::Watch,
            "local-watch",
            ModelReasoningEffort::Low,
        ),
        (
            ModelGovernanceProfile::Quick,
            "local-watch",
            ModelReasoningEffort::Low,
        ),
        (
            ModelGovernanceProfile::Standard,
            "cloud-standard",
            ModelReasoningEffort::Medium,
        ),
        (
            ModelGovernanceProfile::Deep,
            "cloud-deep",
            ModelReasoningEffort::High,
        ),
        (
            ModelGovernanceProfile::Critical,
            "critical-review",
            ModelReasoningEffort::XHigh,
        ),
    ] {
        let decision = resolve_model_route(&request(profile), &catalog);
        let ModelRouteDecision::Selected(route) = decision else {
            panic!("expected selected route for {profile:?}: {decision:?}");
        };

        assert_eq!(route.governance_profile, profile);
        assert_eq!(route.model_id, expected_model);
        assert_eq!(route.reasoning_effort, expected_effort);
        assert!(route.reason_codes.contains(&ModelRouteReason::RiskFloor));
        assert!(
            route
                .reason_codes
                .contains(&ModelRouteReason::CapabilityMatch)
        );
    }
}

#[test]
fn low_risk_route_uses_lowest_total_cost_candidate_that_meets_quality() {
    let decision = resolve_model_route(&request(ModelGovernanceProfile::Quick), &catalog());
    let ModelRouteDecision::Selected(route) = decision else {
        panic!("expected selected quick route");
    };

    assert_eq!(route.model_id, "local-watch");
    assert_ne!(route.model_id, "critical-review");
    assert!(
        route
            .reason_codes
            .contains(&ModelRouteReason::CostLatencyTiebreak)
    );
}

#[test]
fn critical_route_does_not_downgrade_below_quality_floor_for_cost() {
    let mut request = request(ModelGovernanceProfile::Critical);
    request.cost_ceiling_units = 1;

    let decision = resolve_model_route(&request, &catalog());

    assert_eq!(
        decision,
        ModelRouteDecision::Blocked(ModelRouteReason::BudgetExhausted)
    );
}

#[test]
fn route_budget_uses_total_calls_and_provider_call_limit() {
    let mut request = request(ModelGovernanceProfile::Standard);
    request.cost_ceiling_units = 15;

    assert_eq!(
        resolve_model_route(&request, &catalog()),
        ModelRouteDecision::Blocked(ModelRouteReason::BudgetExhausted)
    );

    request.cost_ceiling_units = 40;
    request.max_model_calls = 9;
    assert_eq!(
        resolve_model_route(&request, &catalog()),
        ModelRouteDecision::Blocked(ModelRouteReason::BudgetExhausted)
    );
}

#[test]
fn unknown_or_stale_capability_evidence_fails_closed_without_provider_selection() {
    let mut catalog = catalog();
    for candidate in &mut catalog {
        candidate.evidence_state = ModelEvidenceState::Unknown;
    }

    assert_eq!(
        resolve_model_route(&request(ModelGovernanceProfile::Standard), &catalog),
        ModelRouteDecision::Blocked(ModelRouteReason::CapabilityEvidenceMissing)
    );

    for candidate in &mut catalog {
        candidate.evidence_state = ModelEvidenceState::Verified;
        candidate.freshness = ModelEvidenceFreshness::Stale;
    }

    assert_eq!(
        resolve_model_route(&request(ModelGovernanceProfile::Standard), &catalog),
        ModelRouteDecision::Blocked(ModelRouteReason::CapabilityEvidenceStale)
    );
}

#[test]
fn unavailable_primary_uses_only_verified_compatible_standby() {
    let mut catalog = vec![capability(
        "primary",
        "codex-app",
        "retired-primary",
        ModelProviderRole::Primary,
        &[ModelGovernanceProfile::Standard],
        &[ModelReasoningEffort::Medium],
        2,
        1,
    )];
    catalog[0].available = false;
    catalog.push(capability(
        "standby",
        "codex-app",
        "compatible-standby",
        ModelProviderRole::Standby,
        &[ModelGovernanceProfile::Standard],
        &[ModelReasoningEffort::Medium],
        3,
        2,
    ));
    catalog[1].compatible_standby_for = Some("retired-primary".to_owned());

    let decision = resolve_model_route(&request(ModelGovernanceProfile::Standard), &catalog);
    let ModelRouteDecision::Selected(route) = decision else {
        panic!("expected compatible standby route");
    };

    assert_eq!(route.model_id, "compatible-standby");
    assert!(
        route
            .reason_codes
            .contains(&ModelRouteReason::StandbyCompatible)
    );

    catalog[1].compatible_standby_for = None;
    assert_eq!(
        resolve_model_route(&request(ModelGovernanceProfile::Standard), &catalog),
        ModelRouteDecision::Blocked(ModelRouteReason::NoCompatibleStandby)
    );
}

#[test]
fn verification_failure_escalates_one_profile_at_a_time_with_limits() {
    let request = request(ModelGovernanceProfile::Standard);
    let ModelRouteDecision::Selected(route) = resolve_model_route(&request, &catalog()) else {
        panic!("expected standard route");
    };

    let ModelRouteDecision::Selected(escalated) =
        escalate_model_route(&route, &request, 0, ModelRouteReason::VerificationFailed)
    else {
        panic!("expected bounded escalation");
    };
    assert_eq!(escalated.governance_profile, ModelGovernanceProfile::Deep);
    assert_eq!(
        escalated.reason_codes[0],
        ModelRouteReason::VerificationFailed
    );

    assert_eq!(
        escalate_model_route(&route, &request, 2, ModelRouteReason::VerificationFailed),
        ModelRouteDecision::Blocked(ModelRouteReason::EscalationLimitReached)
    );
}

#[test]
fn actual_runtime_setting_must_match_resolved_route() {
    let ModelRouteDecision::Selected(route) =
        resolve_model_route(&request(ModelGovernanceProfile::Standard), &catalog())
    else {
        panic!("expected route");
    };
    let actual = ActualModelRoute {
        provider_id: route.provider_id.clone(),
        provider_surface: route.provider_surface.clone(),
        model_id: route.model_id.clone(),
        model_version: route.model_version.clone(),
        reasoning_effort: route.reasoning_effort,
        evidence_state: ModelEvidenceState::Verified,
        conformance_match: true,
        source_ref: "runtime:session-effective-settings".to_owned(),
    };

    assert_eq!(
        verify_actual_model_route(&route, Some(&actual)),
        ModelRouteDecision::Selected(route.clone())
    );

    let mut mismatch = actual;
    mismatch.reasoning_effort = ModelReasoningEffort::Low;
    assert_eq!(
        verify_actual_model_route(&route, Some(&mismatch)),
        ModelRouteDecision::Blocked(ModelRouteReason::ActualSettingMismatch)
    );
    assert_eq!(
        verify_actual_model_route(&route, None),
        ModelRouteDecision::Blocked(ModelRouteReason::ActualSettingMismatch)
    );
}

#[test]
fn route_reason_codes_are_stable_wire_values() {
    assert_eq!(ModelRouteReason::RiskFloor.code(), "ROUTE_RISK_FLOOR");
    assert_eq!(
        ModelRouteReason::ActualSettingMismatch.code(),
        "ROUTE_ACTUAL_SETTING_MISMATCH"
    );
    assert_eq!(
        ModelRouteReason::NoCompatibleStandby.code(),
        "ROUTE_NO_COMPATIBLE_STANDBY"
    );
}

fn request(profile: ModelGovernanceProfile) -> ModelRouteRequest {
    ModelRouteRequest {
        task_classes: vec!["development".to_owned()],
        governance_profile: profile,
        risk_floor: profile,
        maximum_reasoning_effort: ModelReasoningEffort::XHigh,
        max_model_calls: 4,
        cost_ceiling_units: 40,
        allowed_provider_surfaces: vec!["local".to_owned(), "codex-app".to_owned()],
        reproducibility_pin: None,
        escalation_allowed: true,
        max_escalations: 2,
        independent_review_required: profile == ModelGovernanceProfile::Critical,
    }
}

fn catalog() -> Vec<ModelCapability> {
    vec![
        capability(
            "local",
            "local",
            "local-watch",
            ModelProviderRole::Primary,
            &[ModelGovernanceProfile::Watch, ModelGovernanceProfile::Quick],
            &[ModelReasoningEffort::Low],
            1,
            1,
        ),
        capability(
            "codex",
            "codex-app",
            "cloud-standard",
            ModelProviderRole::Primary,
            &[ModelGovernanceProfile::Standard],
            &[ModelReasoningEffort::Medium],
            4,
            2,
        ),
        capability(
            "codex",
            "codex-app",
            "cloud-deep",
            ModelProviderRole::Primary,
            &[ModelGovernanceProfile::Deep],
            &[ModelReasoningEffort::High],
            6,
            3,
        ),
        capability(
            "codex",
            "codex-app",
            "critical-review",
            ModelProviderRole::Primary,
            &[ModelGovernanceProfile::Critical],
            &[ModelReasoningEffort::XHigh],
            9,
            4,
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn capability(
    provider_id: &str,
    provider_surface: &str,
    model_id: &str,
    provider_role: ModelProviderRole,
    supported_profiles: &[ModelGovernanceProfile],
    supported_reasoning: &[ModelReasoningEffort],
    cost_units_per_call: u64,
    latency_units: u64,
) -> ModelCapability {
    ModelCapability {
        provider_id: provider_id.to_owned(),
        provider_surface: provider_surface.to_owned(),
        model_id: model_id.to_owned(),
        model_version: "snapshot-1".to_owned(),
        provider_role,
        task_classes_verified: true,
        tools_verified: true,
        modalities_verified: true,
        context_limits_verified: true,
        quota_verified: true,
        cost_verified: true,
        health_verified: true,
        compatible_standby_for: None,
        supported_profiles: supported_profiles.to_vec(),
        supported_reasoning: supported_reasoning.to_vec(),
        max_calls: 8,
        cost_units_per_call,
        latency_units,
        evidence_state: ModelEvidenceState::Verified,
        freshness: ModelEvidenceFreshness::Current,
        available: true,
    }
}
