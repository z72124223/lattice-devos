use lattice_orchestrator::{
    AutonomyCandidate, AutonomyCycleBlocker, AutonomyCycleDecision, AutonomyCycleInput,
    CandidateAuthorization, CandidateEvidence, CandidateSafety, CandidateVerification, ModelRoute,
    ModelShortfall, select_autonomy_cycle,
};

fn candidate(id: &str, value: u8) -> AutonomyCandidate {
    AutonomyCandidate::new(
        id,
        value,
        CandidateSafety::Reversible,
        CandidateAuthorization::Authorized,
        CandidateVerification::Planned,
        CandidateEvidence::Current,
    )
}

#[test]
fn foreground_codex_work_always_yields_before_selection() {
    let decision = select_autonomy_cycle(&AutonomyCycleInput::new(
        true,
        ModelShortfall::None,
        vec![
            candidate("a", 9),
            candidate("b", 8),
            candidate("c", 7),
            candidate("d", 6),
            candidate("e", 5),
        ],
    ));

    assert_eq!(decision, AutonomyCycleDecision::YieldToForeground);
}

#[test]
fn verified_five_way_round_selects_unique_best_candidate_with_luna_low() {
    let decision = select_autonomy_cycle(&AutonomyCycleInput::new(
        false,
        ModelShortfall::None,
        vec![
            candidate("a", 6),
            candidate("b", 9),
            candidate("c", 7),
            candidate("d", 5),
            candidate("e", 4),
        ],
    ));

    assert_eq!(
        decision,
        AutonomyCycleDecision::Select {
            candidate_id: "b".to_owned(),
            route: ModelRoute::LunaLow,
            challenges: vec![
                "a".to_owned(),
                "b".to_owned(),
                "c".to_owned(),
                "d".to_owned(),
                "e".to_owned()
            ],
        }
    );
}

#[test]
fn duplicate_candidates_cannot_impersonate_five_independent_challenges() {
    let decision = select_autonomy_cycle(&AutonomyCycleInput::new(
        false,
        ModelShortfall::None,
        vec![
            candidate("a", 9),
            candidate("a", 8),
            candidate("c", 7),
            candidate("d", 6),
            candidate("e", 5),
        ],
    ));
    assert_eq!(
        decision,
        AutonomyCycleDecision::Blocked(AutonomyCycleBlocker::DuplicateCandidateId)
    );
}

#[test]
fn missing_current_evidence_blocks_instead_of_trusting_a_claim() {
    let decision = select_autonomy_cycle(&AutonomyCycleInput::new(
        false,
        ModelShortfall::None,
        vec![
            candidate("a", 9),
            AutonomyCandidate::new(
                "b",
                8,
                CandidateSafety::Reversible,
                CandidateAuthorization::Authorized,
                CandidateVerification::Planned,
                CandidateEvidence::Missing,
            ),
            candidate("c", 7),
            candidate("d", 6),
            candidate("e", 5),
        ],
    ));

    assert_eq!(
        decision,
        AutonomyCycleDecision::Blocked(AutonomyCycleBlocker::EvidenceMissing)
    );
}

#[test]
fn escalation_is_ordered_by_recorded_preceding_shortfall() {
    let terra = select_autonomy_cycle(&AutonomyCycleInput::new(
        false,
        ModelShortfall::LunaLowInsufficient,
        vec![
            candidate("a", 9),
            candidate("b", 8),
            candidate("c", 7),
            candidate("d", 6),
            candidate("e", 5),
        ],
    ));
    let sol = select_autonomy_cycle(&AutonomyCycleInput::new(
        false,
        ModelShortfall::TerraLowInsufficient,
        vec![
            candidate("a", 9),
            candidate("b", 8),
            candidate("c", 7),
            candidate("d", 6),
            candidate("e", 5),
        ],
    ));

    assert!(matches!(
        terra,
        AutonomyCycleDecision::Select {
            route: ModelRoute::TerraLow,
            ..
        }
    ));
    assert!(matches!(
        sol,
        AutonomyCycleDecision::Select {
            route: ModelRoute::SolLow,
            ..
        }
    ));
}

#[test]
fn incomplete_challenge_set_or_protected_candidate_fails_closed() {
    let incomplete = select_autonomy_cycle(&AutonomyCycleInput::new(
        false,
        ModelShortfall::None,
        vec![
            candidate("a", 9),
            candidate("b", 8),
            candidate("c", 7),
            candidate("d", 6),
        ],
    ));
    let protected = select_autonomy_cycle(&AutonomyCycleInput::new(
        false,
        ModelShortfall::None,
        vec![
            AutonomyCandidate::new(
                "a",
                9,
                CandidateSafety::ProtectedAction,
                CandidateAuthorization::Authorized,
                CandidateVerification::Planned,
                CandidateEvidence::Current,
            ),
            candidate("b", 8),
            candidate("c", 7),
            candidate("d", 6),
            candidate("e", 5),
        ],
    ));

    assert_eq!(
        incomplete,
        AutonomyCycleDecision::Blocked(AutonomyCycleBlocker::ChallengeCountInvalid)
    );
    assert_eq!(
        protected,
        AutonomyCycleDecision::Blocked(AutonomyCycleBlocker::ProtectedAction)
    );
}
