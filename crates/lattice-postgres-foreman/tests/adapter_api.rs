use lattice_contracts::{ContentDigest, ProjectId, ProjectSnapshotId};
use lattice_postgres_foreman::{
    ActiveTaskRef, AdapterError, AdapterErrorKind, AppendDisposition, AttemptClosure,
    ClaimDisposition, ClaimOutcome, ClaimReservationDisposition, CredentialAuthorityKind,
    ExecutionAuthoritySource, ExecutionEnvironmentDescriptor, ExecutionEnvironmentKind,
    MAX_ARTIFACT_BYTES_PER_ATTEMPT, MAX_ARTIFACT_BYTES_PER_TASK, MAX_ARTIFACTS_PER_ATTEMPT,
    MAX_ARTIFACTS_PER_TASK, ManagedPreparationObservation, ManagedPreparationObservationKind,
    ManagedPromotionIntent, ManagedPromotionSource, ModelReason,
    NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF, PendingWorkerAttempt, PersistedExecutionEnvironment,
    PersistedReferenceLinks, PersistedTaskRuntimeRows, PostgresForeman, ProviderDispatchClaim,
    ProviderDispatchKind, ReasoningEffort, ReplayRecordState, RestartTaskCursor, RestartTaskKind,
    RestartTaskRef, StagedArtifactReference, VerifiedExecutionAuthority, VerifiedManagedEvidence,
    VerifiedTaskExecutionBinding, VerifiedWorkerAttemptRecord, WorkerBudget, WorkerModel,
    WorkerObservationKind,
};
use lattice_task_ledger::CorrelationId;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

#[test]
fn model_reasoning_and_terminal_values_are_closed() {
    assert_eq!(WorkerModel::Luna.as_str(), "gpt-5.6-luna");
    assert_eq!(WorkerModel::Terra.as_str(), "gpt-5.6-terra");
    assert_eq!(WorkerModel::Sol.as_str(), "gpt-5.6-sol");
    assert_eq!(ReasoningEffort::Medium.as_str(), "medium");
    assert_eq!(
        ModelReason::from_persisted("ROUTINE_ENGINEERING"),
        Ok(ModelReason::RoutineEngineering)
    );
    assert!(ModelReason::from_persisted("routine_engineering").is_err());
    assert!(!WorkerObservationKind::TurnStarted.is_terminal());
    assert!(WorkerObservationKind::TerminalInterrupted.is_terminal());
    assert_eq!(
        ExecutionAuthoritySource::VerifiedApproval.as_str(),
        "VERIFIED_APPROVAL"
    );
}

#[test]
fn claim_disposition_distinguishes_new_from_exact_replay() {
    assert_ne!(ClaimDisposition::Claimed, ClaimDisposition::ExactReplay);
    assert_ne!(
        ClaimReservationDisposition::Reserved,
        ClaimReservationDisposition::ExactReplay
    );
    assert_ne!(ReplayRecordState::PendingClaim, ReplayRecordState::Retained);
    assert_ne!(
        RestartTaskKind::CapacityWait,
        RestartTaskKind::PromotedNoAttempt
    );
    assert_ne!(
        RestartTaskKind::DraftPendingPromotion,
        RestartTaskKind::PromotedNoAttempt
    );
    assert_ne!(
        RestartTaskKind::DraftProjectReconciliationRequired,
        RestartTaskKind::DraftPendingPromotion
    );
    assert_ne!(
        RestartTaskKind::ProjectReconciliationRequired,
        RestartTaskKind::AttemptReconcileRequired
    );
    assert_ne!(
        RestartTaskKind::WriterReconciliationRequired,
        RestartTaskKind::AttemptReconcileRequired
    );
    assert_ne!(AdapterErrorKind::QuotaRejected, AdapterErrorKind::Database);
    assert_eq!(MAX_ARTIFACTS_PER_ATTEMPT, 64);
    assert_eq!(MAX_ARTIFACT_BYTES_PER_ATTEMPT, 8_388_608);
    assert_eq!(MAX_ARTIFACTS_PER_TASK, 192);
    assert_eq!(MAX_ARTIFACT_BYTES_PER_TASK, 25_165_824);
    assert_ne!(
        ProviderDispatchKind::WorkerThread,
        ProviderDispatchKind::WorkerTurn
    );
    assert_ne!(
        ProviderDispatchKind::ReviewThread,
        ProviderDispatchKind::ReviewTurn
    );
    assert_eq!(ProviderDispatchKind::WorkerThread.as_str(), "WORKER_THREAD");
}

#[test]
fn restart_and_owner_verified_persistence_api_is_typed() {
    let _: fn(&mut PostgresForeman, u16) -> Result<Vec<ActiveTaskRef>, AdapterError> =
        PostgresForeman::list_active_task_refs;
    let _: fn(&mut PostgresForeman, u16) -> Result<Vec<RestartTaskRef>, AdapterError> =
        PostgresForeman::list_restart_task_refs;
    let _: fn(
        &mut PostgresForeman,
        Option<&RestartTaskCursor>,
        u16,
    ) -> Result<Vec<RestartTaskRef>, AdapterError> = PostgresForeman::list_restart_task_refs_page;
    let _: fn(
        &mut PostgresForeman,
        &VerifiedWorkerAttemptRecord,
        u8,
    ) -> Result<ClaimReservationDisposition, AdapterError> =
        PostgresForeman::reserve_worker_attempt;
    let _: fn(
        &mut PostgresForeman,
        &VerifiedWorkerAttemptRecord,
        u8,
        &str,
    ) -> Result<ClaimReservationDisposition, AdapterError> =
        PostgresForeman::reserve_worker_attempt_with_execution_environment_ref;
    let _: fn(
        &mut PostgresForeman,
        &VerifiedWorkerAttemptRecord,
        u8,
    ) -> Result<ClaimOutcome, AdapterError> = PostgresForeman::claim_worker_attempt;
    let _: fn(
        &mut PostgresForeman,
        &VerifiedWorkerAttemptRecord,
        u8,
        &str,
    ) -> Result<ClaimOutcome, AdapterError> =
        PostgresForeman::claim_worker_attempt_with_execution_environment_ref;
    let _: fn(
        &mut PostgresForeman,
        &ContentDigest,
    ) -> Result<Option<PendingWorkerAttempt>, AdapterError> =
        PostgresForeman::load_pending_worker_attempt;
    let _: fn(
        &mut PostgresForeman,
        &VerifiedTaskExecutionBinding,
        &WorkerBudget,
        &ManagedPromotionSource,
    ) -> Result<AppendDisposition, AdapterError> = PostgresForeman::record_task_promotion;
    let _: fn(
        &mut PostgresForeman,
        &ManagedPromotionIntent,
    ) -> Result<AppendDisposition, AdapterError> = PostgresForeman::record_promotion_intent;
    let _: fn(
        &mut PostgresForeman,
        &ContentDigest,
    ) -> Result<Option<ManagedPromotionIntent>, AdapterError> =
        PostgresForeman::load_promotion_intent;
    let _: fn(
        &mut PostgresForeman,
        &ManagedPreparationObservation,
    ) -> Result<AppendDisposition, AdapterError> = PostgresForeman::record_preparation_observation;
    let _: fn(
        &mut PostgresForeman,
        &ContentDigest,
    ) -> Result<Option<ManagedPreparationObservation>, AdapterError> =
        PostgresForeman::load_preparation_observation;
    let _: fn(
        &mut PostgresForeman,
        &ContentDigest,
    ) -> Result<Option<ManagedPromotionSource>, AdapterError> =
        PostgresForeman::load_task_promotion_source;
    let _: fn(&mut PostgresForeman, &ContentDigest) -> Result<WorkerBudget, AdapterError> =
        PostgresForeman::load_worker_budget;
    let _: fn(
        &mut PostgresForeman,
        &ContentDigest,
        u8,
    ) -> Result<Vec<VerifiedManagedEvidence>, AdapterError> =
        PostgresForeman::load_managed_evidence;
    let _: fn(
        &mut PostgresForeman,
        &VerifiedManagedEvidence,
        &lattice_task_ledger::TaskRuntimeEventLink,
        &CorrelationId,
        &str,
    ) -> Result<AppendDisposition, AdapterError> = PostgresForeman::stage_artifact_reference;
    let _: fn(
        &mut PostgresForeman,
        &ContentDigest,
        u8,
        &ContentDigest,
    ) -> Result<AppendDisposition, AdapterError> =
        PostgresForeman::finalize_staged_artifact_reference;
    let _: fn(
        &mut PostgresForeman,
        &ContentDigest,
    ) -> Result<Option<StagedArtifactReference>, AdapterError> =
        PostgresForeman::load_staged_artifact_reference;
    let _: fn(
        &mut PostgresForeman,
        &ContentDigest,
    ) -> Result<PersistedTaskRuntimeRows, AdapterError> = PostgresForeman::load_task_runtime_rows;
    let _: fn(
        &mut PostgresForeman,
        &ContentDigest,
    ) -> Result<PersistedReferenceLinks, AdapterError> = PostgresForeman::load_reference_links;
    let _: fn(
        &mut PostgresForeman,
        &ContentDigest,
        &ContentDigest,
    ) -> Result<VerifiedExecutionAuthority, AdapterError> =
        PostgresForeman::load_execution_authority;
    let _: fn(
        &mut PostgresForeman,
        &VerifiedWorkerAttemptRecord,
        ProviderDispatchKind,
        &ContentDigest,
        &ContentDigest,
        &ContentDigest,
    ) -> Result<ClaimDisposition, AdapterError> = PostgresForeman::claim_provider_dispatch;
    let _: fn(
        &mut PostgresForeman,
        &ContentDigest,
        u64,
        ProviderDispatchKind,
    ) -> Result<Option<ProviderDispatchClaim>, AdapterError> =
        PostgresForeman::load_provider_dispatch_claim;
    let _: fn(
        &mut PostgresForeman,
        &ContentDigest,
        u8,
        &str,
        &ContentDigest,
        &ContentDigest,
        u64,
    ) -> Result<AppendDisposition, AdapterError> =
        PostgresForeman::close_retained_worker_without_provider_effect;
    let _: fn(
        &mut PostgresForeman,
        &ContentDigest,
        u8,
    ) -> Result<Option<AttemptClosure>, AdapterError> = PostgresForeman::load_attempt_closure;
    let _: fn(
        &mut PostgresForeman,
        &VerifiedWorkerAttemptRecord,
        &ExecutionEnvironmentDescriptor,
    ) -> Result<AppendDisposition, AdapterError> = PostgresForeman::record_execution_environment;
    let _: fn(
        &mut PostgresForeman,
        &ContentDigest,
        u64,
    ) -> Result<Option<PersistedExecutionEnvironment>, AdapterError> =
        PostgresForeman::load_execution_environment;
    let _: fn(
        &mut PostgresForeman,
        &ContentDigest,
    ) -> Result<Vec<PersistedExecutionEnvironment>, AdapterError> =
        PostgresForeman::load_execution_environments;
    assert_eq!(
        NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF,
        "execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000001"
    );
}

#[test]
fn wsl2_execution_environment_is_typed_canonical_and_digest_bound() {
    let exact = execution_environment_fixture();
    assert_eq!(exact.kind(), ExecutionEnvironmentKind::Wsl2Linux);
    assert_eq!(
        exact.descriptor_schema(),
        "lattice.execution-environment.wsl2-linux/1.1"
    );
    assert_eq!(exact.distribution(), "Ubuntu");
    assert_eq!(exact.distribution_version(), "26.04");
    assert_eq!(
        exact.linux_repository_path(),
        "/home/lattice/managed-worktrees/task-1"
    );
    assert_eq!(exact.linux_codex_home_path(), "/home/lattice/codex-home");
    assert_eq!(
        exact.credential_authority_kind(),
        CredentialAuthorityKind::LinuxKeyring
    );
    assert_eq!(
        exact.keyring_library_manifest_ref(),
        format!("keyring-library-manifest:sha256:{}", "f".repeat(64))
    );
    assert_eq!(
        exact.keyring_library_manifest_digest().as_str(),
        "f".repeat(64)
    );
    assert_eq!(exact.sandbox_helper().path(), "/usr/bin/bwrap");
    assert_eq!(exact.sandbox_helper().version(), "bubblewrap 0.11.1");
    assert_eq!(
        exact.environment_ref().as_str(),
        format!(
            "execution-environment:sha256:{}",
            exact.descriptor_digest().as_str()
        )
    );
    assert_eq!(exact.execution_domain_digest(), exact.descriptor_digest());
    assert_eq!(exact.as_json(), exact.canonical_json());
    assert_eq!(
        exact.process_fence_kind().as_str(),
        "SYSTEMD_USER_SERVICE_CGROUP_V2"
    );
    assert_eq!(exact.supervisor_bootstrap_node().path(), "/usr/bin/node");
    assert_eq!(exact.supervisor_bootstrap_node().version(), "v22.22.1");
    assert_eq!(exact.immutable_probe_lsattr().path(), "/usr/bin/lsattr");
    assert_eq!(exact.noninteractive_root_probe().path(), "/usr/bin/sudo");
    assert_eq!(
        exact.immutable_snapshot_ref(),
        format!(
            "wsl2-immutable-snapshot:sha256:{}",
            exact.immutable_snapshot_digest().as_str()
        )
    );
    assert_eq!(
        exact.sandbox_policy_ref(),
        "wsl2-sandbox-policy:sha256:f71d706a2a55446bdf292ca950c05b8fe2c30d6f6ac08df89274a05831522822"
    );
    assert_eq!(
        exact.sandbox_policy_digest().as_str(),
        "f71d706a2a55446bdf292ca950c05b8fe2c30d6f6ac08df89274a05831522822"
    );
    assert_eq!(
        exact.privilege_boundary_ref(),
        format!(
            "wsl2-privilege-boundary:sha256:{}",
            exact.privilege_boundary_digest().as_str()
        )
    );

    let changed_path = fixture_with_repository_path("/home/lattice/managed-worktrees/task-2")
        .expect("changed canonical path");
    assert_ne!(
        exact.execution_domain_digest(),
        changed_path.execution_domain_digest()
    );
    assert_ne!(exact.descriptor_digest(), changed_path.descriptor_digest());
    assert_ne!(exact.environment_ref(), changed_path.environment_ref());
    assert_ne!(
        exact.sandbox_policy_ref(),
        changed_path.sandbox_policy_ref(),
        "sandbox cwd substitution must produce a distinct canonical policy digest"
    );

    let mut stale_sandbox_policy = execution_environment_json('8');
    let retained_policy = stale_sandbox_policy["sandbox_policy"]["policy_digest"].clone();
    let substituted_repository = "/home/lattice/managed-worktrees/task-2";
    stale_sandbox_policy["linux"]["cwd"] = Value::String(substituted_repository.to_owned());
    stale_sandbox_policy["path_mapping"]["linux_path"] =
        Value::String(substituted_repository.to_owned());
    stale_sandbox_policy["path_mapping"]["windows_path"] =
        Value::String(r"\\wsl.localhost\Ubuntu\home\lattice\managed-worktrees\task-2".to_owned());
    rehash_environment(&mut stale_sandbox_policy);
    stale_sandbox_policy["sandbox_policy"]["policy_digest"] = retained_policy;
    rehash_environment_identity_only(&mut stale_sandbox_policy);
    assert!(
        ExecutionEnvironmentDescriptor::from_json(
            &serde_json::to_string(&stale_sandbox_policy).expect("stale sandbox policy JSON")
        )
        .is_err(),
        "descriptor substitution must not bypass the canonical sandbox-policy digest"
    );

    let mut forged_sandbox_policy = execution_environment_json('8');
    forged_sandbox_policy["sandbox_policy"]["policy_digest"] =
        Value::String(format!("wsl2-sandbox-policy:sha256:{}", "4".repeat(64)));
    rehash_environment_identity_only(&mut forged_sandbox_policy);
    assert!(
        ExecutionEnvironmentDescriptor::from_json(
            &serde_json::to_string(&forged_sandbox_policy).expect("forged sandbox policy JSON")
        )
        .is_err(),
        "well-shaped but noncanonical sandbox-policy digest must fail closed"
    );

    let mut substituted_path_mapping_digest = execution_environment_json('8');
    substituted_path_mapping_digest["path_mapping"]["digest"] =
        Value::String(format!("path-mapping:sha256:{}", "a".repeat(64)));
    rehash_environment_identity_only(&mut substituted_path_mapping_digest);
    assert!(
        ExecutionEnvironmentDescriptor::from_json(
            &serde_json::to_string(&substituted_path_mapping_digest)
                .expect("substituted path-mapping digest JSON")
        )
        .is_err(),
        "coherent outer identity must not admit a substituted path-mapping digest"
    );

    let changed_toolchain = fixture_with_cargo_digest('a').expect("changed toolchain identity");
    assert_ne!(
        exact.execution_domain_digest(),
        changed_toolchain.execution_domain_digest()
    );
    assert_ne!(exact.environment_ref(), changed_toolchain.environment_ref());

    let mut changed_manifest_json = execution_environment_json('8');
    changed_manifest_json["linux"]["keyring_library_manifest_digest"] = Value::String(format!(
        "keyring-library-manifest:sha256:{}",
        "1".repeat(64)
    ));
    rehash_environment(&mut changed_manifest_json);
    let changed_manifest = ExecutionEnvironmentDescriptor::from_json(
        &serde_json::to_string(&changed_manifest_json).expect("changed manifest JSON"),
    )
    .expect("changed keyring-library manifest");
    assert_ne!(
        exact.credential_authority_ref(),
        changed_manifest.credential_authority_ref()
    );
    assert_ne!(exact.environment_ref(), changed_manifest.environment_ref());

    let mut changed_helper_json = execution_environment_json('8');
    changed_helper_json["verification_toolchain"]["sandbox_helper"]["sha256"] =
        Value::String("4".repeat(64));
    rehash_environment(&mut changed_helper_json);
    let changed_helper = ExecutionEnvironmentDescriptor::from_json(
        &serde_json::to_string(&changed_helper_json).expect("changed helper JSON"),
    )
    .expect("changed sandbox helper");
    assert_ne!(
        exact.verification_toolchain_identity_ref(),
        changed_helper.verification_toolchain_identity_ref()
    );
    assert_ne!(exact.environment_ref(), changed_helper.environment_ref());

    for pointer in [
        "/gateway/version",
        "/linux/launcher_version",
        "/linux/node_version",
        "/linux/git_version",
        "/process_fence/systemd_run_version",
        "/process_fence/systemctl_version",
        "/process_fence/supervisor_bootstrap_node/version",
        "/process_fence/immutable_probe_lsattr/version",
        "/process_fence/noninteractive_root_probe/version",
        "/verification_toolchain/npm/version",
        "/verification_toolchain/cargo/version",
        "/verification_toolchain/rustc/version",
        "/verification_toolchain/rustdoc/version",
        "/verification_toolchain/sandbox_helper/version",
    ] {
        for secret in ["token=fixture", "password=fixture", "secret=fixture"] {
            let mut secret_bearing_version = execution_environment_json('8');
            let version = secret_bearing_version
                .pointer_mut(pointer)
                .and_then(|value| value.as_str())
                .expect("version fixture")
                .to_owned();
            *secret_bearing_version
                .pointer_mut(pointer)
                .expect("mutable version fixture") = Value::String(format!("{version} {secret}"));
            rehash_environment(&mut secret_bearing_version);
            assert!(
                ExecutionEnvironmentDescriptor::from_json(
                    &serde_json::to_string(&secret_bearing_version)
                        .expect("secret-bearing version descriptor JSON")
                )
                .is_err(),
                "digest-valid credential-like tool output passed at {pointer}: {secret}"
            );
        }
    }

    for (pointer, invalid) in [
        ("/gateway/version", "1234567.6.1".to_owned()),
        (
            "/process_fence/systemd_run_version",
            "systemd 259 ()".to_owned(),
        ),
        (
            "/process_fence/noninteractive_root_probe/version",
            format!("sudo-rs 0.2.13-{}", "a".repeat(65)),
        ),
    ] {
        let mut noncanonical_version = execution_environment_json('8');
        *noncanonical_version
            .pointer_mut(pointer)
            .expect("mutable version boundary fixture") = Value::String(invalid);
        rehash_environment(&mut noncanonical_version);
        assert!(
            ExecutionEnvironmentDescriptor::from_json(
                &serde_json::to_string(&noncanonical_version)
                    .expect("version boundary descriptor JSON")
            )
            .is_err(),
            "noncanonical version boundary passed at {pointer}"
        );
    }

    let mut missing_manifest = execution_environment_json('8');
    missing_manifest["linux"]
        .as_object_mut()
        .expect("linux object")
        .remove("keyring_library_manifest_digest");
    assert!(
        ExecutionEnvironmentDescriptor::from_json(
            &serde_json::to_string(&missing_manifest).expect("missing manifest JSON")
        )
        .is_err()
    );

    let mut substituted_helper_path = execution_environment_json('8');
    substituted_helper_path["verification_toolchain"]["sandbox_helper"]["path"] =
        Value::String("/home/lattice/toolchain/bwrap".to_owned());
    rehash_environment(&mut substituted_helper_path);
    assert!(
        ExecutionEnvironmentDescriptor::from_json(
            &serde_json::to_string(&substituted_helper_path).expect("substituted helper path JSON")
        )
        .is_err()
    );

    let mut substituted_bootstrap_node = execution_environment_json('8');
    substituted_bootstrap_node["process_fence"]["supervisor_bootstrap_node"]["path"] =
        Value::String("/home/lattice/toolchain-node/bin/node".to_owned());
    rehash_environment(&mut substituted_bootstrap_node);
    assert!(
        ExecutionEnvironmentDescriptor::from_json(
            &serde_json::to_string(&substituted_bootstrap_node)
                .expect("substituted bootstrap node JSON")
        )
        .is_err()
    );

    let mut substituted_launcher_path = execution_environment_json('8');
    let substituted_launcher = "/home/lattice/codex/codex";
    substituted_launcher_path["linux"]["launcher_path"] =
        Value::String(substituted_launcher.to_owned());
    substituted_launcher_path["verification_toolchain"]["sandbox"]["path"] =
        Value::String(substituted_launcher.to_owned());
    rehash_environment(&mut substituted_launcher_path);
    assert!(
        ExecutionEnvironmentDescriptor::from_json(
            &serde_json::to_string(&substituted_launcher_path)
                .expect("substituted launcher path JSON")
        )
        .is_err(),
        "contained launcher substitution must not bypass the exact immutable-tree path"
    );

    for (field, substituted_path) in [
        (
            "keyring_daemon_path",
            "/home/lattice/keyring-static-v1/root/usr/libexec/gnome-keyring-daemon",
        ),
        (
            "keyring_library_path",
            "/home/lattice/keyring-static-v1/root/usr/lib/x86_64-linux-gnu",
        ),
    ] {
        let mut substituted_keyring_path = execution_environment_json('8');
        substituted_keyring_path["linux"][field] = Value::String(substituted_path.to_owned());
        rehash_environment(&mut substituted_keyring_path);
        assert!(
            ExecutionEnvironmentDescriptor::from_json(
                &serde_json::to_string(&substituted_keyring_path)
                    .expect("substituted keyring path JSON")
            )
            .is_err(),
            "contained keyring substitution must not bypass the exact immutable-tree path: {field}"
        );
    }

    for probe in ["immutable_probe_lsattr", "noninteractive_root_probe"] {
        let mut substituted_probe = execution_environment_json('8');
        substituted_probe["process_fence"][probe]["path"] =
            Value::String("/usr/bin/true".to_owned());
        rehash_environment(&mut substituted_probe);
        assert!(
            ExecutionEnvironmentDescriptor::from_json(
                &serde_json::to_string(&substituted_probe).expect("substituted process probe JSON")
            )
            .is_err(),
            "substituted process probe path accepted: {probe}"
        );
    }

    let mut obsolete_node = execution_environment_json('8');
    obsolete_node["linux"]["node_version"] = Value::String("v24.14.9".to_owned());
    rehash_environment(&mut obsolete_node);
    assert!(
        ExecutionEnvironmentDescriptor::from_json(
            &serde_json::to_string(&obsolete_node).expect("obsolete node JSON")
        )
        .is_err()
    );

    let mut substituted_snapshot = execution_environment_json('8');
    substituted_snapshot["immutable_snapshot"]["trees"]["node"]["manifest_digest"] =
        Value::String(format!("immutable-tree-manifest:sha256:{}", "a".repeat(64)));
    rehash_environment_identity_only(&mut substituted_snapshot);
    assert!(
        ExecutionEnvironmentDescriptor::from_json(
            &serde_json::to_string(&substituted_snapshot).expect("substituted snapshot JSON")
        )
        .is_err(),
        "tree substitution must not bypass the nested immutable-snapshot digest"
    );

    for invalid_device in [Value::from(2096_u64), Value::String("02096".to_owned())] {
        let mut invalid_snapshot_identity = execution_environment_json('8');
        invalid_snapshot_identity["immutable_snapshot"]["task_root_device"] = invalid_device;
        rehash_environment(&mut invalid_snapshot_identity);
        assert!(
            ExecutionEnvironmentDescriptor::from_json(
                &serde_json::to_string(&invalid_snapshot_identity)
                    .expect("invalid snapshot identity JSON")
            )
            .is_err(),
            "noncanonical immutable device identity must fail closed"
        );
    }

    let mut duplicate_snapshot_root = execution_environment_json('8');
    let combined_root = "/home/lattice/combined-toolchain";
    duplicate_snapshot_root["immutable_snapshot"]["trees"]["node"]["root"] =
        Value::String(combined_root.to_owned());
    duplicate_snapshot_root["immutable_snapshot"]["trees"]["rust"]["root"] =
        Value::String(combined_root.to_owned());
    duplicate_snapshot_root["linux"]["node_path"] = Value::String(format!("{combined_root}/node"));
    duplicate_snapshot_root["verification_toolchain"]["npm"]["path"] =
        Value::String(format!("{combined_root}/npm-cli.js"));
    for tool in ["cargo", "rustc", "rustdoc"] {
        duplicate_snapshot_root["verification_toolchain"][tool]["path"] =
            Value::String(format!("{combined_root}/{tool}"));
    }
    rehash_environment(&mut duplicate_snapshot_root);
    assert!(
        ExecutionEnvironmentDescriptor::from_json(
            &serde_json::to_string(&duplicate_snapshot_root).expect("duplicate snapshot root JSON")
        )
        .is_err(),
        "duplicate immutable snapshot roots must fail closed"
    );

    let mut overlapping_snapshot_roots = execution_environment_json('8');
    let node_root = "/home/lattice/combined-toolchain";
    let rust_root = format!("{node_root}/rust");
    overlapping_snapshot_roots["immutable_snapshot"]["trees"]["node"]["root"] =
        Value::String(node_root.to_owned());
    overlapping_snapshot_roots["immutable_snapshot"]["trees"]["rust"]["root"] =
        Value::String(rust_root.clone());
    overlapping_snapshot_roots["linux"]["node_path"] = Value::String(format!("{node_root}/node"));
    overlapping_snapshot_roots["verification_toolchain"]["npm"]["path"] =
        Value::String(format!("{node_root}/npm-cli.js"));
    for tool in ["cargo", "rustc", "rustdoc"] {
        overlapping_snapshot_roots["verification_toolchain"][tool]["path"] =
            Value::String(format!("{rust_root}/{tool}"));
    }
    rehash_environment(&mut overlapping_snapshot_roots);
    assert!(
        ExecutionEnvironmentDescriptor::from_json(
            &serde_json::to_string(&overlapping_snapshot_roots)
                .expect("overlapping snapshot roots JSON")
        )
        .is_err(),
        "nested immutable snapshot roots must fail closed"
    );

    let mut nested_snapshot_root = execution_environment_json('8');
    let nested_codex_root = "/home/lattice/immutable/codex";
    let nested_launcher = format!("{nested_codex_root}/bin/codex");
    nested_snapshot_root["immutable_snapshot"]["trees"]["codex"]["root"] =
        Value::String(nested_codex_root.to_owned());
    nested_snapshot_root["linux"]["launcher_path"] = Value::String(nested_launcher.clone());
    nested_snapshot_root["verification_toolchain"]["sandbox"]["path"] =
        Value::String(nested_launcher);
    rehash_environment(&mut nested_snapshot_root);
    assert!(
        ExecutionEnvironmentDescriptor::from_json(
            &serde_json::to_string(&nested_snapshot_root).expect("nested snapshot root JSON")
        )
        .is_err(),
        "immutable-tree roots must be direct task-root children"
    );

    let mut root_available = execution_environment_json('8');
    root_available["privilege_boundary"]["noninteractive_root_unavailable"] = Value::Bool(false);
    rehash_environment(&mut root_available);
    assert!(
        ExecutionEnvironmentDescriptor::from_json(
            &serde_json::to_string(&root_available).expect("root-available boundary JSON")
        )
        .is_err(),
        "noninteractive root availability must fail closed"
    );

    for codex_home in [
        "/home/lattice/task/managed-worktrees/work-8",
        "/home/lattice/task/managed-worktrees/work-8/.codex",
        "/home/lattice/task/managed-worktrees",
    ] {
        let mut overlapping_home = execution_environment_json('8');
        overlapping_home["linux"]["codex_home"] = Value::String(codex_home.to_owned());
        rehash_environment(&mut overlapping_home);
        assert!(
            ExecutionEnvironmentDescriptor::from_json(
                &serde_json::to_string(&overlapping_home).expect("overlapping CODEX_HOME JSON")
            )
            .is_err(),
            "worktree-overlapping CODEX_HOME must fail closed: {codex_home}"
        );
    }

    for missing in ["immutable_snapshot", "sandbox_policy", "privilege_boundary"] {
        let mut descriptor = execution_environment_json('8');
        descriptor
            .as_object_mut()
            .expect("descriptor")
            .remove(missing);
        assert!(
            ExecutionEnvironmentDescriptor::from_json(
                &serde_json::to_string(&descriptor).expect("missing typed object JSON")
            )
            .is_err(),
            "missing top-level typed object accepted: {missing}"
        );
    }

    for path in [
        "relative/repository",
        "/home/lattice/../escape",
        "/mnt/c/Users/lattice/repository",
        "/home/lattice//repository",
        "/home/lattice/repository/",
        "/home/lattice/managed-worktrees/task space",
    ] {
        assert!(
            fixture_with_repository_path(path).is_err(),
            "noncanonical Linux path accepted: {path:?}"
        );
    }
}

#[test]
fn digest_valid_execution_environment_rejects_credential_shaped_string_leaves() {
    let reject = |mut descriptor: Value, label: &str, sentinel: &str| {
        rehash_environment(&mut descriptor);
        let encoded = serde_json::to_string(&descriptor).expect("credential descriptor JSON");
        let error = ExecutionEnvironmentDescriptor::from_json(&encoded)
            .expect_err("credential-shaped descriptor leaf must fail closed");
        assert_eq!(error.code(), "FOREMAN_ADAPTER_INVALID_INPUT", "{label}");
        assert!(
            !format!("{error:?}").contains(sentinel),
            "{label} leaked input"
        );
    };

    for sentinel in [
        "password=phase4-password-sentinel",
        "token=phase4-token-sentinel",
        "secret=phase4-secret-sentinel",
        "api key=phase4-api-key-sentinel",
        "Bearer phase4-bearer-sentinel",
    ] {
        let mut descriptor = execution_environment_json('8');
        descriptor["distribution_identity"]["kernel_release"] =
            Value::String(format!("{sentinel}-6.18.33.2-microsoft-standard-WSL2"));
        reject(descriptor, "kernel credential leaf", sentinel);
    }

    let task_root_sentinel = "ghp_phase4taskrootsentinel";
    let mut task_root = execution_environment_json('8');
    let old_root = task_root["verification_toolchain"]["task_root"]
        .as_str()
        .expect("task root")
        .to_owned();
    let new_root = format!("/home/{task_root_sentinel}");
    replace_descriptor_string_prefix(&mut task_root, &old_root, &new_root);
    let repository = task_root["linux"]["cwd"]
        .as_str()
        .expect("replaced repository");
    task_root["path_mapping"]["windows_path"] = Value::String(format!(
        r"\\wsl.localhost\Ubuntu{}",
        repository.replace('/', "\\")
    ));
    reject(task_root, "task-root credential leaf", task_root_sentinel);

    let home_sentinel = "github_pat_phase4homesentinel";
    let mut home = execution_environment_json('8');
    let isolation = home["verification_toolchain"]["isolation_root"]
        .as_str()
        .expect("isolation root")
        .to_owned();
    home["verification_toolchain"]["home_dir"] =
        Value::String(format!("{isolation}/{home_sentinel}"));
    reject(home, "home credential leaf", home_sentinel);

    let cwd_sentinel = "sk-phase4repositorysentinel";
    let mut cwd = execution_environment_json('8');
    let root = cwd["verification_toolchain"]["task_root"]
        .as_str()
        .expect("task root")
        .to_owned();
    let repository = format!("{root}/managed-worktrees/{cwd_sentinel}");
    cwd["linux"]["cwd"] = Value::String(repository.clone());
    cwd["path_mapping"]["linux_path"] = Value::String(repository.clone());
    cwd["path_mapping"]["windows_path"] = Value::String(format!(
        r"\\wsl.localhost\Ubuntu{}",
        repository.replace('/', "\\")
    ));
    reject(cwd, "cwd credential leaf", cwd_sentinel);

    let tool_sentinel = "gho_phase4toolpathsentinel";
    let mut tool = execution_environment_json('8');
    let node_root = tool["immutable_snapshot"]["trees"]["node"]["root"]
        .as_str()
        .expect("node tree root")
        .to_owned();
    tool["linux"]["node_path"] = Value::String(format!("{node_root}/root/bin/{tool_sentinel}"));
    reject(tool, "tool credential leaf", tool_sentinel);
}

fn execution_environment_fixture() -> ExecutionEnvironmentDescriptor {
    fixture_with_repository_path("/home/lattice/managed-worktrees/task-1")
        .expect("typed WSL2 execution environment")
}

fn fixture_with_repository_path(
    repository_path: &str,
) -> Result<ExecutionEnvironmentDescriptor, AdapterError> {
    let mut descriptor = execution_environment_json('8');
    descriptor["linux"]["cwd"] = Value::String(repository_path.to_owned());
    descriptor["path_mapping"]["linux_path"] = Value::String(repository_path.to_owned());
    descriptor["path_mapping"]["windows_path"] = Value::String(format!(
        r"\\wsl.localhost\Ubuntu{}",
        repository_path.replace('/', "\\")
    ));
    rehash_environment(&mut descriptor);
    ExecutionEnvironmentDescriptor::from_json(
        &serde_json::to_string(&descriptor).expect("descriptor JSON"),
    )
}

fn fixture_with_cargo_digest(byte: char) -> Result<ExecutionEnvironmentDescriptor, AdapterError> {
    let descriptor = execution_environment_json(byte);
    ExecutionEnvironmentDescriptor::from_json(
        &serde_json::to_string(&descriptor).expect("descriptor JSON"),
    )
}

fn execution_environment_json(cargo_digest: char) -> Value {
    let task_ref = "7".repeat(64);
    let task_root = "/home/lattice";
    let isolation_root = format!("{task_root}/verifier-state/{task_ref}");
    let repository = "/home/lattice/managed-worktrees/task-1";
    let launcher = format!("{task_root}/codex/bin/codex");
    let mut descriptor = json!({
        "schema": "lattice.execution-environment.wsl2-linux/1.1",
        "kind": "WSL2_LINUX",
        "distribution": "Ubuntu",
        "distribution_identity": {
            "os_id": "ubuntu", "os_version_id": "26.04", "os_version_codename": "resolute",
            "os_release_sha256": "1".repeat(64),
            "kernel_release": "6.18.33.2-microsoft-standard-WSL2", "identity_digest": Value::Null
        },
        "gateway": { "windows_path": r"C:\Windows\System32\wsl.exe", "version": "2.6.1", "sha256": "2".repeat(64) },
        "linux": {
            "launcher_path": launcher, "launcher_version": "codex-cli 0.146.0", "launcher_sha256": "3".repeat(64),
            "node_path": format!("{task_root}/toolchain-node-24.15.0/root/bin/node"),
            "node_version": "v24.15.0", "node_sha256": "4".repeat(64),
            "git_path": "/usr/bin/git", "git_version": "git version 2.53.0", "git_sha256": "5".repeat(64),
            "supervisor_path": format!("{task_root}/runtime-v1/wsl2-codex-supervisor.mjs"), "supervisor_sha256": "6".repeat(64),
            "codex_home": format!("{task_root}/codex-home"), "config_digest": format!("codex-config:sha256:{}", "7".repeat(64)),
            "cwd": repository, "repository_head": "0123456789abcdef0123456789abcdef01234567",
            "repository_identity": format!("repository:sha256:{}", "8".repeat(64)),
            "dbus_run_session_path": "/usr/bin/dbus-run-session", "dbus_run_session_sha256": "9".repeat(64),
            "setsid_path": "/usr/bin/setsid", "setsid_sha256": "a".repeat(64),
            "keyring_daemon_path": format!("{task_root}/keyring-static-v1/root/usr/bin/gnome-keyring-daemon"), "keyring_daemon_sha256": "b".repeat(64),
            "keyring_library_path": format!("{task_root}/keyring-static-v1/packages"),
            "keyring_library_manifest_digest": format!("keyring-library-manifest:sha256:{}", "f".repeat(64)),
            "xdg_runtime_dir": "/run/user/1000"
        },
        "credential_authority": { "kind": "LINUX_KEYRING", "authority_digest": Value::Null },
        "process_fence": {
            "schema": "lattice.wsl2-cgroup-v2-fence/1.0", "kind": "SYSTEMD_USER_SERVICE_CGROUP_V2",
            "systemd_run_path": "/usr/bin/systemd-run", "systemd_run_version": "systemd 259", "systemd_run_sha256": "c".repeat(64),
            "systemctl_path": "/usr/bin/systemctl", "systemctl_version": "systemd 259", "systemctl_sha256": "d".repeat(64),
            "cgroup_mount": "/sys/fs/cgroup", "user_runtime_dir": "/run/user/1000",
            "unit_prefix": "lattice-wsl2-7777777777777777",
            "supervisor_bootstrap_node": {
                "path": "/usr/bin/node", "version": "v22.22.1", "sha256": "8".repeat(64)
            },
            "immutable_probe_lsattr": {
                "path": "/usr/bin/lsattr", "version": "lsattr 1.47.2 (1-Jan-2025)", "sha256": "9".repeat(64)
            },
            "noninteractive_root_probe": {
                "path": "/usr/bin/sudo", "version": "Sudo version 1.9.16p2", "sha256": "a".repeat(64)
            },
            "identity_digest": Value::Null
        },
        "verification_toolchain": {
            "schema": "lattice.wsl2-verification-toolchain/1.0", "task_ref": task_ref, "task_root": task_root,
            "isolation_root": isolation_root.clone(), "owner_uid": 1000,
            "home_dir": format!("{isolation_root}/home"), "temp_dir": format!("{isolation_root}/tmp"),
            "npm_cache": format!("{isolation_root}/npm-cache"), "cargo_home": format!("{isolation_root}/cargo-home"),
            "cargo_target_dir": format!("{isolation_root}/cargo-target"), "cargo_host": "x86_64-unknown-linux-gnu",
            "npm": { "path": format!("{task_root}/toolchain-node-24.15.0/root/lib/node_modules/npm/bin/npm-cli.js"), "version": "11.12.1", "sha256": "e".repeat(64) },
            "cargo": { "path": format!("{task_root}/toolchain-rust-1.97.1/bin/cargo"), "version": "cargo 1.97.1 (c980f4866 2026-06-30)", "sha256": cargo_digest.to_string().repeat(64) },
            "rustc": { "path": format!("{task_root}/toolchain-rust-1.97.1/bin/rustc"), "version": "rustc 1.97.1 (8bab26f4f 2026-07-14)", "sha256": "1".repeat(64) },
            "rustdoc": { "path": format!("{task_root}/toolchain-rust-1.97.1/bin/rustdoc"), "version": "rustdoc 1.97.1 (8bab26f4f 2026-07-14)", "sha256": "2".repeat(64) },
            "sandbox": { "path": format!("{task_root}/codex/bin/codex"), "version": "codex-cli 0.146.0", "sha256": "3".repeat(64) },
            "sandbox_helper": { "path": "/usr/bin/bwrap", "version": "bubblewrap 0.11.1", "sha256": "6".repeat(64) },
            "identity_digest": Value::Null
        },
        "immutable_snapshot": {
            "schema": "lattice.wsl2-immutable-snapshot/1.0",
            "task_root_path": task_root,
            "task_root_device": "2096",
            "task_root_inode": "36226",
            "task_root_owner_uid": 0,
            "task_root_owner_gid": 0,
            "task_root_mode": "0555",
            "task_root_immutable": true,
            "trees": {
                "codex": {
                    "root": format!("{task_root}/codex"),
                    "manifest_digest": format!("immutable-tree-manifest:sha256:{}", "1".repeat(64))
                },
                "supervisor_runtime": {
                    "root": format!("{task_root}/runtime-v1"),
                    "manifest_digest": format!("immutable-tree-manifest:sha256:{}", "2".repeat(64))
                },
                "node": {
                    "root": format!("{task_root}/toolchain-node-24.15.0"),
                    "manifest_digest": format!("immutable-tree-manifest:sha256:{}", "3".repeat(64))
                },
                "rust": {
                    "root": format!("{task_root}/toolchain-rust-1.97.1"),
                    "manifest_digest": format!("immutable-tree-manifest:sha256:{}", "4".repeat(64))
                },
                "keyring": {
                    "root": format!("{task_root}/keyring-static-v1"),
                    "manifest_digest": format!("immutable-tree-manifest:sha256:{}", "5".repeat(64))
                }
            },
            "snapshot_digest": Value::Null
        },
        "sandbox_policy": {
            "schema": "lattice.wsl2-sandbox-policy/1.0",
            "policy_digest": format!("wsl2-sandbox-policy:sha256:{}", "9".repeat(64))
        },
        "privilege_boundary": {
            "schema": "lattice.wsl2-privilege-boundary/1.0",
            "effective_uid": 1000,
            "effective_gid": 1000,
            "effective_capabilities_digest": format!("linux-capabilities:sha256:{}", "a".repeat(64)),
            "noninteractive_root_unavailable": true,
            "boundary_digest": Value::Null
        },
        "path_mapping": {
            "windows_path": r"\\wsl.localhost\Ubuntu\home\lattice\managed-worktrees\task-1",
            "linux_path": repository, "digest": format!("path-mapping:sha256:{}", "f".repeat(64))
        },
        "identity_digest": Value::Null
    });
    rehash_environment(&mut descriptor);
    descriptor
}

fn rehash_environment(descriptor: &mut Value) {
    let mut distribution = descriptor["distribution_identity"].clone();
    distribution
        .as_object_mut()
        .expect("distribution")
        .remove("identity_digest");
    distribution["distribution"] = descriptor["distribution"].clone();
    descriptor["distribution_identity"]["identity_digest"] =
        Value::String(typed_json_digest("wsl2-distribution", &distribution));
    let credential = json!({
        "kind": descriptor["credential_authority"]["kind"],
        "distribution_identity_ref": descriptor["distribution_identity"]["identity_digest"],
        "codex_home": descriptor["linux"]["codex_home"], "config_digest": descriptor["linux"]["config_digest"],
        "keyring_daemon_path": descriptor["linux"]["keyring_daemon_path"],
        "keyring_daemon_sha256": descriptor["linux"]["keyring_daemon_sha256"],
        "keyring_library_path": descriptor["linux"]["keyring_library_path"],
        "keyring_library_manifest_digest": descriptor["linux"]["keyring_library_manifest_digest"],
        "xdg_runtime_dir": descriptor["linux"]["xdg_runtime_dir"]
    });
    descriptor["credential_authority"]["authority_digest"] =
        Value::String(typed_json_digest("wsl2-credential-authority", &credential));
    let mut fence = descriptor["process_fence"].clone();
    fence
        .as_object_mut()
        .expect("fence")
        .remove("identity_digest");
    fence["distribution_identity_ref"] =
        descriptor["distribution_identity"]["identity_digest"].clone();
    descriptor["process_fence"]["identity_digest"] =
        Value::String(typed_json_digest("wsl2-process-fence-authority", &fence));
    let mut toolchain = descriptor["verification_toolchain"].clone();
    toolchain
        .as_object_mut()
        .expect("toolchain")
        .remove("identity_digest");
    descriptor["verification_toolchain"]["identity_digest"] =
        Value::String(typed_json_digest("wsl2-verification-toolchain", &toolchain));
    let mut snapshot = descriptor["immutable_snapshot"].clone();
    snapshot
        .as_object_mut()
        .expect("immutable snapshot")
        .remove("snapshot_digest");
    descriptor["immutable_snapshot"]["snapshot_digest"] =
        Value::String(typed_json_digest("wsl2-immutable-snapshot", &snapshot));
    descriptor["sandbox_policy"]["policy_digest"] = Value::String(typed_json_digest(
        "wsl2-sandbox-policy",
        &sandbox_policy_template(descriptor),
    ));
    let mut privilege_boundary = descriptor["privilege_boundary"].clone();
    privilege_boundary
        .as_object_mut()
        .expect("privilege boundary")
        .remove("boundary_digest");
    descriptor["privilege_boundary"]["boundary_digest"] = Value::String(typed_json_digest(
        "wsl2-privilege-boundary",
        &privilege_boundary,
    ));
    let path_mapping_subject = json!({
        "distribution": descriptor["distribution"],
        "windows_path": descriptor["path_mapping"]["windows_path"],
        "linux_path": descriptor["path_mapping"]["linux_path"],
        "repository_identity": descriptor["linux"]["repository_identity"],
        "repository_head": descriptor["linux"]["repository_head"]
    });
    descriptor["path_mapping"]["digest"] =
        Value::String(typed_json_digest("path-mapping", &path_mapping_subject));
    rehash_environment_identity_only(descriptor);
}

fn rehash_environment_identity_only(descriptor: &mut Value) {
    let mut subject = descriptor.clone();
    subject
        .as_object_mut()
        .expect("descriptor")
        .remove("identity_digest");
    descriptor["identity_digest"] =
        Value::String(typed_json_digest("execution-environment", &subject));
}

fn replace_descriptor_string_prefix(value: &mut Value, from: &str, to: &str) {
    match value {
        Value::Array(values) => {
            for value in values {
                replace_descriptor_string_prefix(value, from, to);
            }
        }
        Value::Object(object) => {
            for value in object.values_mut() {
                replace_descriptor_string_prefix(value, from, to);
            }
        }
        Value::String(value) if value.starts_with(from) => {
            *value = format!("{to}{}", &value[from.len()..]);
        }
        _ => {}
    }
}

fn sandbox_policy_template(descriptor: &Value) -> Value {
    let linux = &descriptor["linux"];
    let toolchain = &descriptor["verification_toolchain"];
    let task_root = toolchain["task_root"].as_str().expect("task root");
    let linux_home = task_root.split('/').take(3).collect::<Vec<_>>().join("/");
    json!({
        "schema": "lattice.wsl2-sandbox-template/1.0",
        "permission_profile_type": "managed",
        "filesystem_type": "restricted",
        "network": "restricted",
        "base_entries": [
            { "path": { "type": "special", "value": { "kind": "minimal" } }, "access": "read" },
            { "path": { "type": "path", "path": task_root }, "access": "read" }
        ],
        "role_writes": {
            "PREFLIGHT": [
                linux["cwd"], toolchain["home_dir"], toolchain["temp_dir"],
                toolchain["npm_cache"], toolchain["cargo_home"], toolchain["cargo_target_dir"]
            ],
            "NODE": [toolchain["home_dir"], toolchain["temp_dir"], toolchain["npm_cache"]],
            "CARGO": [
                toolchain["home_dir"], toolchain["temp_dir"],
                toolchain["cargo_home"], toolchain["cargo_target_dir"]
            ],
            "GIT": {
                "bootstrap": ["$GIT_CONTROL_HOME", "$GIT_CONTROL_TMPDIR"],
                "guarded_object_write": [
                    "$GIT_CONTROL_HOME", "$GIT_CONTROL_TMPDIR", "$GIT_COMMON_DIR/objects"
                ],
                "guarded_index_write": [
                    "$GIT_CONTROL_HOME", "$GIT_CONTROL_TMPDIR",
                    "$GIT_CONTROL_ROOT/candidate-index"
                ]
            }
        },
        "deny_entries": [
            { "path": linux["codex_home"], "missing_path_behavior": "skip" },
            { "path": format!("{linux_home}/.codex"), "missing_path_behavior": "skip" },
            { "path": "/mnt", "missing_path_behavior": "skip" },
            { "path": linux["xdg_runtime_dir"], "missing_path_behavior": "skip" }
        ],
        "codex_linux_sandbox_exe": Value::Null,
        "sandbox_cwd": format!("file://{}", linux["cwd"].as_str().expect("Linux cwd")),
        "use_legacy_landlock": false
    })
}

fn typed_json_digest(domain: &str, value: &Value) -> String {
    let encoded = serde_json::to_vec(&canonical(value)).expect("canonical JSON");
    let digest = Sha256::digest(encoded)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{domain}:sha256:{digest}")
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), canonical(&object[key])))
                    .collect::<Map<_, _>>(),
            )
        }
        _ => value.clone(),
    }
}

#[test]
fn ledger_event_sequence_parameters_are_text_then_explicitly_cast_to_numeric() {
    let adapter = include_str!("../src/adapter.rs");
    assert_eq!(adapter.matches("::text::numeric").count(), 9);
    assert!(!adapter.contains("$27::numeric"));
    assert!(!adapter.contains("$22::numeric"));
    assert!(!adapter.contains("$14::numeric"));
    assert!(!adapter.contains("$19::numeric"));
    assert!(!adapter.contains("$15::numeric"));
}

#[test]
fn provider_dispatch_claim_reader_keeps_select_from_separated() {
    let adapter = include_str!("../src/adapter.rs");
    assert!(!adapter.contains("claimed_atFROM"));
}

#[test]
fn promotion_source_is_bounded_and_commit_is_exact_lower_sha1() {
    let source = ManagedPromotionSource::new(
        "refs/heads/product/lattice-control-mvp",
        "0123456789abcdef0123456789abcdef01234567",
    )
    .expect("valid source");
    assert_eq!(source.base_ref(), "refs/heads/product/lattice-control-mvp");
    assert_eq!(
        source.base_commit(),
        "0123456789abcdef0123456789abcdef01234567"
    );

    for base_ref in [
        "",
        "refs/remotes/origin/main",
        "refs/heads/has space",
        "refs/heads/has\ncontrol",
        "https://example.invalid/repository",
    ] {
        assert!(
            ManagedPromotionSource::new(base_ref, "0123456789abcdef0123456789abcdef01234567")
                .is_err(),
            "unsafe base ref accepted: {base_ref:?}"
        );
    }
    assert!(
        ManagedPromotionSource::new("x".repeat(256), "0123456789abcdef0123456789abcdef01234567")
            .is_err()
    );
    for base_commit in [
        "0123456789abcdef0123456789abcdef0123456",
        "0123456789abcdef0123456789abcdef012345678",
        "0123456789ABCDEF0123456789ABCDEF01234567",
        "g123456789abcdef0123456789abcdef01234567",
    ] {
        assert!(
            ManagedPromotionSource::new("refs/heads/main", base_commit).is_err(),
            "noncanonical commit accepted: {base_commit}"
        );
    }
}

#[test]
fn preparation_observation_is_typed_rebuttable_and_digest_bound() {
    let blocked = ManagedPreparationObservation::new(
        ContentDigest::from_sha256("1".repeat(64)).expect("task ref"),
        ProjectId::new("project-preparation").expect("project"),
        ProjectSnapshotId::new("snapshot-preparation").expect("snapshot"),
        ContentDigest::from_sha256("2".repeat(64)).expect("authority"),
        ManagedPreparationObservationKind::WorktreeNotClean,
        ContentDigest::from_sha256("3".repeat(64)).expect("subject"),
        "2026-08-27T12:00:00Z",
    )
    .expect("blocked observation");
    assert_eq!(
        blocked.kind().blocker_code(),
        Some("LATTICE_MANAGED_WORKTREE_NOT_CLEAN")
    );
    let cleared = ManagedPreparationObservation::new(
        blocked.task_ref().clone(),
        blocked.project_id().clone(),
        blocked.project_snapshot_id().clone(),
        blocked.project_authority_receipt_digest().clone(),
        ManagedPreparationObservationKind::Cleared,
        ContentDigest::from_sha256("4".repeat(64)).expect("cleared subject"),
        "2026-08-27T12:01:00Z",
    )
    .expect("cleared observation");
    assert_eq!(cleared.kind().blocker_code(), None);
    assert_ne!(blocked.observation_digest(), cleared.observation_digest());
}
