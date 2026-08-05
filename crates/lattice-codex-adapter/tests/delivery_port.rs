use std::path::PathBuf;
use std::time::{Duration, Instant};

use lattice_codex_adapter::{
    CodexDeliveryAdapter, CodexDeliveryAdapterConfig, CodexIdentityExpectation,
};
use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, CodexDeliveryRequest, ContentDigest, DeliveryProfile,
    DeliveryRunRequest, DeliveryRuntime, DurableIntentEvidence, Invocation,
    PreparedWorkspaceEvidence, ProjectSnapshotId, RequestId, TaskId,
};
use lattice_ports::{DeliveryCodexPort, DeliveryFailureCertainty, PortErrorKind};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
}

fn request() -> CodexDeliveryRequest {
    let invocation = Invocation::new(
        CONTRACT_VERSION,
        RequestId::new("delivery-port-request").expect("request id"),
        TaskId::new("TASK-032").expect("task id"),
        AttemptId::new("attempt-1").expect("attempt id"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot id"),
        digest('a'),
    )
    .expect("invocation");
    let run = DeliveryRunRequest::new(
        invocation,
        DeliveryProfile::Task032CodexPostgres,
        digest('b'),
    )
    .expect("run request");
    let intent = DurableIntentEvidence::new(&run, digest('c')).expect("intent");
    let workspace = PreparedWorkspaceEvidence::new(
        &run,
        &intent,
        "workspace-1",
        std::env::temp_dir().to_string_lossy(),
        "1".repeat(40),
        digest('d'),
    )
    .expect("workspace");
    CodexDeliveryRequest::new(run, intent, workspace).expect("codex request")
}

#[test]
fn missing_launcher_is_a_known_codex_preflight_failure() {
    let missing_launcher = std::env::temp_dir().join(format!(
        "lattice-missing-codex-launcher-{}",
        std::process::id()
    ));
    let config = CodexDeliveryAdapterConfig::new(
        CodexIdentityExpectation::new(&missing_launcher, "codex-cli test", "1".repeat(64)),
        PathBuf::from(format!("{}-schema", missing_launcher.display())),
        PathBuf::from(format!("{}-home", missing_launcher.display())),
        "Apply the fixed TASK-032 delivery change.",
        Duration::from_secs(5),
        DeliveryRuntime::ScriptedAcceptance,
    )
    .expect("valid fixed adapter config");
    let mut adapter = CodexDeliveryAdapter::new(config);

    let error = adapter
        .run_delivery(request())
        .expect_err("a missing configured launcher must fail before spawn");

    assert_eq!(error.certainty(), DeliveryFailureCertainty::Known);
    assert_eq!(error.code(), "CODEX_LAUNCHER_NOT_FILE");
}

#[test]
fn an_expired_composition_deadline_stops_before_codex_preflight() {
    let missing_launcher = std::env::temp_dir().join(format!(
        "lattice-expired-codex-launcher-{}",
        std::process::id()
    ));
    let config = CodexDeliveryAdapterConfig::new(
        CodexIdentityExpectation::new(&missing_launcher, "codex-cli test", "1".repeat(64)),
        PathBuf::from(format!("{}-schema", missing_launcher.display())),
        PathBuf::from(format!("{}-home", missing_launcher.display())),
        "Apply the fixed TASK-032 delivery change.",
        Duration::from_secs(5),
        DeliveryRuntime::ScriptedAcceptance,
    )
    .expect("valid fixed adapter config");
    let deadline = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .expect("past deadline");
    let mut adapter = CodexDeliveryAdapter::with_deadline(config, deadline);

    let error = adapter
        .run_delivery(request())
        .expect_err("expired delivery must stop before launcher inspection");

    assert_eq!(error.kind(), PortErrorKind::Timeout);
    assert_eq!(error.certainty(), DeliveryFailureCertainty::Known);
    assert_eq!(error.code(), "CODEX_DELIVERY_DEADLINE_EXPIRED");
}
