//! Pure, evidence-bound autonomy-cycle recommendations.
//!
//! This module selects no process, provider, scheduler, or task authority. It
//! only classifies a complete current snapshot into a data-only next step.

/// One candidate considered during an autonomy cycle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutonomyCandidate {
    id: String,
    value: u8,
    safety: CandidateSafety,
    authorization: CandidateAuthorization,
    verification: CandidateVerification,
    evidence: CandidateEvidence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateSafety {
    Reversible,
    ProtectedAction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateAuthorization {
    Authorized,
    Missing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateVerification {
    Planned,
    Missing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateEvidence {
    Current,
    Missing,
}

impl AutonomyCandidate {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        value: u8,
        safety: CandidateSafety,
        authorization: CandidateAuthorization,
        verification: CandidateVerification,
        evidence: CandidateEvidence,
    ) -> Self {
        Self {
            id: id.into(),
            value,
            safety,
            authorization,
            verification,
            evidence,
        }
    }
}

/// Ordered observed insufficiency of the immediately preceding model route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelShortfall {
    None,
    LunaLowInsufficient,
    TerraLowInsufficient,
}

/// The only model routes this pure core can recommend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelRoute {
    LunaLow,
    TerraLow,
    SolLow,
}

/// Complete snapshot for one bounded autonomy round.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutonomyCycleInput {
    foreground_codex_task_active: bool,
    prior_shortfall: ModelShortfall,
    candidates: Vec<AutonomyCandidate>,
}

impl AutonomyCycleInput {
    #[must_use]
    pub fn new(
        foreground_codex_task_active: bool,
        prior_shortfall: ModelShortfall,
        candidates: Vec<AutonomyCandidate>,
    ) -> Self {
        Self {
            foreground_codex_task_active,
            prior_shortfall,
            candidates,
        }
    }
}

/// Why a cycle cannot select a candidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyCycleBlocker {
    ChallengeCountInvalid,
    EvidenceMissing,
    CandidateNotReversible,
    CandidateNotAuthorized,
    VerificationPlanMissing,
    ProtectedAction,
    DuplicateCandidateId,
    NoUniqueBestCandidate,
}

/// Data-only next step for one autonomy round.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AutonomyCycleDecision {
    YieldToForeground,
    Blocked(AutonomyCycleBlocker),
    Select {
        candidate_id: String,
        route: ModelRoute,
        challenges: Vec<String>,
    },
}

/// Selects a bounded next step from a complete current snapshot.
#[must_use]
pub fn select_autonomy_cycle(input: &AutonomyCycleInput) -> AutonomyCycleDecision {
    if input.foreground_codex_task_active {
        return AutonomyCycleDecision::YieldToForeground;
    }
    if input.candidates.len() != 5 {
        return AutonomyCycleDecision::Blocked(AutonomyCycleBlocker::ChallengeCountInvalid);
    }
    let mut candidate_ids = std::collections::BTreeSet::new();
    for candidate in &input.candidates {
        if !candidate_ids.insert(candidate.id.as_str()) {
            return AutonomyCycleDecision::Blocked(AutonomyCycleBlocker::DuplicateCandidateId);
        }
        if candidate.evidence != CandidateEvidence::Current {
            return AutonomyCycleDecision::Blocked(AutonomyCycleBlocker::EvidenceMissing);
        }
        if candidate.safety != CandidateSafety::Reversible {
            return AutonomyCycleDecision::Blocked(AutonomyCycleBlocker::ProtectedAction);
        }
        if candidate.authorization != CandidateAuthorization::Authorized {
            return AutonomyCycleDecision::Blocked(AutonomyCycleBlocker::CandidateNotAuthorized);
        }
        if candidate.verification != CandidateVerification::Planned {
            return AutonomyCycleDecision::Blocked(AutonomyCycleBlocker::VerificationPlanMissing);
        }
    }
    let Some(best) = input
        .candidates
        .iter()
        .max_by_key(|candidate| candidate.value)
    else {
        return AutonomyCycleDecision::Blocked(AutonomyCycleBlocker::ChallengeCountInvalid);
    };
    if input
        .candidates
        .iter()
        .filter(|candidate| candidate.value == best.value)
        .count()
        != 1
    {
        return AutonomyCycleDecision::Blocked(AutonomyCycleBlocker::NoUniqueBestCandidate);
    }
    let route = match input.prior_shortfall {
        ModelShortfall::None => ModelRoute::LunaLow,
        ModelShortfall::LunaLowInsufficient => ModelRoute::TerraLow,
        ModelShortfall::TerraLowInsufficient => ModelRoute::SolLow,
    };
    AutonomyCycleDecision::Select {
        candidate_id: best.id.clone(),
        route,
        challenges: input
            .candidates
            .iter()
            .map(|candidate| candidate.id.clone())
            .collect(),
    }
}
