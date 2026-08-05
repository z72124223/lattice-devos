use std::fmt::Write as FmtWrite;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, ContentDigest, HermesResearchRequest, Invocation,
    ProjectSnapshotId, RequestId, TaskId,
};
use crate::{
    CodexProxyInvocation, HERMES_CPYTHON_ARCHIVE_BYTES, HERMES_CPYTHON_ARCHIVE_SHA256,
    HERMES_CPYTHON_BUILD_RELEASE, HERMES_CPYTHON_PROVENANCE, HERMES_CPYTHON_SHA256SUMS_SHA256,
    HERMES_CPYTHON_VERSION, HERMES_LICENSE, HERMES_PYPROJECT_SHA256, HERMES_RELEASE,
    HERMES_RUNTIME_ARCHIVE_SHA256, HERMES_SCHEMA_VERSION, HERMES_UPSTREAM_COMMIT,
    HERMES_UV_LOCK_SHA256, HermesAdapterConfig, HermesAdapterErrorKind, HermesMemoryPolicy,
    HermesOfflineRuntimeManifest, HermesProcessConfig, HermesReflectionAdapter,
    HermesReflectionJob, HermesSandboxProfile, ReflectionEvidence, ReflectionEvidenceKind,
};
use lattice_ports::HermesPort;
use sha2::{Digest, Sha256};

use crate::broker::{
    CodexAppServerFrameKind, CodexBrokerPolicy, CodexNoMarkerCanaryObservation,
    CodexNoMarkerCanaryPlan, CodexReflectionBrokerConfig, classify_codex_app_server_frame,
    verify_codex_no_marker_canary, verify_official_codex_bundle,
};
use crate::containment::{
    HermesContainmentFrameLimits, HermesWslContainmentConfig, build_hermes_bwrap_arguments,
    parse_containment_frame,
};

const SUBJECT_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const GRAPH_DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn pinned_runtime_sandbox_and_proxy_contract_are_closed_by_construction() {
    assert_eq!(HERMES_CPYTHON_VERSION, "3.12.13");
    assert_eq!(
        HERMES_CPYTHON_ARCHIVE_SHA256,
        "a140c0868258075d160fa0da51ddffd423efbc9dd350695abd33e7ce3ce94352"
    );
    assert_eq!(HERMES_CPYTHON_BUILD_RELEASE, "20260804");
    assert_eq!(HERMES_CPYTHON_PROVENANCE, "astral-sh/python-build-standalone");
    assert_eq!(HERMES_CPYTHON_ARCHIVE_BYTES, 111_375_313);
    assert_eq!(
        HERMES_CPYTHON_SHA256SUMS_SHA256,
        "eccfdcc61c9fe48b7fe61db8812925ce30f23943d16c60861001004a4ae8f55c"
    );
    assert_eq!(
        HERMES_RUNTIME_ARCHIVE_SHA256,
        "a9a84a25999a23a859a9d17ef3134ea1c3371d8bf1984313eab839e939528152"
    );
    assert_eq!(
        HERMES_PYPROJECT_SHA256,
        "64d1085ee1c23caf0ae0d9e65c73e280f466362ed43fdda1531f18f3af1d9869"
    );
    assert_eq!(
        HERMES_UV_LOCK_SHA256,
        "aab3c83f71b683507a590b6315b23bdc0abd6b63b76b2349eae15bf00dfbaf2b"
    );

    let profile = HermesSandboxProfile::official();
    assert_eq!(profile.work_directory(), "/work");
    assert_eq!(
        profile.read_only_ingress(),
        [
            "/runtime-input",
            "/config-input",
            "/request-input",
            "/broker-input",
        ]
    );
    assert_eq!(profile.writable_paths(), ["/state", "/output", "/tmp"]);
    assert!(profile.product_source_mount().is_none());
    assert!(profile.network_namespace_isolated());
    assert_eq!(profile.minimum_landlock_abi(), 3);

    assert_eq!(
        CodexProxyInvocation::parse(["--version"]).expect("fixed version probe"),
        CodexProxyInvocation::Version
    );
    assert_eq!(
        CodexProxyInvocation::parse(["app-server", "--listen", "stdio://", "--strict-config"])
            .expect("fixed app-server relay"),
        CodexProxyInvocation::AppServer
    );
    for rejected in [
        vec!["exec"],
        vec!["app-server"],
        vec!["app-server", "--listen", "ws://127.0.0.1:0", "--strict-config"],
        vec!["app-server", "--listen", "stdio://"],
        vec!["app-server", "--strict-config", "--listen", "stdio://"],
        vec!["--version", "--verbose"],
        Vec::new(),
    ] {
        let failure = CodexProxyInvocation::parse(rejected)
            .expect_err("the in-sandbox proxy has no caller-selected command surface");
        assert_eq!(failure.code(), "HERMES_CODEX_PROXY_INVOCATION_REJECTED");
    }
}

#[test]
fn offline_runtime_manifest_is_canonical_strict_and_exactly_pinned() {
    let bytes = format!(
        concat!(
            "{{\"cpython_archive_bytes\":{},",
            "\"cpython_archive_sha256\":\"{}\",",
            "\"cpython_build_release\":\"{}\",",
            "\"cpython_provenance\":\"{}\",",
            "\"cpython_sha256sums_sha256\":\"{}\",",
            "\"cpython_version\":\"3.12.13\",",
            "\"hermes_archive_sha256\":\"{}\",",
            "\"hermes_commit\":\"{}\",",
            "\"hermes_release\":\"v2026.8.3\",",
            "\"payload_byte_count\":1,\"payload_file_count\":1,",
            "\"payload_manifest_sha256\":\"{}\",",
            "\"platform\":\"x86_64-unknown-linux-gnu\",",
            "\"pyproject_sha256\":\"{}\",",
            "\"schema\":\"lattice.hermes.offline-runtime.v1\",",
            "\"uv_lock_sha256\":\"{}\"}}"
        ),
        HERMES_CPYTHON_ARCHIVE_BYTES,
        HERMES_CPYTHON_ARCHIVE_SHA256,
        HERMES_CPYTHON_BUILD_RELEASE,
        HERMES_CPYTHON_PROVENANCE,
        HERMES_CPYTHON_SHA256SUMS_SHA256,
        HERMES_RUNTIME_ARCHIVE_SHA256,
        HERMES_UPSTREAM_COMMIT,
        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        HERMES_PYPROJECT_SHA256,
        HERMES_UV_LOCK_SHA256,
    );
    let manifest = HermesOfflineRuntimeManifest::from_canonical_json(bytes.as_bytes())
        .expect("one exact offline manifest");
    assert_eq!(manifest.payload_file_count(), 1);
    assert_eq!(manifest.payload_byte_count(), 1);
    assert_eq!(
        manifest.payload_manifest_sha256(),
        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
    );

    let unknown = bytes.replacen(
        '{',
        "{\"future_install_mode\":\"network\",",
        1,
    );
    let failure = HermesOfflineRuntimeManifest::from_canonical_json(unknown.as_bytes())
        .expect_err("unknown install surfaces fail closed");
    assert_eq!(failure.code(), "HERMES_RUNTIME_MANIFEST_UNKNOWN_FIELD");

    let drift = bytes.replace(HERMES_CPYTHON_VERSION, "3.12.14");
    let failure = HermesOfflineRuntimeManifest::from_canonical_json(drift.as_bytes())
        .expect_err("runtime version drift fails closed");
    assert_eq!(failure.code(), "HERMES_RUNTIME_MANIFEST_IDENTITY_MISMATCH");
}

#[test]
#[allow(clippy::too_many_lines)]
fn codex_broker_pins_four_files_and_locks_the_proven_empty_tool_policy() {
    let policy = CodexBrokerPolicy::official();
    assert_eq!(policy.codex_version(), "codex-cli 0.146.0");
    assert_eq!(
        policy.launcher_sha256(),
        "bc343ba420dc2e2e9f59e6fc5e5bf0aae1cd8c771fc319665241fc9c0271fddb"
    );
    assert_eq!(
        policy.sandbox_setup_sha256(),
        "c12d225b34e7f82cdab6bbc714797abed661f40e158104694953889750121cef"
    );
    assert_eq!(
        policy.command_runner_sha256(),
        "0102fa1820ecd03bb03a991fd2303a1a484118f7da8a71864f88ec94bca61d6d"
    );
    assert_eq!(
        policy.package_manifest_sha256(),
        "aaa0646d6b615da94187b51efd50c69621a00867761161ae55cc16cfd545bec7"
    );
    let lock = policy.config_lock_toml();
    for required in [
        "approval_policy = \"never\"",
        "sandbox_mode = \"read-only\"",
        "web_search = \"disabled\"",
        "mcp_servers = {}",
        "include_apps_instructions = false",
        "include_collaboration_mode_instructions = false",
        "external_agent_memory_import = false",
        "artifact = false",
        "plugin_sharing = false",
        "enabled = false",
        "shell_tool = false",
        "unified_exec = false",
        "shell_snapshot = false",
        "apps = false",
        "plugins = false",
        "multi_agent = false",
        "hooks = false",
        "memories = false",
        "code_mode = false",
        "image_generation = false",
        "browser_use = false",
        "computer_use = false",
        "goals = false",
        "workspace_dependencies = false",
        "auth_elicitation = false",
        "request_permissions_tool = false",
        "deferred_executor = false",
        "token_budget = false",
    ] {
        assert!(lock.contains(required), "missing no-tools lock: {required}");
    }
    assert!(!lock.contains("auth.json"));
    let environment = policy.required_child_environment();
    assert_eq!(environment.len(), 2);
    assert_eq!(environment["CODEX_EXEC_SERVER_URL"], "none");
    assert_eq!(
        environment["CODEX_INTERNAL_APP_SERVER_REMOTE_CONTROL_DISABLED"],
        "1"
    );

    let response = br#"{"id":1,"result":{}}"#;
    assert_eq!(
        classify_codex_app_server_frame(response).expect("pending response shape"),
        CodexAppServerFrameKind::Response { id: 1 }
    );
    for (id, method) in [
        (9, "account/chatgptAuthTokens/refresh"),
        (10, "attestation/generate"),
        (11, "applyPatchApproval"),
        (12, "execCommandApproval"),
        (13, "item/commandExecution/requestApproval"),
        (14, "item/fileChange/requestApproval"),
        (15, "item/permissions/requestApproval"),
        (16, "item/tool/call"),
        (17, "item/tool/requestUserInput"),
        (18, "mcpServer/elicitation/request"),
    ] {
        let frame = serde_json::to_vec(&serde_json::json!({
            "id": id,
            "method": method,
            "params": {}
        }))
        .expect("server request fixture");
        assert_eq!(
            classify_codex_app_server_frame(&frame).expect("typed deny request"),
            CodexAppServerFrameKind::ServerRequest {
                id,
                method: method.to_owned(),
            }
        );
    }
    for fatal in [
        br#"{"method":"item/started","params":{"item":{"type":"commandExecution"}}}"#
            .as_slice(),
        br#"{"method":"item/completed","params":{"item":{"type":"hookPrompt"}}}"#
            .as_slice(),
        br#"{"method":"turn/completed","params":{"turn":{"id":"turn-1","items":[{"type":"mcpToolCall"}],"status":"completed"}}}"#
            .as_slice(),
        br#"{"method":"thread/environment/connected","params":{}}"#.as_slice(),
        br#"{"method":"hook/started","params":{}}"#.as_slice(),
        br#"{"method":"hook/completed","params":{}}"#.as_slice(),
        br#"{"method":"item/commandExecution/outputDelta","params":{}}"#.as_slice(),
        br#"{"method":"item/fileChange/outputDelta","params":{}}"#.as_slice(),
        br#"{"method":"item/fileChange/patchUpdated","params":{}}"#.as_slice(),
        br#"{"method":"turn/plan/updated","params":{}}"#.as_slice(),
        br#"{"method":"future/notification","params":{}}"#.as_slice(),
    ] {
        let failure = classify_codex_app_server_frame(fatal)
            .expect_err("unknown, approval, plan, and tool frames terminate the broker");
        assert_eq!(failure.code(), "HERMES_CODEX_BROKER_FATAL_FRAME");
    }

    policy
        .verify_model_visible_tools(std::iter::empty::<&str>())
        .expect("the admitted 0.146 policy has no model-visible tools");
    let failure = policy
        .verify_model_visible_tools(["update_plan"])
        .expect_err("any observed tool fails the no-tools canary");
    assert_eq!(failure.code(), "HERMES_CODEX_TOOLSET_NOT_EMPTY");

    let wrong_bundle = std::env::current_exe().expect("test binary");
    let failure = verify_official_codex_bundle(&wrong_bundle)
        .expect_err("one binary can never substitute for the four-file bundle");
    assert_eq!(failure.code(), "HERMES_CODEX_BUNDLE_IDENTITY_REJECTED");
}

#[test]
fn codex_no_marker_plan_and_receipt_require_joint_empty_tool_evidence() {
    let cwd = std::env::temp_dir().join("lattice-hermes-empty-cwd-contract");
    let nonce = "abababababababababababababababababababababababababababababababab";
    let plan = CodexNoMarkerCanaryPlan::new(cwd.clone(), nonce, "gpt-5.6-sol")
        .expect("fixed canary plan");
    let initialize = plan.initialize_request();
    for capability in [
        "experimentalApi",
        "requestAttestation",
        "mcpServerOpenaiFormElicitation",
    ] {
        assert_eq!(initialize["params"]["capabilities"][capability], false);
    }
    let thread = plan.thread_start_request();
    assert_eq!(thread["params"]["approvalPolicy"], "never");
    assert_eq!(thread["params"]["sandbox"], "read-only");
    assert_eq!(thread["params"]["ephemeral"], true);
    assert!(thread["params"].get("environments").is_none());
    assert!(thread["params"].get("runtimeWorkspaceRoots").is_none());
    assert!(thread["params"].get("dynamicTools").is_none());
    assert!(thread["params"].get("selectedCapabilityRoots").is_none());
    let turn = plan.turn_start_request("thread-canary");
    assert_eq!(turn["id"], 2);
    assert_eq!(turn["params"]["sandboxPolicy"]["type"], "readOnly");
    assert_eq!(turn["params"]["sandboxPolicy"]["networkAccess"], false);
    assert!(turn["params"].get("environments").is_none());
    assert!(turn["params"].get("runtimeWorkspaceRoots").is_none());
    assert_eq!(
        turn["params"]["outputSchema"]["properties"]["nonce"]["const"],
        nonce
    );
    assert_eq!(
        turn["params"]["outputSchema"]["properties"]["markerCreated"]["const"],
        false
    );

    let tree = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
    let transcript =
        "dededededededededededededededededededededededededededededededede";
    let output = format!(r#"{{"markerCreated":false,"nonce":"{nonce}"}}"#);
    let observation = CodexNoMarkerCanaryObservation::new(
        nonce,
        tree,
        0,
        tree,
        0,
        false,
        false,
        "completed",
        1,
        0,
        0,
        output.as_bytes(),
        transcript,
        true,
    )
    .expect("typed observation");
    let receipt = verify_codex_no_marker_canary(&observation).expect("joint canary evidence");
    assert_eq!(receipt.receipt_digest().as_str().len(), 64);
    assert_eq!(receipt.transcript_sha256(), transcript);

    let marker_observed = CodexNoMarkerCanaryObservation::new(
        nonce,
        tree,
        0,
        tree,
        0,
        false,
        true,
        "completed",
        1,
        0,
        0,
        output.as_bytes(),
        transcript,
        true,
    )
    .expect("typed observation");
    let failure = verify_codex_no_marker_canary(&marker_observed)
        .expect_err("marker appearance fails even if the model claims false");
    assert_eq!(failure.code(), "HERMES_CODEX_NO_MARKER_CANARY_REJECTED");
}

#[test]
#[ignore = "requires the staged official @openai/codex 0.146.0 four-file bundle"]
fn official_codex_0146_four_file_bundle_identity_is_live_verified() {
    let launcher = std::env::var_os("LATTICE_HERMES_CODEX_0146_LAUNCHER")
        .expect("set exact staged launcher path");
    let reviewed = verify_official_codex_bundle(std::path::Path::new(&launcher))
        .expect("exact four-file official bundle");
    assert_eq!(reviewed.version(), "codex-cli 0.146.0");
    assert_eq!(reviewed.launcher_sha256(), CodexBrokerPolicy::official().launcher_sha256());
    assert_eq!(
        reviewed.package_manifest_sha256(),
        CodexBrokerPolicy::official().package_manifest_sha256()
    );
}

#[cfg(windows)]
#[test]
#[ignore = "requires the built broker helper, official Codex 0.146.0 bundle, and daily subscription login"]
fn official_codex_0146_no_marker_broker_canary_is_live_verified() {
    let launcher = std::path::PathBuf::from(
        std::env::var_os("LATTICE_HERMES_CODEX_0146_LAUNCHER")
            .expect("set exact staged launcher path"),
    );
    let helper = std::path::PathBuf::from(
        std::env::var_os("LATTICE_HERMES_CODEX_BROKER_HELPER")
            .expect("set built broker helper path"),
    );
    let codex_home = std::path::PathBuf::from(
        std::env::var_os("LATTICE_HERMES_DAILY_CODEX_HOME")
            .expect("set daily logged-in CODEX_HOME"),
    );
    let product_root = std::fs::canonicalize(env!("CARGO_MANIFEST_DIR"))
        .expect("crate product root");
    let sequence = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("monotonic wall clock seed")
        .as_nanos();
    let isolation_root = std::env::temp_dir().join(format!(
        "lattice-hermes-codex-broker-live-{}-{sequence}",
        std::process::id()
    ));
    let helper_sha256 = crate::sha256_file(&helper).expect("broker helper digest");
    let config = CodexReflectionBrokerConfig::new(
        helper,
        helper_sha256.clone(),
        launcher,
        codex_home,
        isolation_root.clone(),
        product_root,
        "gpt-5.6-sol",
    )
    .expect("sealed broker config");
    let receipt = config
        .run_no_marker_canary(Instant::now() + Duration::from_mins(4))
        .expect("real no-marker broker canary");
    assert_eq!(receipt.helper_sha256(), helper_sha256);
    assert_eq!(
        receipt.launcher_sha256(),
        CodexBrokerPolicy::official().launcher_sha256()
    );
    for digest in [
        receipt.receipt_digest().as_str(),
        receipt.canary_receipt_digest().as_str(),
        receipt.config_lock_sha256(),
        receipt.child_environment_sha256(),
        receipt.transcript_sha256(),
    ] {
        assert_eq!(digest.len(), 64);
    }
    std::fs::remove_dir_all(&isolation_root).expect("remove exact successful canary root");
}

#[test]
fn bwrap_plan_and_private_frame_are_fixed_bounded_and_cross_bound() {
    let arguments = build_hermes_bwrap_arguments(
        "/var/tmp/lattice-runtime-targets/hermes-v2026.8.3-cpython-3.12.13-pbs-20260804",
        "/mnt/c/lattice-run/config",
        "/mnt/c/lattice-run/request.json",
    )
    .expect("fixed sandbox plan");
    let rendered = arguments
        .iter()
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>();
    for required in [
        "--die-with-parent",
        "--unshare-all",
        "--unshare-user",
        "--disable-userns",
        "--assert-userns-disabled",
        "--new-session",
        "--cap-drop",
        "--clearenv",
        "LATTICE_CODEX_BROKER_READ_FD",
        "LATTICE_CODEX_BROKER_WRITE_FD",
        "/work",
        "/runtime-input",
        "/config-input",
        "/request-input/request.json",
        "/state",
        "/output",
        "/tmp",
    ] {
        assert!(rendered.iter().any(|value| value == required));
    }
    assert!(!rendered.iter().any(|value| value.contains("hermes-reflection")));
    assert!(!rendered.iter().any(|value| value == "/source"));
    assert_eq!(
        rendered
            .iter()
            .filter(|argument| argument.as_ref() == "--ro-bind")
            .count(),
        6
    );
    assert!(!rendered.iter().any(|value| value == "/broker-input"));

    let digests = (*b"abcdef").map(|byte| vec![byte; 64]);
    let reflection = br#"{"schema_version":"lattice.hermes.reflection.v1"}"#;
    let mut frame = b"LATTICE_HERMES_CONTAINED_V1\n".to_vec();
    for field in digests
        .iter()
        .map(Vec::as_slice)
        .chain(std::iter::once(reflection.as_slice()))
    {
        frame.extend_from_slice(&(field.len() as u64).to_be_bytes());
        frame.extend_from_slice(field);
    }
    let parsed = parse_containment_frame(&frame, HermesContainmentFrameLimits::default())
        .expect("one complete strict frame");
    assert_eq!(parsed.runtime_manifest_sha256(), digests[0]);
    assert_eq!(parsed.request_sha256(), digests[2]);
    assert_eq!(parsed.reflection(), reflection);

    frame.push(0);
    let failure = parse_containment_frame(&frame, HermesContainmentFrameLimits::default())
        .expect_err("trailing bytes fail closed");
    assert_eq!(failure.code(), "HERMES_CONTAINMENT_FRAME_TRAILING_BYTES");
}

#[test]
#[ignore = "requires WSL2, bubblewrap, and the staged Linux CPython runtime"]
fn wsl_bwrap_socketpair_inherited_fd_canary_is_live_verified() {
    let wsl = std::path::PathBuf::from(r"C:\Windows\System32\wsl.exe");
    let product_root = std::fs::canonicalize(std::env::current_dir().expect("cwd"))
        .expect("canonical product root");
    let isolation_root = std::env::temp_dir().join(format!(
        "lattice-hermes-socketpair-canary-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let config = HermesWslContainmentConfig::new(
        wsl,
        "/var/tmp/lattice-runtime-targets/hermes-v2026.8.3-cpython-3.12.13-pbs-20260804",
        isolation_root,
        product_root,
    )
    .expect("exact WSL containment config");
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let receipt = config
        .run_socketpair_canary(deadline)
        .expect("socketpair survives into bwrap as fd 3 and reaps");
    assert_eq!(receipt.broker_read_fd(), 0);
    assert_eq!(receipt.broker_write_fd(), 1);
    assert_eq!(receipt.python_version(), "3.12.13");
    assert_eq!(receipt.bwrap_sha256().len(), 64);
    assert!(receipt.descendants_reaped());
    assert_eq!(receipt.receipt_digest().as_str().len(), 64);
}

#[test]
fn capabilities_runs_events_and_status_produce_bound_canonical_reflection() {
    let request = request();
    let reflection_job = job(request.clone());
    let output = bound_output(&reflection_job);
    let server = ScriptedServer::start(vec![
        capabilities(),
        Response::json(202, r#"{"run_id":"run_abc123","status":"started"}"#),
        completed_events("run_abc123", &output),
        completed_status("run_abc123", "lattice-task-034-session", &output),
    ]);

    let mut adapter =
        HermesReflectionAdapter::connect(config(&server), reflection_job).expect("adapter");
    let reflection = adapter
        .run_reflection(&request)
        .expect("strict reflection accepted");

    assert_eq!(HERMES_RELEASE, "v2026.8.3");
    assert_eq!(
        HERMES_UPSTREAM_COMMIT,
        "3c27eb6234bf91b8ceee9e9071591b31e9b148cb"
    );
    assert_eq!(HERMES_LICENSE, "MIT");
    assert_eq!(
        reflection.summary(),
        "The adapter boundary is read-only and fail-closed."
    );
    assert_eq!(reflection.findings().len(), 1);
    assert_eq!(reflection.next_actions().len(), 1);
    assert_eq!(reflection.binding().request_id(), "request-hermes-1");
    assert_eq!(reflection.canonical_bytes()[0], b'{');
    assert_eq!(reflection.output_digest().as_str().len(), 64);
    println!(
        "SCRIPTED_CONTRACT_CANONICAL_REFLECTION_SHA256={}",
        reflection.output_digest().as_str()
    );
    println!(
        "SCRIPTED_CONTRACT_CANONICAL_REFLECTION={}",
        std::str::from_utf8(reflection.canonical_bytes()).expect("canonical UTF-8")
    );

    let requests = server.finish();
    assert_eq!(requests.len(), 4);
    assert!(requests[0].starts_with("GET /v1/capabilities HTTP/1.1\r\n"));
    assert!(requests[1].starts_with("POST /v1/runs HTTP/1.1\r\n"));
    assert!(requests[1].contains("Authorization: Bearer test-only-loopback-key\r\n"));
    assert!(requests[1].contains(r#""session_id":"lattice-task-034-session""#));
    assert!(requests[1].contains(r#""model":"hermes-agent""#));
    assert!(requests[1].contains("Do not call tools"));
    assert!(requests[1].contains("Hermes session memory as unavailable"));
    assert!(requests[1].contains("do not read or write PostgreSQL"));
    assert!(requests[1].contains("Label every finding as inference"));
    assert!(requests[2].starts_with("GET /v1/runs/run_abc123/events HTTP/1.1\r\n"));
    assert!(requests[3].starts_with("GET /v1/runs/run_abc123 HTTP/1.1\r\n"));
}

#[test]
fn event_transport_loss_recovers_through_bound_status_polling() {
    let request = request();
    let job = job(request.clone());
    let output = bound_output(&job);
    let server = ScriptedServer::start(vec![
        capabilities(),
        Response::json(202, r#"{"run_id":"run_recoverable","status":"started"}"#),
        Response::json(503, r#"{"error":{"message":"event buffer expired"}}"#),
        completed_status("run_recoverable", "lattice-task-034-session", &output),
    ]);
    let mut adapter = HermesReflectionAdapter::connect(config(&server), job).expect("adapter");

    let reflection = adapter
        .run_reflection(&request)
        .expect("polling recovers an unavailable event stream");

    assert_eq!(
        reflection.summary(),
        "The adapter boundary is read-only and fail-closed."
    );
    assert_eq!(server.finish().len(), 4);
}

#[test]
fn same_process_reconciliation_uses_receipt_without_resubmission() {
    let request = request();
    let job = job(request.clone());
    let output = bound_output(&job);
    let server = ScriptedServer::start(vec![
        capabilities(),
        Response::json(202, r#"{"run_id":"run_reconcile","status":"started"}"#),
        Response::json(503, r#"{"error":{"message":"events unavailable"}}"#),
        completed_status("run_reconcile", "lattice-task-034-session", &output)
            .with_delay(Duration::from_millis(200)),
        capabilities(),
        completed_status("run_reconcile", "lattice-task-034-session", &output),
    ]);
    let timeout_config = contained_config_with_timing(
        &server,
        Duration::from_millis(100),
        Duration::from_millis(1),
    );
    let mut adapter = HermesReflectionAdapter::connect(timeout_config, job).expect("adapter");

    let failure = adapter
        .run_reflection(&request)
        .expect_err("ambiguous post-submit status must return a receipt");
    let receipt = failure
        .recovery_receipt()
        .expect("typed recovery receipt")
        .clone();
    assert_eq!(receipt.run_id(), Some("run_reconcile"));
    let duplicate = adapter
        .run_reflection(&request)
        .expect_err("same job cannot be silently resubmitted");
    assert_eq!(duplicate.code(), "HERMES_RUN_RECONCILIATION_REQUIRED");

    thread::sleep(Duration::from_millis(250));
    let reflection = adapter
        .reconcile_reflection(&request, &receipt)
        .expect("same-process status reconciliation");

    assert_eq!(reflection.binding().input_digest().len(), 64);
    let requests = server.finish();
    assert_eq!(requests.len(), 6);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.starts_with("POST /v1/runs HTTP/1.1"))
            .count(),
        1
    );
    assert!(requests[0].starts_with("GET /v1/capabilities"));
    assert!(requests[5].starts_with("GET /v1/runs/run_reconcile HTTP/1.1"));
}

#[test]
fn malformed_reflection_schema_fails_closed() {
    let request = request();
    let job = job(request.clone());
    let mut malformed_output: serde_json::Value =
        serde_json::from_str(&bound_output(&job)).expect("reflection fixture");
    malformed_output
        .as_object_mut()
        .expect("reflection object")
        .insert("unexpected".to_owned(), serde_json::json!(true));
    let malformed_output = malformed_output.to_string();
    let server = ScriptedServer::start(vec![
        capabilities(),
        Response::json(202, r#"{"run_id":"run_malformed","status":"started"}"#),
        completed_events("run_malformed", &malformed_output),
        completed_status(
            "run_malformed",
            "lattice-task-034-session",
            &malformed_output,
        ),
    ]);
    let mut adapter = HermesReflectionAdapter::connect(config(&server), job).expect("adapter");

    let failure = adapter
        .run_reflection(&request)
        .expect_err("unknown schema fields must fail closed");

    assert_eq!(failure.kind(), HermesAdapterErrorKind::Malformed);
    assert_eq!(failure.code(), "HERMES_REFLECTION_SCHEMA_REJECTED");
    assert_eq!(server.finish().len(), 4);
}

#[test]
fn evidence_and_reflection_text_reject_sensitive_values_but_accept_digest_only_binding() {
    let secret = "Authorization: Bearer sk-example-secret-value-123456";
    let sensitive_digest =
        digest("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");
    let digest_only = ReflectionEvidence::new_digest_only(
        ReflectionEvidenceKind::Test,
        digest(GRAPH_DIGEST),
        vec![sensitive_digest.clone()],
    )
    .expect("typed digest-only evidence");
    let digest_only_job = HermesReflectionJob::new(
        request(),
        "lattice-task-034-session",
        "hermes-agent",
        vec![digest_only],
    )
    .expect("digest-only job");
    assert!(digest_only_job.prompt().contains(sensitive_digest.as_str()));
    assert!(!digest_only_job.prompt().contains(secret));

    let request = request();
    let job = job(request.clone());
    let mut unsafe_output: serde_json::Value =
        serde_json::from_str(&bound_output(&job)).expect("reflection fixture");
    unsafe_output["summary"] = serde_json::json!(secret);
    let unsafe_output = unsafe_output.to_string();
    let server = ScriptedServer::start(vec![
        capabilities(),
        Response::json(202, r#"{"run_id":"run_secret","status":"started"}"#),
        completed_events("run_secret", &unsafe_output),
        completed_status("run_secret", "lattice-task-034-session", &unsafe_output),
    ]);
    let mut adapter = HermesReflectionAdapter::connect(config(&server), job).expect("adapter");
    let failure = adapter
        .run_reflection(&request)
        .expect_err("secret-bearing reflection output must fail closed");
    assert_eq!(failure.kind(), HermesAdapterErrorKind::Malformed);
    assert_eq!(server.finish().len(), 4);
}

#[test]
fn status_session_cross_binding_fails_closed() {
    let request = request();
    let job = job(request.clone());
    let output = bound_output(&job);
    let server = ScriptedServer::start(vec![
        capabilities(),
        Response::json(202, r#"{"run_id":"run_cross","status":"started"}"#),
        completed_events("run_cross", &output),
        completed_status("run_cross", "foreign-session", &output),
    ]);
    let mut adapter = HermesReflectionAdapter::connect(config(&server), job).expect("adapter");

    let failure = adapter
        .run_reflection(&request)
        .expect_err("foreign session must fail closed");

    assert_eq!(failure.kind(), HermesAdapterErrorKind::CrossBinding);
    assert_eq!(failure.code(), "HERMES_STATUS_SESSION_BINDING_REJECTED");
    assert_eq!(server.finish().len(), 4);
}

#[test]
fn event_and_status_timeouts_fail_closed() {
    let request = request();
    let job = job(request.clone());
    let server = ScriptedServer::start(vec![
        capabilities().with_delay(Duration::from_millis(20)),
        Response::json(202, r#"{"run_id":"run_timeout","status":"started"}"#)
            .with_delay(Duration::from_millis(20)),
        Response::json(503, r#"{"error":{"message":"events unavailable"}}"#),
        Response::json(200, r#"{"object":"hermes.run"}"#).with_delay(Duration::from_millis(120)),
    ]);
    let timeout_config = contained_config_with_timing(
        &server,
        Duration::from_millis(100),
        Duration::from_millis(1),
    );
    let mut adapter = HermesReflectionAdapter::connect(timeout_config, job).expect("adapter");

    let failure = adapter
        .run_reflection(&request)
        .expect_err("timeout must fail closed");

    assert_eq!(failure.kind(), HermesAdapterErrorKind::Timeout);
    assert_eq!(
        failure
            .recovery_receipt()
            .and_then(|receipt| receipt.run_id()),
        Some("run_timeout")
    );
    assert_eq!(server.finish().len(), 4);
}

#[test]
fn server_side_execution_or_missing_read_only_capabilities_fail_closed() {
    let mut unsafe_cases = Vec::new();

    for feature in ["tools", "file_access", "shell", "memory_write_api"] {
        let mut enabled = capabilities_value();
        enabled["features"][feature] = serde_json::json!(true);
        unsafe_cases.push(enabled);
    }

    for unsafe_capabilities in unsafe_cases {
        let request = request();
        let server =
            ScriptedServer::start(vec![Response::json(200, unsafe_capabilities.to_string())]);
        let mut adapter = HermesReflectionAdapter::connect(config(&server), job(request.clone()))
            .expect("adapter");

        let failure = adapter
            .run_reflection(&request)
            .expect_err("unsafe or incomplete capability contract must fail closed");

        assert!(matches!(
            failure.kind(),
            HermesAdapterErrorKind::CapabilityMismatch | HermesAdapterErrorKind::Malformed
        ));
        assert_eq!(server.finish().len(), 1);
    }
}

#[test]
fn uncontained_io_entrypoints_fail_before_network() {
    let request = request();
    let adapter_job = job(request.clone());
    let recovery = crate::recovery_receipt(&adapter_job, Some("run_uncontained".to_owned()));
    let server = ScriptedServer::start(Vec::new());
    let mut adapter =
        HermesReflectionAdapter::connect(uncontained_config(&server), adapter_job).expect("adapter");

    let failure = adapter
        .run_reflection(&request)
        .expect_err("uncontained run must fail before network I/O");
    assert_eq!(failure.code(), "HERMES_LIVE_RUNTIME_RECEIPT_REQUIRED");

    adapter.active_run = Some(recovery.clone());
    let failure = adapter
        .reconcile_reflection(&request, &recovery)
        .expect_err("uncontained reconciliation must fail before network I/O");
    assert_eq!(failure.code(), "HERMES_LIVE_RUNTIME_RECEIPT_REQUIRED");

    let failure = HermesPort::interrupt(&mut adapter, request.invocation().request_id())
        .expect_err("uncontained interrupt must fail before network I/O");
    assert_eq!(failure.code(), "HERMES_LIVE_RUNTIME_RECEIPT_REQUIRED");

    assert!(server.finish().is_empty());
}

#[test]
fn containment_binding_mismatch_fails_before_network() {
    let request = request();
    let adapter_job = job(request.clone());
    let recovery = crate::recovery_receipt(&adapter_job, Some("run_cross_bound".to_owned()));
    let server = ScriptedServer::start(Vec::new());
    let mut mismatched = config(&server);
    mismatched
        .containment_receipt
        .as_mut()
        .expect("test-only containment receipt")
        .api_key_sha256 = crate::sha256_text("foreign-loopback-key");
    let mut adapter = HermesReflectionAdapter::connect(mismatched, adapter_job).expect("adapter");

    let failure = adapter
        .run_reflection(&request)
        .expect_err("cross-bound run must fail before network I/O");
    assert_eq!(
        failure.code(),
        "HERMES_CONTAINMENT_ENDPOINT_BINDING_REJECTED"
    );

    adapter.active_run = Some(recovery.clone());
    let failure = adapter
        .reconcile_reflection(&request, &recovery)
        .expect_err("cross-bound reconciliation must fail before network I/O");
    assert_eq!(
        failure.code(),
        "HERMES_CONTAINMENT_ENDPOINT_BINDING_REJECTED"
    );

    let failure = HermesPort::interrupt(&mut adapter, request.invocation().request_id())
        .expect_err("cross-bound interrupt must fail before network I/O");
    assert_eq!(
        failure.code(),
        "HERMES_CONTAINMENT_ENDPOINT_BINDING_REJECTED"
    );

    assert!(server.finish().is_empty());
}

#[test]
fn control_envelopes_reject_unknown_fields_and_event_discriminator_drift() {
    let request = request();
    let mut capability = capabilities_value();
    capability["unexpected"] = serde_json::json!(true);
    let capability_server =
        ScriptedServer::start(vec![Response::json(200, capability.to_string())]);
    let mut capability_adapter =
        HermesReflectionAdapter::connect(config(&capability_server), job(request.clone()))
            .expect("adapter");
    let capability_failure = capability_adapter
        .run_reflection(&request)
        .expect_err("capability unknown field must fail closed");
    assert_eq!(
        capability_failure.code(),
        "HERMES_CAPABILITIES_UNKNOWN_FIELD"
    );
    assert_eq!(capability_server.finish().len(), 1);

    let event_job = job(request.clone());
    let drift_event = serde_json::json!({
        "event": "run.future",
        "run_id": "run_event_drift",
        "timestamp": 1.0
    });
    let event_server = ScriptedServer::start(vec![
        capabilities(),
        Response::json(202, r#"{"run_id":"run_event_drift","status":"started"}"#),
        Response::sse(200, format!("data: {drift_event}\n\n")),
    ]);
    let mut event_adapter =
        HermesReflectionAdapter::connect(config(&event_server), event_job).expect("adapter");
    let event_failure = event_adapter
        .run_reflection(&request)
        .expect_err("unknown SSE discriminator must fail closed");
    assert_eq!(event_failure.code(), "HERMES_EVENT_DISCRIMINATOR_REJECTED");
    assert!(event_failure.recovery_receipt().is_some());
    assert_eq!(event_server.finish().len(), 3);

    let status_job = job(request.clone());
    let output = bound_output(&status_job);
    let status_server = ScriptedServer::start(vec![
        capabilities(),
        Response::json(202, r#"{"run_id":"run_status_extra","status":"started"}"#),
        completed_events("run_status_extra", &output),
        Response::json(
            200,
            serde_json::json!({
                "object": "hermes.run",
                "run_id": "run_status_extra",
                "status": "completed",
                "session_id": "lattice-task-034-session",
                "model": "hermes-agent",
                "output": output,
                "unexpected": true
            })
            .to_string(),
        ),
    ]);
    let mut status_adapter =
        HermesReflectionAdapter::connect(config(&status_server), status_job).expect("adapter");
    let status_failure = status_adapter
        .run_reflection(&request)
        .expect_err("status unknown field must fail closed");
    assert_eq!(status_failure.code(), "HERMES_STATUS_UNKNOWN_FIELD");
    assert_eq!(status_server.finish().len(), 4);
}

#[test]
fn scripted_adapter_cannot_emit_live_port_evidence() {
    let request = request();
    let server = ScriptedServer::start(Vec::new());
    let mut adapter = HermesReflectionAdapter::connect(
        uncontained_config(&server),
        job(request.clone()),
    )
    .expect("adapter");

    let failure = HermesPort::research(&mut adapter, request)
        .expect_err("no exact binary plus OS containment receipt means no Live evidence");

    assert_eq!(failure.code(), "HERMES_LIVE_RUNTIME_RECEIPT_REQUIRED");
    assert!(server.finish().is_empty());
}

#[test]
fn process_command_uses_only_explicit_isolated_homes_and_loopback_api() {
    let executable = std::env::current_exe().expect("test executable");
    let executable_hash = sha256_file(&executable);
    let scratch = std::env::temp_dir().join(format!(
        "lattice-hermes-process-contract-{}",
        std::process::id()
    ));
    let product_root = std::fs::canonicalize(std::env::current_dir().expect("current directory"))
        .expect("canonical product root");
    let process = HermesProcessConfig::new(
        executable.clone(),
        executable_hash.clone(),
        scratch.clone(),
        product_root.clone(),
        "127.0.0.1:8642".parse().expect("loopback"),
        "process-local-api-key",
        "hermes-agent",
        Duration::from_secs(2),
    )
    .expect("isolated process config");

    assert_eq!(
        process.memory_policy(),
        HermesMemoryPolicy::EphemeralIsolatedHome
    );
    assert_gateway_command(&process, &executable);
    assert_probe_command(&process);
    assert_eq!(process.executable_sha256(), executable_hash);
    assert_process_rejections(&executable, &executable_hash, &product_root);
}

fn assert_gateway_command(process: &HermesProcessConfig, executable: &std::path::Path) {
    let command = process.gateway_command().expect("gateway command");
    assert_eq!(
        command.get_program(),
        std::fs::canonicalize(executable)
            .expect("canonical executable")
            .as_os_str()
    );
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        vec![std::ffi::OsStr::new("gateway")]
    );
    assert_eq!(command.get_current_dir(), Some(process.working_directory()));
    let environment = command
        .get_envs()
        .filter_map(|(key, value)| value.map(|value| (key.to_owned(), value.to_owned())))
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(
        environment
            .get(std::ffi::OsStr::new("HERMES_HOME"))
            .map(std::ffi::OsString::as_os_str),
        Some(process.hermes_home().as_os_str())
    );
    assert_eq!(
        environment
            .get(std::ffi::OsStr::new("CODEX_HOME"))
            .map(std::ffi::OsString::as_os_str),
        Some(process.codex_home().as_os_str())
    );
    assert_eq!(
        environment.get(std::ffi::OsStr::new("API_SERVER_HOST")),
        Some(&std::ffi::OsString::from("127.0.0.1"))
    );
    assert_eq!(
        environment.get(std::ffi::OsStr::new("API_SERVER_PORT")),
        Some(&std::ffi::OsString::from("8642"))
    );
    assert_eq!(
        environment.get(std::ffi::OsStr::new("API_SERVER_KEY")),
        Some(&std::ffi::OsString::from("process-local-api-key"))
    );
    assert!(!environment.contains_key(std::ffi::OsStr::new("OPENAI_API_KEY")));
    assert!(!environment.contains_key(std::ffi::OsStr::new("DATABASE_URL")));
    assert!(!environment.contains_key(std::ffi::OsStr::new("GIT_ASKPASS")));
    assert_eq!(
        environment
            .get(std::ffi::OsStr::new("HOME"))
            .map(std::ffi::OsString::as_os_str),
        Some(process.hermes_home().as_os_str())
    );
    assert_eq!(
        environment
            .get(std::ffi::OsStr::new("USERPROFILE"))
            .map(std::ffi::OsString::as_os_str),
        Some(process.hermes_home().as_os_str())
    );
    assert_eq!(
        environment
            .get(std::ffi::OsStr::new("TEMP"))
            .map(std::ffi::OsString::as_os_str),
        Some(process.temp_directory().as_os_str())
    );
    assert!(!environment.contains_key(std::ffi::OsStr::new("PATH")));
}

fn assert_probe_command(process: &HermesProcessConfig) {
    let probe = process
        .version_probe_command()
        .expect("bounded identity probe command");
    assert_eq!(
        probe.get_args().collect::<Vec<_>>(),
        vec![std::ffi::OsStr::new("--version")]
    );
    let probe_environment = probe
        .get_envs()
        .filter_map(|(key, value)| value.map(|value| (key.to_owned(), value.to_owned())))
        .collect::<std::collections::HashMap<_, _>>();
    for secret_or_server_setting in [
        "API_SERVER_KEY",
        "API_SERVER_ENABLED",
        "API_SERVER_HOST",
        "API_SERVER_PORT",
        "API_SERVER_MODEL_NAME",
    ] {
        assert!(!probe_environment.contains_key(std::ffi::OsStr::new(secret_or_server_setting)));
    }
}

fn assert_process_rejections(
    executable: &std::path::Path,
    executable_hash: &str,
    product_root: &std::path::Path,
) {
    let Err(hash_failure) = HermesProcessConfig::new(
        executable.to_path_buf(),
        "0".repeat(64),
        std::env::temp_dir().join(format!("lattice-hermes-hash-reject-{}", std::process::id())),
        product_root.to_path_buf(),
        "127.0.0.1:8642".parse().expect("loopback"),
        "process-local-api-key",
        "hermes-agent",
        Duration::from_secs(2),
    ) else {
        panic!("an executable outside the pinned hash must fail closed");
    };
    assert_eq!(hash_failure.kind(), HermesAdapterErrorKind::Identity);
    assert_eq!(hash_failure.code(), "HERMES_EXECUTABLE_HASH_MISMATCH");

    let Err(existing_home_failure) = HermesProcessConfig::new(
        executable.to_path_buf(),
        executable_hash.to_owned(),
        std::env::temp_dir(),
        product_root.to_path_buf(),
        "127.0.0.1:8642".parse().expect("loopback"),
        "process-local-api-key",
        "hermes-agent",
        Duration::from_secs(2),
    ) else {
        panic!("an existing home cannot be reused as ephemeral state");
    };
    assert_eq!(
        existing_home_failure.kind(),
        HermesAdapterErrorKind::Configuration
    );

    let Err(overlap) = HermesProcessConfig::new(
        executable.to_path_buf(),
        executable_hash.to_owned(),
        product_root.join("forbidden-hermes-run"),
        product_root.to_path_buf(),
        "127.0.0.1:8642".parse().expect("loopback"),
        "process-local-api-key",
        "hermes-agent",
        Duration::from_secs(2),
    ) else {
        panic!("product descendant cannot be an isolation root");
    };
    assert_eq!(overlap.code(), "HERMES_PRODUCT_ROOT_OVERLAP_REJECTED");
}

fn sha256_file(path: &std::path::Path) -> String {
    let bytes = std::fs::read(path).expect("read executable bytes");
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("write digest to string");
    }
    encoded
}

fn config(server: &ScriptedServer) -> HermesAdapterConfig {
    contained_config_with_timing(
        server,
        Duration::from_secs(2),
        Duration::from_millis(1),
    )
}

fn contained_config_with_timing(
    server: &ScriptedServer,
    timeout: Duration,
    poll_interval: Duration,
) -> HermesAdapterConfig {
    let mut config = HermesAdapterConfig::new(
        server.address(),
        "test-only-loopback-key",
        timeout,
        poll_interval,
    )
    .expect("loopback config");
    config.containment_receipt = Some(crate::HermesContainmentReceipt {
        endpoint: server.address(),
        api_key_sha256: crate::sha256_text("test-only-loopback-key"),
        receipt_digest: digest(
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        ),
    });
    config
}

fn uncontained_config(server: &ScriptedServer) -> HermesAdapterConfig {
    HermesAdapterConfig::new(
        server.address(),
        "test-only-loopback-key",
        Duration::from_secs(2),
        Duration::from_millis(1),
    )
    .expect("loopback config")
}

fn job(request: HermesResearchRequest) -> HermesReflectionJob {
    HermesReflectionJob::new(
        request,
        "lattice-task-034-session",
        "hermes-agent",
        vec![
            ReflectionEvidence::new(ReflectionEvidenceKind::Graphify, digest(GRAPH_DIGEST))
                .expect("bounded evidence"),
        ],
    )
    .expect("valid bound job")
}

fn bound_output(job: &HermesReflectionJob) -> String {
    format!(
        r#"{{"schema_version":"{HERMES_SCHEMA_VERSION}","binding":{{"request_id":"request-hermes-1","task_id":"task-034","attempt_id":"attempt-1","project_snapshot_id":"snapshot-79096b6","subject_digest":"{SUBJECT_DIGEST}","session_id":"lattice-task-034-session","input_digest":"{}","model":"hermes-agent"}},"summary":"The adapter boundary is read-only and fail-closed.","findings":[{{"classification":"inference","statement":"Graphify evidence is bound to the requested snapshot.","evidence_digests":["{GRAPH_DIGEST}"]}}],"next_actions":["Persist only through the later LATTICE Memory port."]}}"#,
        job.input_digest().as_str(),
    )
}

fn capabilities() -> Response {
    Response::json(200, capabilities_value().to_string())
}

fn capabilities_value() -> serde_json::Value {
    serde_json::json!({
        "object": "hermes.api_server.capabilities",
        "platform": "hermes-agent",
        "model": "hermes-agent",
        "auth": {"type": "bearer", "required": true},
        "runtime": {
            "mode": "server_agent",
            "tool_execution": "server",
            "split_runtime": false
        },
        "features": {
            "run_submission": true,
            "run_status": true,
            "run_events_sse": true,
            "run_stop": true,
            "admin_config_rw": false,
            "memory_write_api": false
        }
    })
}

fn completed_events(run_id: &str, output: &str) -> Response {
    let event = serde_json::json!({
        "event": "run.completed",
        "run_id": run_id,
        "timestamp": 1.0,
        "output": output,
    });
    Response::sse(200, format!("data: {event}\n\n: stream closed\n\n"))
}

fn completed_status(run_id: &str, session_id: &str, output: &str) -> Response {
    Response::json(
        200,
        serde_json::json!({
            "object": "hermes.run",
            "run_id": run_id,
            "status": "completed",
            "session_id": session_id,
            "model": "hermes-agent",
            "output": output,
            "usage": {"input_tokens": 10, "output_tokens": 20, "total_tokens": 30},
        })
        .to_string(),
    )
}

fn request() -> HermesResearchRequest {
    HermesResearchRequest::new(
        Invocation::new(
            CONTRACT_VERSION,
            RequestId::new("request-hermes-1").expect("request id"),
            TaskId::new("task-034").expect("task id"),
            AttemptId::new("attempt-1").expect("attempt id"),
            ProjectSnapshotId::new("snapshot-79096b6").expect("snapshot id"),
            digest(SUBJECT_DIGEST),
        )
        .expect("invocation"),
    )
}

fn digest(value: &str) -> ContentDigest {
    ContentDigest::from_sha256(value.to_owned()).expect("sha256")
}

struct Response {
    status: u16,
    content_type: &'static str,
    body: String,
    delay: Duration,
}

impl Response {
    fn json(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    fn sse(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "text/event-stream",
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

struct ScriptedServer {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    handle: JoinHandle<()>,
}

impl ScriptedServer {
    fn start(responses: Vec<Response>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback fixture");
        let address = listener.local_addr().expect("fixture address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let handle = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("fixture accept");
                let request = read_request(&mut stream);
                captured.lock().expect("request lock").push(request);
                if !response.delay.is_zero() {
                    thread::sleep(response.delay);
                }
                write_response(&mut stream, &response);
            }
        });
        Self {
            address,
            requests,
            handle,
        }
    }

    fn address(&self) -> SocketAddr {
        self.address
    }

    fn finish(self) -> Vec<String> {
        self.handle.join().expect("fixture server");
        Arc::try_unwrap(self.requests)
            .expect("sole request owner")
            .into_inner()
            .expect("request lock")
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    let header_end = loop {
        let count = stream.read(&mut buffer).expect("read request");
        assert!(count > 0, "request closed before headers");
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(index) = find_bytes(&bytes, b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8(bytes[..header_end].to_vec()).expect("utf8 headers");
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.strip_prefix("Content-Length: ")
                .and_then(|value| value.parse::<usize>().ok())
        })
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let count = stream.read(&mut buffer).expect("read body");
        assert!(count > 0, "request closed before body");
        bytes.extend_from_slice(&buffer[..count]);
    }
    String::from_utf8(bytes).expect("utf8 request")
}

fn write_response(stream: &mut TcpStream, response: &Response) {
    let reason = match response.status {
        200 => "OK",
        202 => "Accepted",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        response.content_type,
        response.body.len()
    );
    if stream.write_all(head.as_bytes()).is_ok() {
        let _ = stream.write_all(response.body.as_bytes());
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
