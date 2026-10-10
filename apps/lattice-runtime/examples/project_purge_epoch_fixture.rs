//! Opt-in synthetic fixture only. Uses the shipped native audit and real Store.
use lattice_contracts::{
    ContentDigest, DaemonEpoch, GitRefIdentity, ProjectClass, ProjectId, RuntimeAdmissionMode,
    RuntimeKind, StoreAuthorityHead, StoreAuthorityRevision, StoreDaemonInstanceId,
};
use lattice_postgres_store::{
    PostgresProjectRegistry, PostgresProjectRegistryErrorKind, connect_project_purge,
};
use lattice_project_registry::{CommandId, RegistryCommand, RepositoryObservation};
use postgres::{Config, NoTls};
use serde_json::json;
use std::path::PathBuf;

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).unwrap()
}
fn main() {
    assert_eq!(
        std::env::var("LATTICE_PURGE_FIXTURE_ONLY").as_deref(),
        Ok("1")
    );
    let root = PathBuf::from(std::env::var("LATTICE_PURGE_FIXTURE_ROOT").unwrap());
    assert!(root.is_absolute() && root.join("project-purge-fixture.marker").is_file());
    let port: u16 = std::env::var("LATTICE_TASK019_PORT")
        .unwrap()
        .parse()
        .unwrap();
    assert!(![4317, 5432, 55432, 58743, 64272].contains(&port));
    lattice_runtime::initialize_registry_anchor_file_audit().unwrap();
    let run = std::env::var("LATTICE_TASK019_RUN_ID").unwrap();
    let password = std::env::var("LATTICE_TASK019_PASSWORD").unwrap();
    let mode = std::env::args().nth(1).unwrap();
    if mode == "bot-install" {
        println!(
            "{}",
            lattice_postgres_store::install_bot_lifecycle(port, &run, &password).unwrap()
        );
        return;
    }
    if mode == "bot-register" {
        for project in ["purge-target", "unrelated-alias"] {
            let owner = "11111111-2222-3333-4444-555555555555";
            let value = lattice_postgres_store::execute_bot_lifecycle(port, &run, &password, &json!({
                "action":"register","request_id":"fixture-register","project_id":project,"role_id":"fixture-role",
                "expected_revision":0,"expected_generation":0,"owner_thread_id":owner,"owner_host_id":"fixture",
                "body":{"work_ids":[],"policy_digest":"a".repeat(64),"rules_digest":"b".repeat(64),
                    "binding_receipt":{"tool":"read_thread","target_thread_id":owner,"target_host_id":"fixture",
                        "result_digest":"c".repeat(64),"readback_digest":"d".repeat(64),
                        "evidence_ref":"synthetic-fixture:bot-inventory","success":true,"readback_verified":true,"old_pending_count":0}}
            })).unwrap();
            assert_eq!(value["status"], "APPLIED");
        }
        println!(
            "{}",
            json!({"status":"REGISTERED","syntheticEvidence":true})
        );
        return;
    }
    let (mut admin, target) = connect_project_purge(port, &run, &password).unwrap();
    let client = Config::new()
        .host("127.0.0.1")
        .port(port)
        .dbname(target.database_name())
        .user("lattice_runtime_login")
        .password(password)
        .application_name("lattice-devos-task019")
        .options("-c role=lattice_runtime -c search_path=pg_catalog")
        .connect(NoTls)
        .unwrap();
    let mut registry = PostgresProjectRegistry::new(client, &target).unwrap();
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
    let make = |id: &str, project: &str, directory: &str, byte: char| {
        RegistryCommand::register(
            CommandId::new(id).unwrap(),
            ProjectId::new(project).unwrap(),
            ProjectClass::UserProject,
            RepositoryObservation::new(
                root.join(directory).to_string_lossy().into_owned(),
                digest(byte),
                digest(byte),
                digest(byte),
                GitRefIdentity::new("refs/heads/main", digest(byte)).unwrap(),
            )
            .unwrap(),
        )
    };
    match mode.as_str() {
        "retry-survivor" => {
            let before = registry.load().unwrap().state().clone();
            let result = registry
                .execute(
                    make(
                        "fixture-register-purge-survivor",
                        "purge-survivor",
                        "project-b",
                        '1',
                    ),
                    authority,
                )
                .unwrap();
            assert!(result.is_exact_retry());
            assert_eq!(registry.load().unwrap().state(), &before);
            println!(
                "{}",
                json!({"status":"EXACT_RETRY","semanticReceipt":result.semantic_receipt().result_digest().as_str(),"persistenceReceipt":result.persistence_receipt().receipt_digest().as_str()})
            );
        }
        "redacted" => {
            for command in [
                make(
                    "fixture-register-purge-target",
                    "purge-target",
                    "project-a",
                    '2',
                ),
                make(
                    "fixture-register-purge-target",
                    "changed-project",
                    "project-b",
                    '1',
                ),
            ] {
                assert_eq!(
                    registry
                        .execute(command, authority.clone())
                        .unwrap_err()
                        .kind(),
                    PostgresProjectRegistryErrorKind::CommandRedacted
                );
            }
            println!("{}", json!({"status":"REDACTED_ID_REJECTED"}));
        }
        "new-tail" => {
            let before = registry
                .load()
                .unwrap()
                .state()
                .checkpoint()
                .command_count();
            assert_eq!(admin.execute("UPDATE control.runtime_admission SET admission_mode='ACTIVE',daemon_instance_id='task050-fresh-process',daemon_epoch=50,authority_revision=50,observation_digest=decode(repeat('a',64),'hex'),authority_head_digest=decode(repeat('b',64),'hex') WHERE singleton AND admission_mode='STOPPED'",&[]).unwrap(),1);
            let result = registry
                .execute(
                    make("fresh-target-identity", "purge-new", "project-a", '2'),
                    authority,
                )
                .unwrap();
            assert!(!result.is_exact_retry());
            let loaded = registry.load().unwrap();
            assert_eq!(loaded.state().checkpoint().command_count(), before + 1);
            assert_eq!(admin.execute("UPDATE control.runtime_admission SET admission_mode='STOPPED',daemon_instance_id=NULL,daemon_epoch=NULL,authority_revision=0,observation_digest=NULL,authority_head_digest=NULL WHERE singleton AND daemon_instance_id='task050-fresh-process'",&[]).unwrap(),1);
            println!(
                "{}",
                json!({"status":"NEW_TAIL_VERIFIED","epoch":loaded.state().epoch(),"ordinal":loaded.state().checkpoint().command_ordinal()})
            );
        }
        _ => panic!("unknown fixture operation"),
    }
}
