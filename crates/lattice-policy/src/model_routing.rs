//! Pure deterministic model and reasoning-effort routing for Issue #8.

/// Model-routing policy contract version.
pub const MODEL_ROUTING_CONTRACT_VERSION: u16 = 1;

/// Stable governance profile selected before any provider/model name.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ModelGovernanceProfile {
    Watch,
    Quick,
    Standard,
    Deep,
    Critical,
}

impl ModelGovernanceProfile {
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::Watch => 0,
            Self::Quick => 1,
            Self::Standard => 2,
            Self::Deep => 3,
            Self::Critical => 4,
        }
    }

    #[must_use]
    pub const fn next(self) -> Option<Self> {
        match self {
            Self::Watch => Some(Self::Quick),
            Self::Quick => Some(Self::Standard),
            Self::Standard => Some(Self::Deep),
            Self::Deep => Some(Self::Critical),
            Self::Critical => None,
        }
    }
}

/// Provider-neutral reasoning-effort ladder.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ModelReasoningEffort {
    Low,
    Medium,
    High,
    XHigh,
    Max,
    Ultra,
}

/// Provider role in a capability catalog.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelProviderRole {
    Primary,
    Standby,
    Retired,
}

/// Capability evidence state. Only verified evidence can route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelEvidenceState {
    Verified,
    Unverified,
    Unknown,
}

/// Freshness state for one capability snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelEvidenceFreshness {
    Current,
    Pinned,
    Stale,
    Unknown,
}

/// Stable reason codes for route selection, escalation, and fail-closed blocks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelRouteReason {
    ReproducibilityPin,
    RiskFloor,
    ProfileRequested,
    CapabilityMatch,
    PrimaryHealthy,
    StandbyCompatible,
    CostLatencyTiebreak,
    VerificationFailed,
    ScopeExpanded,
    RiskBoundaryCrossed,
    ConflictingEvidence,
    MajorReviewFinding,
    SessionAbnormalExit,
    CapabilityEvidenceMissing,
    CapabilityEvidenceStale,
    ProviderUnavailable,
    NoCompatibleStandby,
    BudgetExhausted,
    QualityFloorUnmet,
    ActualSettingMismatch,
    EscalationLimitReached,
    ProviderScopeDenied,
    IndependentReviewRequired,
    FutureTaskEvalProven,
    FutureTaskLowRiskRetry,
}

impl ModelRouteReason {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::ReproducibilityPin => "ROUTE_REPRODUCIBILITY_PIN",
            Self::RiskFloor => "ROUTE_RISK_FLOOR",
            Self::ProfileRequested => "ROUTE_PROFILE_REQUESTED",
            Self::CapabilityMatch => "ROUTE_CAPABILITY_MATCH",
            Self::PrimaryHealthy => "ROUTE_PRIMARY_HEALTHY",
            Self::StandbyCompatible => "ROUTE_STANDBY_COMPATIBLE",
            Self::CostLatencyTiebreak => "ROUTE_COST_LATENCY_TIEBREAK",
            Self::VerificationFailed => "ROUTE_VERIFICATION_FAILED",
            Self::ScopeExpanded => "ROUTE_SCOPE_EXPANDED",
            Self::RiskBoundaryCrossed => "ROUTE_RISK_BOUNDARY_CROSSED",
            Self::ConflictingEvidence => "ROUTE_CONFLICTING_EVIDENCE",
            Self::MajorReviewFinding => "ROUTE_MAJOR_REVIEW_FINDING",
            Self::SessionAbnormalExit => "ROUTE_SESSION_ABNORMAL_EXIT",
            Self::CapabilityEvidenceMissing => "ROUTE_CAPABILITY_EVIDENCE_MISSING",
            Self::CapabilityEvidenceStale => "ROUTE_CAPABILITY_EVIDENCE_STALE",
            Self::ProviderUnavailable => "ROUTE_PROVIDER_UNAVAILABLE",
            Self::NoCompatibleStandby => "ROUTE_NO_COMPATIBLE_STANDBY",
            Self::BudgetExhausted => "ROUTE_BUDGET_EXHAUSTED",
            Self::QualityFloorUnmet => "ROUTE_QUALITY_FLOOR_UNMET",
            Self::ActualSettingMismatch => "ROUTE_ACTUAL_SETTING_MISMATCH",
            Self::EscalationLimitReached => "ROUTE_ESCALATION_LIMIT_REACHED",
            Self::ProviderScopeDenied => "ROUTE_PROVIDER_SCOPE_DENIED",
            Self::IndependentReviewRequired => "ROUTE_INDEPENDENT_REVIEW_REQUIRED",
            Self::FutureTaskEvalProven => "ROUTE_FUTURE_TASK_EVAL_PROVEN",
            Self::FutureTaskLowRiskRetry => "ROUTE_FUTURE_TASK_LOW_RISK_RETRY",
        }
    }
}

/// Provider-neutral model capability and operational evidence.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelCapability {
    pub provider_id: String,
    pub provider_surface: String,
    pub model_id: String,
    pub model_version: String,
    pub provider_role: ModelProviderRole,
    pub task_classes_verified: bool,
    pub tools_verified: bool,
    pub modalities_verified: bool,
    pub context_limits_verified: bool,
    pub quota_verified: bool,
    pub cost_verified: bool,
    pub health_verified: bool,
    pub compatible_standby_for: Option<String>,
    pub supported_profiles: Vec<ModelGovernanceProfile>,
    pub supported_reasoning: Vec<ModelReasoningEffort>,
    pub max_calls: u64,
    pub cost_units_per_call: u64,
    pub latency_units: u64,
    pub evidence_state: ModelEvidenceState,
    pub freshness: ModelEvidenceFreshness,
    pub available: bool,
}

impl ModelCapability {
    #[must_use]
    pub fn supports(&self, profile: ModelGovernanceProfile, effort: ModelReasoningEffort) -> bool {
        self.supported_profiles.contains(&profile) && self.supported_reasoning.contains(&effort)
    }

    #[must_use]
    pub const fn evidence_is_admissible(&self) -> bool {
        matches!(self.evidence_state, ModelEvidenceState::Verified)
            && matches!(
                self.freshness,
                ModelEvidenceFreshness::Current | ModelEvidenceFreshness::Pinned
            )
            && self.task_classes_verified
            && self.tools_verified
            && self.modalities_verified
            && self.context_limits_verified
            && self.quota_verified
            && self.cost_verified
            && self.health_verified
    }
}

/// Complete immutable route request supplied by Policy, not by a model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelRouteRequest {
    pub task_classes: Vec<String>,
    pub governance_profile: ModelGovernanceProfile,
    pub risk_floor: ModelGovernanceProfile,
    pub maximum_reasoning_effort: ModelReasoningEffort,
    pub max_model_calls: u64,
    pub cost_ceiling_units: u64,
    pub allowed_provider_surfaces: Vec<String>,
    pub reproducibility_pin: Option<(String, String)>,
    pub escalation_allowed: bool,
    pub max_escalations: u8,
    pub independent_review_required: bool,
}

/// Deterministic route decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedModelRoute {
    pub provider_id: String,
    pub provider_surface: String,
    pub model_id: String,
    pub model_version: String,
    pub governance_profile: ModelGovernanceProfile,
    pub reasoning_effort: ModelReasoningEffort,
    pub primary_or_standby: ModelProviderRole,
    pub reason_codes: Vec<ModelRouteReason>,
}

/// Runtime/provider-observed effective setting.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActualModelRoute {
    pub provider_id: String,
    pub provider_surface: String,
    pub model_id: String,
    pub model_version: String,
    pub reasoning_effort: ModelReasoningEffort,
    pub evidence_state: ModelEvidenceState,
    pub conformance_match: bool,
    pub source_ref: String,
}

/// Fail-closed route result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelRouteDecision {
    Selected(ResolvedModelRoute),
    Blocked(ModelRouteReason),
}

#[must_use]
pub fn resolve_model_route(
    request: &ModelRouteRequest,
    catalog: &[ModelCapability],
) -> ModelRouteDecision {
    if request.independent_review_required
        && request.risk_floor == ModelGovernanceProfile::Critical
        && request.max_escalations == 0
    {
        return ModelRouteDecision::Blocked(ModelRouteReason::IndependentReviewRequired);
    }
    let profile = request.governance_profile.max(request.risk_floor);
    let effort = effort_for_profile(profile).min(request.maximum_reasoning_effort);
    if effort < effort_for_profile(profile) {
        return ModelRouteDecision::Blocked(ModelRouteReason::QualityFloorUnmet);
    }

    if let Some((provider_id, model_id)) = &request.reproducibility_pin {
        let Some(candidate) = catalog.iter().find(|candidate| {
            &candidate.provider_id == provider_id && &candidate.model_id == model_id
        }) else {
            return ModelRouteDecision::Blocked(ModelRouteReason::ProviderUnavailable);
        };
        return select_candidate(request, candidate, profile, effort, true);
    }

    let mut in_scope = catalog.iter().filter(|candidate| {
        request
            .allowed_provider_surfaces
            .contains(&candidate.provider_surface)
    });
    let Some(first_in_scope) = in_scope.next() else {
        return ModelRouteDecision::Blocked(ModelRouteReason::ProviderScopeDenied);
    };
    let candidates = std::iter::once(first_in_scope).chain(in_scope);

    let mut inadmissible = false;
    let mut unavailable_primary = false;
    let mut compatible_standby_seen = false;
    let mut best: Option<&ModelCapability> = None;
    for candidate in candidates {
        if !candidate.evidence_is_admissible() {
            inadmissible = true;
            continue;
        }
        if !candidate.available || candidate.provider_role == ModelProviderRole::Retired {
            if candidate.provider_role == ModelProviderRole::Primary {
                unavailable_primary = true;
            }
            continue;
        }
        if !candidate.supports(profile, effort) {
            continue;
        }
        if candidate.provider_role == ModelProviderRole::Standby {
            compatible_standby_seen = candidate.compatible_standby_for.is_some();
            if !compatible_standby_seen {
                continue;
            }
        }
        if !budget_fits(request, candidate) {
            return ModelRouteDecision::Blocked(ModelRouteReason::BudgetExhausted);
        }
        best = match best {
            Some(current) if route_cost_key(candidate) >= route_cost_key(current) => Some(current),
            _ => Some(candidate),
        };
    }

    if let Some(candidate) = best {
        return select_candidate(request, candidate, profile, effort, false);
    }
    if unavailable_primary && !compatible_standby_seen {
        return ModelRouteDecision::Blocked(ModelRouteReason::NoCompatibleStandby);
    }
    if inadmissible {
        let stale = catalog.iter().any(|candidate| {
            request
                .allowed_provider_surfaces
                .contains(&candidate.provider_surface)
                && matches!(
                    candidate.freshness,
                    ModelEvidenceFreshness::Stale | ModelEvidenceFreshness::Unknown
                )
        });
        return ModelRouteDecision::Blocked(if stale {
            ModelRouteReason::CapabilityEvidenceStale
        } else {
            ModelRouteReason::CapabilityEvidenceMissing
        });
    }
    ModelRouteDecision::Blocked(ModelRouteReason::ProviderUnavailable)
}

#[must_use]
pub fn escalate_model_route(
    resolved: &ResolvedModelRoute,
    request: &ModelRouteRequest,
    used_escalations: u8,
    trigger: ModelRouteReason,
) -> ModelRouteDecision {
    if !request.escalation_allowed || used_escalations >= request.max_escalations {
        return ModelRouteDecision::Blocked(ModelRouteReason::EscalationLimitReached);
    }
    if !matches!(
        trigger,
        ModelRouteReason::VerificationFailed
            | ModelRouteReason::ScopeExpanded
            | ModelRouteReason::RiskBoundaryCrossed
            | ModelRouteReason::ConflictingEvidence
            | ModelRouteReason::MajorReviewFinding
            | ModelRouteReason::SessionAbnormalExit
    ) {
        return ModelRouteDecision::Blocked(ModelRouteReason::QualityFloorUnmet);
    }
    let Some(next_profile) = resolved.governance_profile.next() else {
        return ModelRouteDecision::Blocked(ModelRouteReason::EscalationLimitReached);
    };
    let next_effort = effort_for_profile(next_profile);
    if next_effort > request.maximum_reasoning_effort {
        return ModelRouteDecision::Blocked(ModelRouteReason::QualityFloorUnmet);
    }
    let mut next = resolved.clone();
    next.governance_profile = next_profile;
    next.reasoning_effort = next_effort;
    next.reason_codes = vec![trigger, ModelRouteReason::RiskFloor];
    ModelRouteDecision::Selected(next)
}

#[must_use]
pub fn verify_actual_model_route(
    resolved: &ResolvedModelRoute,
    actual: Option<&ActualModelRoute>,
) -> ModelRouteDecision {
    let Some(actual) = actual else {
        return ModelRouteDecision::Blocked(ModelRouteReason::ActualSettingMismatch);
    };
    if actual.evidence_state != ModelEvidenceState::Verified
        || !actual.conformance_match
        || (!actual.source_ref.starts_with("provider:")
            && !actual.source_ref.starts_with("runtime:"))
        || actual.provider_id != resolved.provider_id
        || actual.provider_surface != resolved.provider_surface
        || actual.model_id != resolved.model_id
        || actual.model_version != resolved.model_version
        || actual.reasoning_effort != resolved.reasoning_effort
    {
        return ModelRouteDecision::Blocked(ModelRouteReason::ActualSettingMismatch);
    }
    ModelRouteDecision::Selected(resolved.clone())
}

fn select_candidate(
    request: &ModelRouteRequest,
    candidate: &ModelCapability,
    profile: ModelGovernanceProfile,
    effort: ModelReasoningEffort,
    pinned: bool,
) -> ModelRouteDecision {
    if !request
        .allowed_provider_surfaces
        .contains(&candidate.provider_surface)
    {
        return ModelRouteDecision::Blocked(ModelRouteReason::ProviderScopeDenied);
    }
    if !candidate.evidence_is_admissible() {
        return ModelRouteDecision::Blocked(ModelRouteReason::CapabilityEvidenceMissing);
    }
    if !candidate.available || candidate.provider_role == ModelProviderRole::Retired {
        return ModelRouteDecision::Blocked(ModelRouteReason::ProviderUnavailable);
    }
    if !candidate.supports(profile, effort) {
        return ModelRouteDecision::Blocked(ModelRouteReason::QualityFloorUnmet);
    }
    if !budget_fits(request, candidate) {
        return ModelRouteDecision::Blocked(ModelRouteReason::BudgetExhausted);
    }

    let mut reason_codes = vec![
        ModelRouteReason::RiskFloor,
        ModelRouteReason::CapabilityMatch,
    ];
    if pinned {
        reason_codes.insert(0, ModelRouteReason::ReproducibilityPin);
    } else if candidate.provider_role == ModelProviderRole::Primary {
        reason_codes.push(ModelRouteReason::PrimaryHealthy);
        reason_codes.push(ModelRouteReason::CostLatencyTiebreak);
    } else {
        reason_codes.push(ModelRouteReason::StandbyCompatible);
    }
    ModelRouteDecision::Selected(ResolvedModelRoute {
        provider_id: candidate.provider_id.clone(),
        provider_surface: candidate.provider_surface.clone(),
        model_id: candidate.model_id.clone(),
        model_version: candidate.model_version.clone(),
        governance_profile: profile,
        reasoning_effort: effort,
        primary_or_standby: candidate.provider_role,
        reason_codes,
    })
}

const fn effort_for_profile(profile: ModelGovernanceProfile) -> ModelReasoningEffort {
    match profile {
        ModelGovernanceProfile::Watch | ModelGovernanceProfile::Quick => ModelReasoningEffort::Low,
        ModelGovernanceProfile::Standard => ModelReasoningEffort::Medium,
        ModelGovernanceProfile::Deep => ModelReasoningEffort::High,
        ModelGovernanceProfile::Critical => ModelReasoningEffort::XHigh,
    }
}

const fn route_cost_key(candidate: &ModelCapability) -> (u64, u64) {
    (candidate.cost_units_per_call, candidate.latency_units)
}

const fn budget_fits(request: &ModelRouteRequest, candidate: &ModelCapability) -> bool {
    if request.max_model_calls == 0
        || candidate.max_calls == 0
        || request.max_model_calls > candidate.max_calls
    {
        return false;
    }
    let Some(total_cost) = candidate
        .cost_units_per_call
        .checked_mul(request.max_model_calls)
    else {
        return false;
    };
    total_cost <= request.cost_ceiling_units
}
