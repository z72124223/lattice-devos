//! Operator-only branch identity recovery. No paths or authority supplied by requests.
use super::*;
use lattice_project_registry::{IdentityDrift, ReconciliationDecision, VerifiedRegistryState};
use serde_json::json;

/// Exact reviewed request. No automatic expected-head refresh is permitted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryRequest {
    pub project_id: ProjectId,
    pub revision: u64,
    pub receipt_digest: ContentDigest,
    pub pending_digest: ContentDigest,
}

impl RecoveryRequest {
    fn command_id(&self) -> ProjectBridgeResult<CommandId> {
        let digest = bridge_digest(
            "lattice.project-bridge.branch-recovery",
            &CanonicalValue::Object(vec![
                (
                    "project_id".into(),
                    CanonicalValue::String(self.project_id.as_str().into()),
                ),
                (
                    "revision".into(),
                    CanonicalValue::String(self.revision.to_string()),
                ),
                (
                    "receipt_digest".into(),
                    CanonicalValue::String(self.receipt_digest.as_str().into()),
                ),
                (
                    "pending_digest".into(),
                    CanonicalValue::String(self.pending_digest.as_str().into()),
                ),
            ]),
        )?;
        CommandId::new(format!("branch-recovery-{}", digest.as_str()))
            .map_err(|_| bridge_error(ProjectBridgeErrorKind::InvalidSelector))
    }
}

fn branch_only(current: &RegistryProjectProjection) -> bool {
    current.project_class() == ProjectClass::UserProject
        && current.authority().lifecycle() == ProjectLifecycle::ReconciliationRequired
        && !current.drift().is_empty()
        && current.drift().iter().all(|d| {
            matches!(
                d,
                IdentityDrift::PrimaryRefName | IdentityDrift::PrimaryRefStorage
            )
        })
}

fn prepare(
    state: &VerifiedRegistryState,
    request: &RecoveryRequest,
    fresh: &RepositoryObservation,
) -> ProjectBridgeResult<RegistryCommand> {
    let current = state
        .project(&request.project_id)
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectNotFound))?;
    if !branch_only(current) {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectInactive));
    }
    let head = current.authority().head();
    if head.registry_revision() != request.revision
        || head.receipt_digest() != &request.receipt_digest
    {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectRegistryConflict,
        ));
    }
    let pending = current
        .pending_observation()
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected))?;
    if pending.digest() != &request.pending_digest || pending != fresh {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    Ok(RegistryCommand::reconcile(
        request.command_id()?,
        request.project_id.clone(),
        head,
        pending.clone(),
        ReconciliationDecision::AcceptIdentityChange,
        request.pending_digest.clone(),
    ))
}

// Replay returns the retained result, never interprets it as current ACTIVE authority.
fn replay<'a>(
    state: &'a VerifiedRegistryState,
    request: &RecoveryRequest,
) -> ProjectBridgeResult<Option<&'a lattice_project_registry::RegistryCommandReceipt>> {
    let id = request.command_id()?;
    let retained = state
        .commands()
        .values()
        .find(|record| record.command().command_id() == &id);
    let Some(record) = retained else {
        return Ok(None);
    };
    match record.command() {
        RegistryCommand::Reconcile {
            project_id,
            expected_head,
            observation,
            decision,
            evidence_digest,
            ..
        } if project_id == &request.project_id
            && expected_head.project_id() == &request.project_id
            && expected_head.registry_revision() == request.revision
            && expected_head.receipt_digest() == &request.receipt_digest
            && observation.digest() == &request.pending_digest
            && *decision == ReconciliationDecision::AcceptIdentityChange
            && evidence_digest == &request.pending_digest =>
        {
            Ok(Some(record.receipt()))
        }
        _ => Err(bridge_error(
            ProjectBridgeErrorKind::ProjectRegistryRejected,
        )),
    }
}

fn projection(current: &RegistryProjectProjection, fresh: &RepositoryObservation) -> Value {
    let head = current.authority().head();
    json!({
        "project_id": head.project_id().as_str(),
        "registry_revision": head.registry_revision(),
        "receipt_digest": head.receipt_digest().as_str(),
        "lifecycle": head.lifecycle().as_str(),
        "active": head.lifecycle() == ProjectLifecycle::Active,
        "accepted_observation_digest": current.observation().digest().as_str(),
        "accepted_ref": current.observation().primary_branch().reference(),
        "pending_observation_digest": current.pending_observation().map(|p| p.digest().as_str()),
        "pending_ref": current.pending_observation().map(|p| p.primary_branch().reference()),
        "fresh_observation_digest": fresh.digest().as_str(),
        "fresh_matches_accepted": fresh == current.observation(),
        "fresh_matches_pending": current.pending_observation() == Some(fresh),
        "branch_only_recovery_eligible": branch_only(current) && current.pending_observation() == Some(fresh),
        "drift": current.drift().iter().map(|d| d.as_str()).collect::<Vec<_>>()
    })
}

pub(crate) fn run(
    database: &DeliveryDatabaseBinding,
    password: &str,
    project_id: &ProjectId,
    request: Option<&RecoveryRequest>,
) -> ProjectBridgeResult<Value> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let origin = match env::var(CONTROL_ORIGIN_ENV) {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => DEFAULT_CONTROL_ORIGIN.into(),
        Err(_) => return Err(bridge_error(ProjectBridgeErrorKind::ControlConfiguration)),
    };
    let origin = parse_control_origin(&origin)
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ControlConfiguration))?;
    let selector = ProjectSelector::new(Some(project_id.as_str()), None)?;
    let state = control_get_json(origin, "/api/state", deadline)?;
    let selected = select_catalog_project(&parse_catalog_state(&state)?, &selector)?.clone();
    let detail_path = format!("/api/projects/{}", selected.id);
    let detail_value = control_get_json(origin, &detail_path, deadline)?;
    let detail = parse_catalog_detail(&detail_value, Some(&selected))?;
    let fresh = inspect_repository(
        &detail.canonical_path,
        &configured_git_executable()?,
        deadline,
    )?;
    let again = control_get_json(origin, "/api/state", deadline)?;
    verify_catalog_replay(&state, &again, &selector, &selected)?;
    if control_get_json(origin, &detail_path, deadline)? != detail_value {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    let target = MigrationTarget::new(database.database_name(), database.run_id())
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected))?;
    let client = connect_fixed_runtime_client(database, password, deadline)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectRegistryUnavailable))?;
    let mut registry = PostgresProjectRegistry::new(client, &target).map_err(map_registry_error)?;
    let loaded = registry.load().map_err(map_registry_error)?;
    let current = loaded
        .state()
        .project(project_id)
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectNotFound))?;
    let Some(request) = request else {
        return Ok(
            json!({"schema_version":"lattice.project-registry.recovery.v1", "status":"INSPECTED",
                "registry_checkpoint_digest": loaded.state().checkpoint().checkpoint_digest().as_str(),
                "registry_command_count": loaded.state().commands().len(), "current":projection(current, &fresh)}),
        );
    };
    if &request.project_id != project_id {
        return Err(bridge_error(ProjectBridgeErrorKind::InvalidSelector));
    }
    if let Some(receipt) = replay(loaded.state(), request)? {
        return Ok(
            json!({"schema_version":"lattice.project-registry.recovery.v1", "status":"REPLAYED",
            "registry_checkpoint_digest": loaded.state().checkpoint().checkpoint_digest().as_str(),
            "registry_command_count": loaded.state().commands().len(),
            "command_id":request.command_id()?.as_str(), "historical_receipt_digest":receipt.result_digest().as_str(),
            "historical_outcome":format!("{:?}",receipt.outcome()), "current":projection(current, &fresh)}),
        );
    }
    let command = prepare(loaded.state(), request, &fresh)?;
    // DB loading can take time. Recheck the selected physical repository and
    // locator before committing; never replace the reviewed pending observation.
    if inspect_repository(
        &detail.canonical_path,
        &configured_git_executable()?,
        deadline,
    )? != fresh
        || control_get_json(origin, &detail_path, deadline)? != detail_value
    {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    let authority = crate::composition::configured_store_authority()
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected))?;
    let execution = registry
        .execute(command, authority)
        .map_err(map_registry_error)?;
    match execution.semantic_receipt().outcome() {
        RegistryCommandOutcome::Applied => {}
        RegistryCommandOutcome::Denied(d) => return Err(map_registry_denial(&d)),
        RegistryCommandOutcome::Blocked(_) => {
            return Err(bridge_error(
                ProjectBridgeErrorKind::ProjectIdentityCollision,
            ));
        }
    }
    let reloaded = registry.load().map_err(map_registry_error)?;
    let current = reloaded
        .state()
        .project(project_id)
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected))?;
    // Both receipt and current projection are reported: a concurrent later command
    // must not be hidden by the historical success of this exact request.
    Ok(
        json!({"schema_version":"lattice.project-registry.recovery.v1", "status":if execution.is_exact_retry(){"REPLAYED"}else{"APPLIED"},
        "registry_checkpoint_digest": reloaded.state().checkpoint().checkpoint_digest().as_str(),
        "registry_command_count": reloaded.state().commands().len(),
        "command_id":request.command_id()?.as_str(), "historical_receipt_digest":execution.semantic_receipt().result_digest().as_str(),
        "historical_outcome":"Applied", "current":projection(current, &fresh)}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_project_registry::FakeProjectRegistry;

    #[test]
    fn branch_recovery_replay_rejects_command_id_substitution() {
        let pending = observation("refs/heads/feature");
        let (mut registry, request) = fixture(pending.clone());
        registry
            .execute(RegistryCommand::reconcile(
                request.command_id().unwrap(),
                request.project_id.clone(),
                registry.current_head(&request.project_id).unwrap(),
                pending,
                ReconciliationDecision::AcceptIdentityChange,
                digest('a'),
            ))
            .unwrap();
        assert_eq!(
            replay(registry.verified_state(), &request)
                .unwrap_err()
                .kind(),
            ProjectBridgeErrorKind::ProjectRegistryRejected
        );
    }

    fn digest(c: char) -> ContentDigest {
        ContentDigest::from_sha256(c.to_string().repeat(64)).unwrap()
    }
    fn observation(branch: &str) -> RepositoryObservation {
        RepositoryObservation::new(
            r"C:\work\recovery",
            digest('1'),
            digest('2'),
            digest('3'),
            GitRefIdentity::new(branch, digest('4')).unwrap(),
        )
        .unwrap()
    }
    fn fixture(pending: RepositoryObservation) -> (FakeProjectRegistry, RecoveryRequest) {
        let mut registry = FakeProjectRegistry::new();
        let id = ProjectId::new("recovery-project").unwrap();
        registry
            .execute(RegistryCommand::register(
                CommandId::new("register").unwrap(),
                id.clone(),
                ProjectClass::UserProject,
                observation("refs/heads/main"),
            ))
            .unwrap();
        registry
            .execute(RegistryCommand::observe(
                CommandId::new("observe").unwrap(),
                id.clone(),
                registry.current_head(&id).unwrap(),
                pending.clone(),
            ))
            .unwrap();
        let head = registry.current_head(&id).unwrap();
        (
            registry,
            RecoveryRequest {
                project_id: id,
                revision: head.registry_revision(),
                receipt_digest: head.receipt_digest().clone(),
                pending_digest: pending.digest().clone(),
            },
        )
    }
    #[test]
    fn branch_recovery_applies_once_and_replays_original_receipt_after_later_change() {
        let pending = observation("refs/heads/feature");
        let (mut registry, request) = fixture(pending.clone());
        let command = prepare(registry.verified_state(), &request, &pending).unwrap();
        let receipt = registry.execute(command.clone()).unwrap();
        assert_eq!(receipt.outcome(), RegistryCommandOutcome::Applied);
        let checkpoint = registry.checkpoint().clone();
        assert_eq!(registry.execute(command).unwrap(), receipt);
        assert_eq!(registry.checkpoint(), &checkpoint);
        assert_eq!(
            replay(registry.verified_state(), &request).unwrap(),
            Some(&receipt)
        );
        registry
            .execute(RegistryCommand::observe(
                CommandId::new("later").unwrap(),
                request.project_id.clone(),
                registry.current_head(&request.project_id).unwrap(),
                observation("refs/heads/later"),
            ))
            .unwrap();
        assert_eq!(
            replay(registry.verified_state(), &request).unwrap(),
            Some(&receipt)
        );
        let current = registry
            .verified_state()
            .project(&request.project_id)
            .unwrap();
        assert_eq!(projection(current, &pending)["active"], false);
        assert_eq!(
            projection(current, &pending)["fresh_matches_pending"],
            false
        );
    }
    #[test]
    fn branch_recovery_rejects_wrong_project_stale_head_and_substituted_pending() {
        let pending = observation("refs/heads/feature");
        let (registry, request) = fixture(pending.clone());
        let mut bad = request.clone();
        bad.project_id = ProjectId::new("wrong-project").unwrap();
        assert!(prepare(registry.verified_state(), &bad, &pending).is_err());
        assert!(replay(registry.verified_state(), &bad).unwrap().is_none());
        let mut bad = request.clone();
        bad.revision -= 1;
        assert_eq!(
            prepare(registry.verified_state(), &bad, &pending)
                .unwrap_err()
                .kind(),
            ProjectBridgeErrorKind::ProjectRegistryConflict
        );
        let mut bad = request.clone();
        bad.receipt_digest = digest('a');
        assert!(prepare(registry.verified_state(), &bad, &pending).is_err());
        let mut bad = request.clone();
        bad.pending_digest = digest('b');
        assert!(prepare(registry.verified_state(), &bad, &pending).is_err());
        assert!(
            prepare(
                registry.verified_state(),
                &request,
                &observation("refs/heads/new-change")
            )
            .is_err()
        );
    }
    #[test]
    fn branch_recovery_rejects_moved_root_repository_and_worktree_changes() {
        for dimension in 0..4 {
            let pending = RepositoryObservation::new(
                if dimension == 0 {
                    r"C:\moved"
                } else {
                    r"C:\work\recovery"
                },
                digest(if dimension == 1 { 'a' } else { '1' }),
                digest(if dimension == 2 { 'b' } else { '2' }),
                digest(if dimension == 3 { 'c' } else { '3' }),
                GitRefIdentity::new("refs/heads/feature", digest('4')).unwrap(),
            )
            .unwrap();
            let (registry, request) = fixture(pending.clone());
            assert_eq!(
                prepare(registry.verified_state(), &request, &pending)
                    .unwrap_err()
                    .kind(),
                ProjectBridgeErrorKind::ProjectInactive
            );
        }
    }
    #[test]
    fn branch_recovery_rejects_suspension_and_never_refreshes_stale_request() {
        let pending = observation("refs/heads/feature");
        let (mut registry, request) = fixture(pending.clone());
        let prepared = prepare(registry.verified_state(), &request, &pending).unwrap();
        // Another reconciliation wins before execution: the domain rejects
        // the exact stale head even though our earlier preflight passed.
        let raced = registry
            .execute(RegistryCommand::reconcile(
                CommandId::new("race").unwrap(),
                request.project_id.clone(),
                registry.current_head(&request.project_id).unwrap(),
                pending.clone(),
                ReconciliationDecision::AcceptIdentityChange,
                digest('a'),
            ))
            .unwrap();
        assert_eq!(raced.outcome(), RegistryCommandOutcome::Applied);
        assert!(matches!(
            registry.execute(prepared).unwrap().outcome(),
            RegistryCommandOutcome::Denied(RegistryDenial::StaleHead)
        ));
        let (mut registry, request) = fixture(pending.clone());
        registry
            .execute(prepare(registry.verified_state(), &request, &pending).unwrap())
            .unwrap();
        registry
            .execute(RegistryCommand::suspend(
                CommandId::new("suspend").unwrap(),
                request.project_id.clone(),
                registry.current_head(&request.project_id).unwrap(),
                digest('a'),
            ))
            .unwrap();
        let head = registry.current_head(&request.project_id).unwrap();
        let request = RecoveryRequest {
            revision: head.registry_revision(),
            receipt_digest: head.receipt_digest().clone(),
            ..request
        };
        assert_eq!(
            prepare(registry.verified_state(), &request, &pending)
                .unwrap_err()
                .kind(),
            ProjectBridgeErrorKind::ProjectInactive
        );
    }
}
