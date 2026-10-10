use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256, canonicalize};
use lattice_contracts::{
    ContentDigest, GitRefIdentity, ProjectClass, ProjectId, ProjectLifecycle, RuntimeKind,
};
use lattice_project_registry::{
    CommandId, ReconciliationDecision, RegistryCommand, RegistryCommandLookup,
    RegistryCommandOutcome, RegistryEpochBaseline, RegistryEpochPlan, RegistryError,
    RegistryRedactionAuthorization, RepositoryObservation, UntrustedRegistrySnapshot,
    VerifiedRegistryState, apply_command_plan, export_untrusted_registry_snapshot,
    lookup_registry_command, plan_command, plan_registry_epoch, preview_required_redactions,
    registry_command_id_commitment, verify_registry_epoch_baseline,
    verify_untrusted_registry_snapshot, verify_untrusted_registry_snapshot_from_baseline,
};

const A: &str = "epoch-target-a";
const B: &str = "epoch-survivor-b";
fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).unwrap()
}
fn project(id: &str) -> ProjectId {
    ProjectId::new(id).unwrap()
}
fn id(value: &str) -> CommandId {
    CommandId::new(value).unwrap()
}
fn observation(root: &str, values: [char; 4]) -> RepositoryObservation {
    RepositoryObservation::new(
        root,
        digest(values[0]),
        digest(values[1]),
        digest(values[2]),
        GitRefIdentity::new("refs/heads/main", digest(values[3])).unwrap(),
    )
    .unwrap()
}
fn alpha() -> RepositoryObservation {
    observation(r"C:\synthetic\epoch\alpha", ['1', '2', '3', '4'])
}
fn bravo() -> RepositoryObservation {
    observation(r"C:\synthetic\epoch\bravo", ['5', '6', '7', '8'])
}
fn register(command: &str, owner: &str, observation: RepositoryObservation) -> RegistryCommand {
    RegistryCommand::register(
        id(command),
        project(owner),
        ProjectClass::UserProject,
        observation,
    )
}
fn execute(base: &VerifiedRegistryState, command: RegistryCommand) -> VerifiedRegistryState {
    let plan = plan_command(base, command).unwrap();
    apply_command_plan(base, &plan).unwrap().state().clone()
}
fn fixture() -> VerifiedRegistryState {
    let initial = VerifiedRegistryState::vacant(RuntimeKind::Live).unwrap();
    let a = execute(&initial, register("register-alpha", A, alpha()));
    let b = execute(&a, register("register-bravo", B, bravo()));
    let head = b.project(&project(A)).unwrap().authority().head();
    execute(
        &b,
        RegistryCommand::observe(id("observe-alpha"), project(A), head, alpha()),
    )
}
fn restore(plan: &RegistryEpochPlan) -> VerifiedRegistryState {
    let value = plan.baseline().to_canonical_value();
    let retained = RegistryEpochBaseline::from_canonical_value(&value).unwrap();
    assert_eq!(
        canonicalize(&value).unwrap(),
        canonicalize(&retained.to_canonical_value()).unwrap()
    );
    verify_registry_epoch_baseline(&retained, plan.baseline().epoch(), plan.baseline_digest())
        .unwrap()
}
fn authorization(
    base: &VerifiedRegistryState,
    command: &RegistryCommand,
) -> RegistryRedactionAuthorization {
    let original = plan_command(base, command.clone()).unwrap();
    assert!(original.is_replay());
    RegistryRedactionAuthorization::new(
        registry_command_id_commitment(command.command_id()).unwrap(),
        original.record_set().record_set_digest().clone(),
    )
}
fn field_mut<'a>(value: &'a mut CanonicalValue, key: &str) -> &'a mut CanonicalValue {
    let CanonicalValue::Object(fields) = value else {
        panic!("object");
    };
    &mut fields.iter_mut().find(|(name, _)| name == key).unwrap().1
}

#[test]
fn interleaved_history_becomes_attested_baseline_without_rewriting_survivor() {
    let before = fixture();
    assert_eq!(
        verify_untrusted_registry_snapshot(&export_untrusted_registry_snapshot(&before)).unwrap(),
        before
    );
    let original_b = plan_command(&before, register("register-bravo", B, bravo())).unwrap();
    let plan = plan_registry_epoch(&before, &project(A), 1, &[]).unwrap();
    let baseline = restore(&plan);
    assert_eq!(baseline.project(&project(B)), before.project(&project(B)));
    assert!(baseline.project(&project(A)).is_none());
    assert_eq!(baseline.checkpoint().command_ordinal(), 0);
    assert_eq!(baseline.checkpoint().command_count(), 0);
    assert!(baseline.commands().is_empty());
    assert_eq!(baseline.epoch(), 1);
    assert_eq!(plan.baseline().normal_history().len(), 1);
    let old = plan
        .baseline()
        .normal_history()
        .get(&id("register-bravo"))
        .unwrap();
    assert_eq!(old.origin_epoch(), 0);
    assert_eq!(old.record().ordinal(), 2);
    assert_eq!(old.record_set(), original_b.record_set());
    assert_eq!(old.record().receipt(), original_b.receipt());
    assert_eq!(baseline.retained_command(&id("register-bravo")).unwrap(),Some(old.record()));
    assert_eq!(baseline.retained_command(&id("register-alpha")),Err(RegistryError::CommandRedacted));
    assert_eq!(before.history_assurance(),"FULL_GENESIS_REPLAY");
    assert_eq!(baseline.history_assurance(),"ATTESTED_BASELINE_AND_REPLAYED_TAIL");
    let encoded = String::from_utf8(
        canonicalize(&plan.baseline().to_canonical_value())
            .unwrap()
            .into_vec(),
    )
    .unwrap();
    assert!(!encoded.contains(A));
    assert!(!encoded.contains("alpha"));
    assert!(
        verify_untrusted_registry_snapshot(&export_untrusted_registry_snapshot(&baseline)).is_err(),
        "attested baseline must not masquerade as complete v1 replay"
    );
    assert_eq!(
        verify_untrusted_registry_snapshot_from_baseline(
            &baseline,
            &export_untrusted_registry_snapshot(&baseline)
        )
        .unwrap(),
        baseline
    );
}

#[test]
fn original_retry_and_changed_request_work_after_persisted_baseline() {
    let before = fixture();
    let plan = plan_registry_epoch(&before, &project(A), 1, &[]).unwrap();
    let baseline = restore(&plan);
    let command = register("register-bravo", B, bravo());
    let lookup = lookup_registry_command(
        &baseline,
        command.command_id(),
        &command.request_digest().unwrap(),
    )
    .unwrap();
    assert!(matches!(lookup, RegistryCommandLookup::ExactHistorical(_)));
    let replay = plan_command(&baseline, command).unwrap();
    assert!(replay.is_replay());
    assert_eq!(replay.record_set().ordinal(), 2);
    assert_eq!(
        apply_command_plan(&baseline, &replay).unwrap().state(),
        &baseline
    );
    assert_eq!(
        plan_command(
            &baseline,
            register("register-bravo", "another-project", bravo())
        ),
        Err(RegistryError::CommandIdReuse)
    );
    for command in [
        register("register-alpha", A, alpha()),
        register("register-alpha", "different-owner", bravo()),
    ] {
        assert_eq!(
            plan_command(&baseline, command),
            Err(RegistryError::CommandRedacted)
        );
    }
}

#[test]
fn fresh_id_can_register_released_identity_and_epoch_tail_starts_at_one() {
    let plan = plan_registry_epoch(&fixture(), &project(A), 1, &[]).unwrap();
    let baseline = restore(&plan);
    assert!(
        baseline
            .reservations()
            .iter()
            .all(|reservation| reservation.project_id() != &project(A))
    );
    let command = register("register-alpha-new", A, alpha());
    let first = plan_command(&baseline, command.clone()).unwrap();
    assert!(!first.is_replay());
    assert_eq!(first.record_set().ordinal(), 1);
    assert_eq!(first.receipt().outcome(), RegistryCommandOutcome::Applied);
    let after = apply_command_plan(&baseline, &first)
        .unwrap()
        .state()
        .clone();
    assert_eq!(after.checkpoint().command_count(), 1);
    assert_eq!(after.checkpoint().command_ordinal(), 1);
    assert!(matches!(
        lookup_registry_command(
            &after,
            command.command_id(),
            &command.request_digest().unwrap()
        )
        .unwrap(),
        RegistryCommandLookup::ExactTail(_)
    ));
    assert_eq!(
        verify_untrusted_registry_snapshot_from_baseline(
            &baseline,
            &export_untrusted_registry_snapshot(&after)
        )
        .unwrap(),
        after
    );
    let exported = export_untrusted_registry_snapshot(&after);
    let missing = UntrustedRegistrySnapshot::from_retained(
        exported.claimed_checkpoint().clone(),
        exported.observations().to_vec(),
        exported.projects().to_vec(),
        vec![],
        exported.reservations().to_vec(),
    );
    assert_eq!(
        verify_untrusted_registry_snapshot_from_baseline(&baseline, &missing),
        Err(RegistryError::CorruptSnapshot)
    );
    assert_eq!(
        verify_untrusted_registry_snapshot_from_baseline(&after, &exported),
        Err(RegistryError::EpochBaselineInvalid)
    );
}

#[test]
fn seals_and_epochs_are_independent_inputs_and_tampering_never_self_authorizes() {
    let plan = plan_registry_epoch(&fixture(), &project(A), 1, &[]).unwrap();
    assert_eq!(
        verify_registry_epoch_baseline(plan.baseline(), 0, plan.baseline_digest()),
        Err(RegistryError::EpochTrustMismatch)
    );
    assert_eq!(
        verify_registry_epoch_baseline(plan.baseline(), 2, plan.baseline_digest()),
        Err(RegistryError::EpochTrustMismatch)
    );
    assert_eq!(
        verify_registry_epoch_baseline(plan.baseline(), 1, &digest('f')),
        Err(RegistryError::EpochTrustMismatch)
    );
    let mut value = plan.baseline().to_canonical_value();
    *field_mut(&mut value, "epoch") = CanonicalValue::String("2".to_owned());
    let changed = RegistryEpochBaseline::from_canonical_value(&value).unwrap();
    assert_eq!(
        verify_registry_epoch_baseline(&changed, 2, plan.baseline_digest()),
        Err(RegistryError::EpochTrustMismatch)
    );
    let changed_digest = ContentDigest::from_sha256(
        canonical_sha256(
            &HashDomain::new("lattice.project-registry.epoch-baseline", "1").unwrap(),
            &value,
        )
        .unwrap()
        .to_hex(),
    )
    .unwrap();
    let first = restore(&plan);
    let second = verify_registry_epoch_baseline(&changed, 2, &changed_digest).unwrap();
    assert_ne!(
        first.checkpoint().checkpoint_digest(),
        second.checkpoint().checkpoint_digest()
    );
    let command = register("register-alpha-new", A, alpha());
    assert_ne!(
        plan_command(&first, command.clone())
            .unwrap()
            .result_checkpoint(),
        plan_command(&second, command).unwrap().result_checkpoint()
    );
}

#[test]
fn codec_rejects_unknown_duplicate_missing_noncanonical_and_corrupt_fields() {
    let plan = plan_registry_epoch(&fixture(), &project(A), 1, &[]).unwrap();
    for mode in [
        "unknown",
        "duplicate",
        "missing",
        "integer",
        "bad-epoch",
        "bad-history",
    ] {
        let mut value = plan.baseline().to_canonical_value();
        let CanonicalValue::Object(fields) = &mut value else {
            panic!("object");
        };
        match mode {
            "unknown" => fields.push((
                "ignored_payload".to_owned(),
                CanonicalValue::String(A.to_owned()),
            )),
            "duplicate" => fields.push(fields[0].clone()),
            "missing" => {
                fields.pop();
            }
            "integer" => *field_mut(&mut value, "epoch") = CanonicalValue::Bool(true),
            "bad-epoch" => {
                *field_mut(&mut value, "epoch") = CanonicalValue::String("01".to_owned());
            }
            "bad-history" => {
                let CanonicalValue::Array(history) = field_mut(&mut value, "history") else {
                    panic!("array");
                };
                history.push(history[0].clone());
            }
            _ => unreachable!(),
        }
        assert!(
            RegistryEpochBaseline::from_canonical_value(&value).is_err(),
            "{mode}"
        );
    }
}

#[test]
fn second_purge_carries_prior_tombstones_and_preserves_new_epoch_history() {
    let first = plan_registry_epoch(&fixture(), &project(A), 1, &[]).unwrap();
    let baseline = restore(&first);
    let new_a = register("register-alpha-new", A, alpha());
    let after = execute(&baseline, new_a.clone());
    let second = plan_registry_epoch(&after, &project(B), 2, &[]).unwrap();
    let restored = restore(&second);
    assert_eq!(restored.epoch(), 2);
    assert_eq!(restored.checkpoint().command_count(), 0);
    assert!(restored.project(&project(B)).is_none());
    assert_eq!(restored.project(&project(A)), after.project(&project(A)));
    assert_eq!(
        second
            .baseline()
            .normal_history()
            .get(new_a.command_id())
            .unwrap()
            .origin_epoch(),
        1
    );
    assert!(plan_command(&restored, new_a).unwrap().is_replay());
    assert_eq!(
        plan_command(&restored, register("register-alpha", A, alpha())),
        Err(RegistryError::CommandRedacted)
    );
    assert_eq!(
        plan_command(&restored, register("register-bravo", B, bravo())),
        Err(RegistryError::CommandRedacted)
    );
    let third = plan_registry_epoch(&restored, &project(A), 3, &[]).unwrap();
    assert!(third.baseline().normal_history().is_empty());
    assert_eq!(third.baseline().redacted_command_ids().len(), 4);
    assert!(restore(&third).project(&project(A)).is_none());
}

#[test]
fn cross_project_denial_needs_exact_whole_record_redaction_permission() {
    let denied = register("duplicate-alpha", "epoch-reference-c", alpha());
    let before = execute(&fixture(), denied.clone());
    assert_eq!(
        plan_registry_epoch(&before, &project(A), 1, &[]),
        Err(RegistryError::EpochRedactionRequired)
    );
    let permission = authorization(&before, &denied);
    let preview = preview_required_redactions(&before, &project(A)).unwrap();
    assert_eq!(preview, vec![permission.clone()]);
    assert_eq!(
        preview[0].command_id_digest(),
        &registry_command_id_commitment(denied.command_id()).unwrap()
    );
    assert_eq!(
        preview[0].record_set_digest(),
        plan_command(&before, denied.clone())
            .unwrap()
            .record_set()
            .record_set_digest()
    );
    let plan =
        plan_registry_epoch(&before, &project(A), 1, std::slice::from_ref(&permission)).unwrap();
    let baseline = restore(&plan);
    assert_eq!(
        plan_command(&baseline, denied.clone()),
        Err(RegistryError::CommandRedacted)
    );
    let stale = RegistryRedactionAuthorization::new(
        registry_command_id_commitment(denied.command_id()).unwrap(),
        digest('f'),
    );
    assert_eq!(
        plan_registry_epoch(&before, &project(A), 1, &[stale]),
        Err(RegistryError::EpochRedactionInvalid)
    );
    assert_eq!(
        plan_registry_epoch(&before, &project(A), 1, &[permission.clone(), permission]),
        Err(RegistryError::EpochRedactionInvalid)
    );
    let unrelated = authorization(&before, &register("register-bravo", B, bravo()));
    assert_eq!(
        plan_registry_epoch(&fixture(), &project(A), 1, &[unrelated]),
        Err(RegistryError::EpochRedactionInvalid)
    );
}

#[test]
fn survivor_current_projection_dependency_cannot_be_authorized_by_command_redaction() {
    let base = fixture();
    let b = project(B);
    let conflict = RegistryCommand::observe(
        id("conflicting-observation"),
        b.clone(),
        base.project(&b).unwrap().authority().head(),
        alpha(),
    );
    let before = execute(&base, conflict.clone());
    assert_eq!(
        before.project(&b).unwrap().authority().lifecycle(),
        ProjectLifecycle::Suspended
    );
    let permission = authorization(&before, &conflict);
    assert_eq!(
        preview_required_redactions(&before, &project(A)),
        Err(RegistryError::EpochSurvivorReference)
    );
    assert_eq!(
        plan_registry_epoch(&before, &project(A), 1, &[permission]),
        Err(RegistryError::EpochSurvivorReference)
    );
}

#[test]
fn redaction_preview_omits_target_owned_and_unrelated_survivor_commands() {
    let before = fixture();
    let unchanged = before.clone();
    assert_eq!(
        preview_required_redactions(&before, &project(A)).unwrap(),
        vec![]
    );
    assert_eq!(before, unchanged);
    let plan = plan_registry_epoch(&before, &project(A), 1, &[]).unwrap();
    assert_eq!(plan.baseline().redacted_command_ids().len(), 2);
    assert_eq!(plan.baseline().normal_history().len(), 1);
}

#[test]
fn redaction_preview_classifies_attested_survivor_history_and_current_tail_together() {
    let base = fixture();
    let b = project(B);
    let historical_reference = RegistryCommand::observe(
        id("historical-bravo-mentions-epoch-target-a"),
        b.clone(),
        base.project(&b).unwrap().authority().head(),
        bravo(),
    );
    let referenced = execute(&base, historical_reference.clone());
    let third = project("epoch-unrelated-c");
    let before = execute(
        &referenced,
        register(
            "register-charlie",
            third.as_str(),
            observation(r"C:\synthetic\epoch\charlie", ['9', 'a', 'b', 'c']),
        ),
    );
    let baseline = restore(&plan_registry_epoch(&before, &third, 1, &[]).unwrap());
    let tail_reference = RegistryCommand::observe(
        id("tail-bravo-mentions-epoch-target-a"),
        b.clone(),
        baseline.project(&b).unwrap().authority().head(),
        bravo(),
    );
    let referenced = execute(&baseline, tail_reference.clone());
    let ordinary_tail = RegistryCommand::observe(
        id("tail-ordinary-bravo"),
        b.clone(),
        referenced.project(&b).unwrap().authority().head(),
        bravo(),
    );
    let before = execute(&referenced, ordinary_tail.clone());
    let preview = preview_required_redactions(&before, &project(A)).unwrap();
    assert_eq!(
        preview,
        vec![
            authorization(&before, &historical_reference),
            authorization(&before, &tail_reference),
        ]
    );
    assert_eq!(
        plan_registry_epoch(&before, &project(A), 2, &[]),
        Err(RegistryError::EpochRedactionRequired)
    );
    let planned = plan_registry_epoch(&before, &project(A), 2, &preview).unwrap();
    assert_eq!(planned.baseline().normal_history().len(), 2);
    assert!(
        planned
            .baseline()
            .normal_history()
            .contains_key(ordinary_tail.command_id())
    );
    assert!(
        planned
            .baseline()
            .normal_history()
            .contains_key(&id("register-bravo"))
    );
    assert_eq!(restore(&planned).project(&b), before.project(&b));
}

#[test]
fn command_text_references_are_not_ignored_even_without_typed_foreign_project_id() {
    let base = execute(
        &VerifiedRegistryState::vacant(RuntimeKind::Live).unwrap(),
        register("register-alpha", A, alpha()),
    );
    let command = register("register-bravo-mentions-epoch-target-a", B, bravo());
    let before = execute(&base, command.clone());
    assert_eq!(
        plan_registry_epoch(&before, &project(A), 1, &[]),
        Err(RegistryError::EpochRedactionRequired)
    );
    let plan =
        plan_registry_epoch(&before, &project(A), 1, &[authorization(&before, &command)]).unwrap();
    assert_eq!(
        restore(&plan).project(&project(B)),
        before.project(&project(B))
    );
}

#[test]
fn suspend_reconcile_and_observe_history_roundtrips_without_receipt_changes() {
    let initial = fixture();
    let b = project(B);
    let suspended = execute(
        &initial,
        RegistryCommand::suspend(
            id("suspend-bravo"),
            b.clone(),
            initial.project(&b).unwrap().authority().head(),
            digest('e'),
        ),
    );
    let restored = execute(
        &suspended,
        RegistryCommand::reconcile(
            id("reactivate-bravo"),
            b.clone(),
            suspended.project(&b).unwrap().authority().head(),
            bravo(),
            ReconciliationDecision::Reactivate,
            digest('f'),
        ),
    );
    let moved = observation(r"C:\synthetic\epoch\bravo-moved", ['5', '6', '7', '8']);
    let observed = execute(
        &restored,
        RegistryCommand::observe(
            id("move-bravo"),
            b.clone(),
            restored.project(&b).unwrap().authority().head(),
            moved.clone(),
        ),
    );
    let settled = execute(
        &observed,
        RegistryCommand::reconcile(
            id("accept-bravo-move"),
            b.clone(),
            observed.project(&b).unwrap().authority().head(),
            moved,
            ReconciliationDecision::AcceptMove,
            digest('a'),
        ),
    );
    let plan = plan_registry_epoch(&settled, &project(A), 1, &[]).unwrap();
    let baseline = restore(&plan);
    for record in settled
        .commands()
        .values()
        .filter(|record| record.command().command_id().as_str().contains("bravo"))
    {
        let replay = plan_command(&baseline, record.command().clone()).unwrap();
        assert!(replay.is_replay());
        assert_eq!(replay.receipt(), record.receipt());
        assert_eq!(
            replay.record_set().record_set_digest(),
            record.record_set_digest()
        );
    }
    assert_eq!(baseline.project(&b), settled.project(&b));
}

#[test]
fn successor_and_target_checks_are_closed() {
    assert_eq!(
        plan_registry_epoch(&fixture(), &project(A), 2, &[]),
        Err(RegistryError::EpochSequenceInvalid)
    );
    assert_eq!(
        plan_registry_epoch(&fixture(), &project("unknown"), 1, &[]),
        Err(RegistryError::EpochProjectNotFound)
    );
    assert_eq!(
        preview_required_redactions(&fixture(), &project("unknown")),
        Err(RegistryError::EpochProjectNotFound)
    );
}

#[test]
fn command_id_commitments_match_exact_cjson_string_domain() {
    for command in ["register-alpha", "命令-台灣", "quote\"slash\\line\nend"] {
        let domain =
            HashDomain::new("lattice.project-registry.command-id-commitment", "1").unwrap();
        let expected = canonical_sha256(&domain, &CanonicalValue::String(command.to_owned()))
            .unwrap()
            .to_hex();
        assert_eq!(
            registry_command_id_commitment(&id(command))
                .unwrap()
                .as_str(),
            expected
        );
        println!("commitment-vector {command:?} {expected}");
    }
}
