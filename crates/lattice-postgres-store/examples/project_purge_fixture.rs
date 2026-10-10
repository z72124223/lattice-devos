//! Synthetic, opt-in fixture writer. Not a shipped runtime command.
use lattice_contracts::{
    ContentDigest, DaemonEpoch, GitRefIdentity, ProjectClass, ProjectId, RuntimeAdmissionMode,
    RuntimeKind, StoreAuthorityHead, StoreAuthorityRevision, StoreDaemonInstanceId, TaskId,
    TaskLedgerStreamIdentity,
};
use lattice_postgres_store::{
    MigrationTarget, PostgresProjectRegistry, PostgresTaskLedger, apply_control_product_extension,
    apply_migrations, connect_project_purge,
};
use lattice_project_registry::{
    CommandId, RegistryCommand, RegistryCommandOutcome, RegistryDenial, RepositoryObservation,
};
use lattice_task_ledger::{
    ActorId, AppendCommand, CorrelationId, TaskSubmissionEnvelope, VerifiedStream,
};
use postgres::{Config, NoTls};
use serde_json::json;
use std::path::PathBuf;

fn digest(c: char) -> ContentDigest {
    ContentDigest::from_sha256(c.to_string().repeat(64)).unwrap()
}
#[allow(clippy::too_many_lines)] // Preserve the ordered synthetic setup and assertions.
fn main() {
    assert_eq!(
        std::env::var("LATTICE_PURGE_FIXTURE_ONLY").as_deref(),
        Ok("1"),
        "fixture opt-in required"
    );
    let root = PathBuf::from(std::env::var("LATTICE_PURGE_FIXTURE_ROOT").expect("fixture root"));
    assert!(
        root.is_absolute() && root.join("project-purge-fixture.marker").is_file(),
        "fixture marker required"
    );
    let port: u16 = std::env::var("LATTICE_TASK019_PORT")
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        ![4317, 5432, 55432, 58743, 64272].contains(&port),
        "production port rejected"
    );
    let run = std::env::var("LATTICE_TASK019_RUN_ID").unwrap();
    let password = std::env::var("LATTICE_TASK019_PASSWORD").unwrap();
    let (mut admin, target) = connect_project_purge(port, &run, &password).unwrap();
    if std::env::args().nth(1).as_deref() == Some("bootstrap") {
        apply_migrations(&mut admin, &target).unwrap();
        println!("{}", json!({"status":"BOOTSTRAPPED"}));
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("verify-survivor") {
        let mut registry =
            PostgresProjectRegistry::new(runtime(port, &target, &password), &target).unwrap();
        let loaded = registry.load().unwrap();
        assert!(
            loaded
                .state()
                .project(&ProjectId::new("purge-survivor").unwrap())
                .is_some()
        );
        let task:String=admin.query_one("SELECT task_ref::text FROM control.task_submission_envelopes WHERE project_id='purge-survivor'",&[]).unwrap().get(0);
        let mut ledger =
            PostgresTaskLedger::new(runtime(port, &target, &password), &target).unwrap();
        let receipt = ledger
            .load_submission_by_task_ref(&ContentDigest::from_sha256(&task).unwrap())
            .unwrap()
            .unwrap();
        println!(
            "{}",
            json!({"status":"VERIFIED","registryCheckpoint":loaded.state().checkpoint().checkpoint_digest().as_str(),"survivorTaskRef":task,"survivorLedgerHead":receipt.ledger().stream().head().head_digest().as_str()})
        );
        return;
    }
    assert_eq!(
        admin
            .query_one(
                "SELECT count(*)::bigint FROM control.project_registry_commands",
                &[]
            )
            .unwrap()
            .get::<_, i64>(0),
        0,
        "fixture requires empty Registry"
    );
    apply_control_product_extension(&mut admin, &target).unwrap();
    admin.batch_execute("UPDATE control.runtime_admission SET admission_mode='ACTIVE',daemon_instance_id='task050-fresh-process',daemon_epoch=50,authority_revision=50,observation_digest=decode(repeat('a',64),'hex'),authority_head_digest=decode(repeat('b',64),'hex') WHERE singleton").unwrap();
    let authority = StoreAuthorityHead::new(
        RuntimeKind::Live,
        StoreDaemonInstanceId::new("task050-fresh-process").unwrap(),
        DaemonEpoch::new(50).unwrap(),
        RuntimeAdmissionMode::Active,
        StoreAuthorityRevision::new(50).unwrap(),
        digest('a'),
        digest('b'),
    )
    .unwrap();
    let order = if std::env::args().nth(1).as_deref() == Some("target-first") {
        vec![
            ("purge-target", '2', "project-a"),
            ("purge-survivor", '1', "project-b"),
        ]
    } else {
        vec![
            ("purge-survivor", '1', "project-b"),
            ("purge-target", '2', "project-a"),
        ]
    };
    let mut records = Vec::new();
    for (id, byte, directory) in order {
        let path = root.join(directory);
        assert!(path.is_dir(), "fixture project directory missing");
        let mut registry =
            PostgresProjectRegistry::new(runtime(port, &target, &password), &target).unwrap();
        let observation = RepositoryObservation::new(
            path.to_string_lossy().into_owned(),
            digest(byte),
            digest(byte),
            digest(byte),
            GitRefIdentity::new("refs/heads/main", digest(byte)).unwrap(),
        )
        .unwrap();
        let result = registry
            .execute(
                RegistryCommand::register(
                    CommandId::new(format!("fixture-register-{id}")).unwrap(),
                    ProjectId::new(id).unwrap(),
                    ProjectClass::UserProject,
                    observation,
                ),
                authority.clone(),
            )
            .unwrap();
        let project = result.semantic_receipt().authority().unwrap();
        let identity = TaskLedgerStreamIdentity::new_general_task_intake(
            ProjectId::new(id).unwrap(),
            project.project_snapshot_id().clone(),
            TaskId::new(format!("TASK-{}", id.to_ascii_uppercase())).unwrap(),
            "1",
            digest(byte),
        )
        .unwrap();
        let submission = TaskSubmissionEnvelope::new(
            "lattice_task_submit.v1",
            format!("fixture-{id}"),
            format!("Synthetic task for {id}"),
            id,
            identity.clone(),
            project.receipt_digest().clone(),
        )
        .unwrap();
        let vacant = VerifiedStream::vacant(identity, RuntimeKind::Live).unwrap();
        let command = AppendCommand::new_general_task_created(
            vacant.head().clone(),
            lattice_task_ledger::CommandId::new(format!("mcp-submit:fixture-{id}")).unwrap(),
            CorrelationId::new("general-task-intake-v1").unwrap(),
            "2026-10-09T00:00:00Z",
            ActorId::new("lattice-mcp").unwrap(),
            &submission,
        )
        .unwrap();
        let mut ledger =
            PostgresTaskLedger::new(runtime(port, &target, &password), &target).unwrap();
        ledger
            .execute_submission(command, authority.clone(), submission.clone())
            .unwrap();
        records.push(json!({"projectId":id,"taskRef":submission.task_ref().as_str()}));
    }
    let mut output = json!({"targetProjectId":"purge-target","survivorProjectId":"purge-survivor","records":records});
    if std::env::args().nth(1).as_deref() == Some("survivor-reference") {
        let mut registry =
            PostgresProjectRegistry::new(runtime(port, &target, &password), &target).unwrap();
        let before = registry.load().unwrap();
        let target_id = ProjectId::new("purge-target").unwrap();
        let survivor_id = ProjectId::new("purge-survivor").unwrap();
        let reference_id = ProjectId::new("purge-reference").unwrap();
        let observation = before
            .state()
            .project(&target_id)
            .unwrap()
            .observation()
            .clone();
        // Re-registering the existing survivor would collide with its own ID
        // first. A new rejected project retains a genuine cross-project command
        // without changing either accepted project or creating another ledger.
        let denied = registry
            .execute(
                RegistryCommand::register(
                    CommandId::new("fixture-duplicate-survivor").unwrap(),
                    reference_id.clone(),
                    ProjectClass::UserProject,
                    observation,
                ),
                authority.clone(),
            )
            .unwrap();
        assert!(matches!(
            denied.semantic_receipt().outcome(),
            RegistryCommandOutcome::Denied(RegistryDenial::DuplicateIdentity {
                existing_project_id,
                ..
            }) if existing_project_id == target_id
        ));
        let after = registry.load().unwrap();
        for project_id in [&target_id, &survivor_id] {
            assert_eq!(
                before.state().project(project_id),
                after.state().project(project_id),
                "denial must preserve accepted project records"
            );
        }
        assert!(after.state().project(&reference_id).is_none());
        assert_eq!(
            after.state().checkpoint().command_count(),
            before.state().checkpoint().command_count() + 1,
            "duplicate denial must be retained through the Registry API"
        );
        output["denial"] = json!(true);
        output["referenceProjectId"] = json!(reference_id.as_str());
        output["referenceCommandId"] = json!("fixture-duplicate-survivor");
        output["referencedProjectId"] = json!(target_id.as_str());
    }
    // Only this opted-in empty-DB fixture changes admission; the product purge
    // implementation never changes daemon authority or stops any service.
    admin.batch_execute("UPDATE control.runtime_admission SET admission_mode='STOPPED',daemon_instance_id=NULL,daemon_epoch=NULL,authority_revision=0,observation_digest=NULL,authority_head_digest=NULL WHERE singleton").unwrap();
    println!("{output}");
}
fn runtime(port: u16, target: &MigrationTarget, password: &str) -> postgres::Client {
    Config::new()
        .host("127.0.0.1")
        .port(port)
        .dbname(target.database_name())
        .user("lattice_runtime_login")
        .password(password)
        .application_name("lattice-devos-task019")
        .options("-c role=lattice_runtime -c search_path=pg_catalog")
        .connect(NoTls)
        .unwrap()
}
