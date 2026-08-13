use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

#[cfg(windows)]
use std::fs;
#[cfg(windows)]
use std::sync::atomic::{AtomicU64, Ordering};

use lattice_contracts::{DeliveryProfile, DeliveryRuntime};
#[cfg(windows)]
use lattice_hermes_adapter::preparation::materialize_official_preparation_bundle;
use lattice_runtime::composition::{
    LatticedDeliveryConfig, LatticedDeliveryService, LatticedErrorKind, fixed_gateway_submission,
};
use lattice_runtime::delivery_ledger::DeliveryDatabaseBinding;
use serde_json::{Value, json};
#[cfg(windows)]
use sha2::{Digest, Sha256};

#[cfg(windows)]
static NEXT_SCRIPTED_GATE_FIXTURE: AtomicU64 = AtomicU64::new(1);

#[cfg(windows)]
struct HermesPreparationFixtureCleanup(PathBuf);

#[cfg(windows)]
impl Drop for HermesPreparationFixtureCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(windows)]
fn hermes_preparation_fixture(
    name: &str,
) -> (PathBuf, PathBuf, String, HermesPreparationFixtureCleanup) {
    let unique = NEXT_SCRIPTED_GATE_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let fixture_root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("task058-{name}-{}-{unique}", std::process::id()));
    fs::create_dir_all(&fixture_root).expect("create preparation gate fixture root");
    let product_root = fixture_root.join("product");
    fs::create_dir(&product_root).expect("create protected product root");
    let preparation_root = fixture_root.join("prepared-assets");
    let outcome = materialize_official_preparation_bundle(&preparation_root, &product_root)
        .expect("materialize exact preparation gate fixture");
    let receipt = outcome.receipt().bundle_sha256().to_owned();
    (
        preparation_root,
        product_root,
        receipt,
        HermesPreparationFixtureCleanup(fixture_root),
    )
}

fn database(run_id: &str) -> DeliveryDatabaseBinding {
    DeliveryDatabaseBinding::new("127.0.0.1", 55432, run_id).expect("database binding")
}

#[test]
fn gateway_submission_carries_the_complete_server_owned_task_spec() {
    let submission = fixed_gateway_submission().expect("fixed Task Spec submission");
    let document: Value =
        serde_json::from_slice(submission.canonical_document()).expect("Task Spec document JSON");
    let object = document.as_object().expect("Task Spec object");

    assert_eq!(object.len(), 23);
    assert_eq!(document["schema_version"], "2.1");
    assert_eq!(document["task_id"], "TASK-038-CANARY");
    assert_eq!(document["project_id"], "task038-controlled-canary");
    assert_eq!(
        document["base_commit_id"],
        "e3b01a182c3273441c879d4d8b796865bba9131a"
    );
    assert_eq!(document["scope"]["allowed_paths"], json!(["answer.txt"]));
    assert_eq!(document["scope"]["forbidden_paths"], json!([".git/**"]));
    assert_eq!(
        submission.binding().task_spec_digest(),
        submission.claimed_spec_digest()
    );
}

#[allow(clippy::too_many_arguments)]
fn delivery_config(
    launcher: &str,
    version: &str,
    launcher_sha256: char,
    schema_directory: &str,
    codex_home: &str,
    delivery_root: &str,
    git_executable: &str,
    timeout: Duration,
    runtime: DeliveryRuntime,
) -> LatticedDeliveryConfig {
    LatticedDeliveryConfig::new(
        PathBuf::from(launcher),
        version,
        launcher_sha256.to_string().repeat(64),
        PathBuf::from(schema_directory),
        PathBuf::from(codex_home),
        PathBuf::from(delivery_root),
        PathBuf::from(git_executable),
        timeout,
        runtime,
    )
    .expect("delivery config")
}

#[test]
#[allow(clippy::too_many_lines)]
fn execution_configuration_digest_detects_every_process_owned_substitution() {
    let run_id = "0123456789abcdef0123456789abcdef";
    let base = delivery_config(
        r"C:\tools\codex.exe",
        "codex-cli 0.144.6",
        'a',
        r"C:\delivery\schema",
        r"C:\delivery\codex-home",
        r"C:\delivery\root",
        r"C:\tools\git.exe",
        Duration::from_secs(30),
        DeliveryRuntime::ScriptedAcceptance,
    );
    let base_service = LatticedDeliveryService::for_delivery(
        base.clone(),
        database(run_id),
        "password-one".to_owned(),
    )
    .expect("base service");
    let base_digest = base_service
        .request_binding()
        .expect("delivery request")
        .configuration_digest()
        .clone();

    let substitutions = [
        delivery_config(
            r"C:\tools\codex-next.exe",
            "codex-cli 0.144.6",
            'a',
            r"C:\delivery\schema",
            r"C:\delivery\codex-home",
            r"C:\delivery\root",
            r"C:\tools\git.exe",
            Duration::from_secs(30),
            DeliveryRuntime::ScriptedAcceptance,
        ),
        delivery_config(
            r"C:\tools\codex.exe",
            "codex-cli 0.145.0",
            'a',
            r"C:\delivery\schema",
            r"C:\delivery\codex-home",
            r"C:\delivery\root",
            r"C:\tools\git.exe",
            Duration::from_secs(30),
            DeliveryRuntime::ScriptedAcceptance,
        ),
        delivery_config(
            r"C:\tools\codex.exe",
            "codex-cli 0.144.6",
            'b',
            r"C:\delivery\schema",
            r"C:\delivery\codex-home",
            r"C:\delivery\root",
            r"C:\tools\git.exe",
            Duration::from_secs(30),
            DeliveryRuntime::ScriptedAcceptance,
        ),
        delivery_config(
            r"C:\tools\codex.exe",
            "codex-cli 0.144.6",
            'a',
            r"C:\delivery\schema-next",
            r"C:\delivery\codex-home",
            r"C:\delivery\root",
            r"C:\tools\git.exe",
            Duration::from_secs(30),
            DeliveryRuntime::ScriptedAcceptance,
        ),
        delivery_config(
            r"C:\tools\codex.exe",
            "codex-cli 0.144.6",
            'a',
            r"C:\delivery\schema",
            r"C:\delivery\codex-home-next",
            r"C:\delivery\root",
            r"C:\tools\git.exe",
            Duration::from_secs(30),
            DeliveryRuntime::ScriptedAcceptance,
        ),
        delivery_config(
            r"C:\tools\codex.exe",
            "codex-cli 0.144.6",
            'a',
            r"C:\delivery\schema",
            r"C:\delivery\codex-home",
            r"C:\delivery\root-next",
            r"C:\tools\git.exe",
            Duration::from_secs(30),
            DeliveryRuntime::ScriptedAcceptance,
        ),
        delivery_config(
            r"C:\tools\codex.exe",
            "codex-cli 0.144.6",
            'a',
            r"C:\delivery\schema",
            r"C:\delivery\codex-home",
            r"C:\delivery\root",
            r"C:\tools\git-next.exe",
            Duration::from_secs(30),
            DeliveryRuntime::ScriptedAcceptance,
        ),
        delivery_config(
            r"C:\tools\codex.exe",
            "codex-cli 0.144.6",
            'a',
            r"C:\delivery\schema",
            r"C:\delivery\codex-home",
            r"C:\delivery\root",
            r"C:\tools\git.exe",
            Duration::from_secs(31),
            DeliveryRuntime::ScriptedAcceptance,
        ),
    ];

    for substituted in substitutions {
        let service = LatticedDeliveryService::for_delivery(
            substituted,
            database(run_id),
            "password-one".to_owned(),
        )
        .expect("substituted service");
        assert_ne!(
            service
                .request_binding()
                .expect("delivery request")
                .configuration_digest(),
            &base_digest
        );
    }

    let different_secret =
        LatticedDeliveryService::for_delivery(base, database(run_id), "password-two".to_owned())
            .expect("different secret service");
    assert_eq!(
        different_secret
            .request_binding()
            .expect("delivery request")
            .configuration_digest(),
        &base_digest
    );
}

#[test]
fn restart_status_does_not_fabricate_an_execution_configuration_without_postgres() {
    let first = LatticedDeliveryService::status_only(
        database("0123456789abcdef0123456789abcdef"),
        "test-password".to_owned(),
        Duration::from_secs(30),
    )
    .expect("first service");
    let restarted = LatticedDeliveryService::status_only(
        database("0123456789abcdef0123456789abcdef"),
        "test-password".to_owned(),
        Duration::from_secs(30),
    )
    .expect("restarted service");
    let another_run = LatticedDeliveryService::status_only(
        database("fedcba9876543210fedcba9876543210"),
        "test-password".to_owned(),
        Duration::from_secs(30),
    )
    .expect("another service");

    assert!(first.request_binding().is_none());
    assert!(restarted.request_binding().is_none());
    assert!(another_run.request_binding().is_none());
}

#[test]
fn executable_has_config_binding_while_status_waits_for_durable_reconstruction() {
    let database = database("0123456789abcdef0123456789abcdef");
    let config = LatticedDeliveryConfig::new(
        PathBuf::from(r"C:\tools\codex.exe"),
        "codex-cli 0.144.6",
        "a".repeat(64),
        PathBuf::from(r"C:\delivery\schema"),
        PathBuf::from(r"C:\delivery\codex-home"),
        PathBuf::from(r"C:\delivery\root"),
        PathBuf::from(r"C:\tools\git.exe"),
        Duration::from_secs(30),
        DeliveryRuntime::ScriptedAcceptance,
    )
    .expect("delivery config");
    let executable =
        LatticedDeliveryService::for_delivery(config, database.clone(), "test-password".to_owned())
            .expect("executable service");
    let status = LatticedDeliveryService::status_only(
        database,
        "test-password".to_owned(),
        Duration::from_secs(30),
    )
    .expect("status service");

    assert_eq!(
        executable
            .request_binding()
            .expect("executable request")
            .profile(),
        DeliveryProfile::Task032CodexPostgres
    );
    assert!(status.request_binding().is_none());
}

#[cfg(windows)]
#[test]
fn official_codex_rejects_arbitrary_or_content_mismatched_launchers_before_effects() {
    let arbitrary = LatticedDeliveryConfig::new(
        PathBuf::from(r"C:\tools\codex.exe"),
        "codex-cli 0.146.0",
        "bc343ba420dc2e2e9f59e6fc5e5bf0aae1cd8c771fc319665241fc9c0271fddb",
        PathBuf::from(r"C:\delivery\schema"),
        PathBuf::from(r"C:\delivery\codex-home"),
        PathBuf::from(r"C:\delivery\root"),
        PathBuf::from(r"C:\tools\git.exe"),
        Duration::from_secs(30),
        DeliveryRuntime::OfficialCodexAppServer,
    )
    .expect_err("an arbitrary launcher path must fail before any database effect");
    assert_eq!(arbitrary.kind(), LatticedErrorKind::OfficialLiveBlocked);
    assert_eq!(arbitrary.code(), "LATTICE_OFFICIAL_CODEX_IDENTITY_REJECTED");

    let unique = NEXT_SCRIPTED_GATE_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let repository_root = std::env::temp_dir().join(format!(
        "lattice-official-gate-{}-{unique}",
        std::process::id()
    ));
    let target_root = repository_root.join("target");
    let launcher = target_root
        .join("codex-official")
        .join("0.146.0")
        .join("node_modules")
        .join("@openai")
        .join("codex-win32-x64")
        .join("vendor")
        .join("x86_64-pc-windows-msvc")
        .join("bin")
        .join("codex.exe");
    fs::create_dir_all(launcher.parent().expect("launcher parent"))
        .expect("create exact-looking bundle layout");
    fs::write(&launcher, b"not the official Codex launcher")
        .expect("write content-mismatched launcher");
    let delivery_root = target_root
        .join("lattice-delivery")
        .join("a".repeat(32))
        .join("delivery");
    let mismatched = LatticedDeliveryConfig::new(
        launcher,
        "codex-cli 0.146.0",
        "bc343ba420dc2e2e9f59e6fc5e5bf0aae1cd8c771fc319665241fc9c0271fddb",
        target_root.join("schema"),
        repository_root.join("codex-home"),
        delivery_root,
        PathBuf::from(r"C:\tools\git.exe"),
        Duration::from_secs(30),
        DeliveryRuntime::OfficialCodexAppServer,
    )
    .expect_err("self-claimed identity cannot authorize mismatched launcher bytes");
    fs::remove_dir_all(&repository_root).expect("remove owned official gate fixture");
    assert_eq!(mismatched.kind(), LatticedErrorKind::OfficialLiveBlocked);
    assert_eq!(
        mismatched.code(),
        "LATTICE_OFFICIAL_CODEX_IDENTITY_REJECTED"
    );
}

#[test]
fn untrusted_launcher_cannot_bypass_the_incident_gate_by_claiming_scripted_runtime() {
    let config = LatticedDeliveryConfig::new(
        PathBuf::from(r"C:\official\codex.exe"),
        "codex-cli 0.144.6",
        "a".repeat(64),
        PathBuf::from(r"C:\delivery\schema"),
        PathBuf::from(r"C:\delivery\codex-home"),
        PathBuf::from(r"C:\delivery\root"),
        PathBuf::from(r"C:\tools\git.exe"),
        Duration::from_secs(30),
        DeliveryRuntime::ScriptedAcceptance,
    )
    .expect("configuration remains inspectable");
    let mut service = LatticedDeliveryService::for_delivery(
        config,
        database("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
        "test-password".to_owned(),
    )
    .expect("service binding");

    let error = service
        .run_scripted_acceptance_json()
        .expect_err("a mode label cannot authorize an untrusted launcher");

    assert_eq!(error.kind(), LatticedErrorKind::ScriptedFixtureRejected);
    assert_eq!(error.code(), "LATTICE_SCRIPTED_FIXTURE_REJECTED");
}

#[cfg(windows)]
#[test]
fn self_consistent_marker_and_wrapper_cannot_authorize_a_tampered_scripted_server() {
    let unique = NEXT_SCRIPTED_GATE_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let repository_root = std::env::temp_dir().join(format!(
        "lattice-scripted-gate-{}-{unique}",
        std::process::id()
    ));
    let fixture_id = "c".repeat(32);
    let fixture_root = repository_root
        .join("target")
        .join("lattice-delivery")
        .join(&fixture_id);
    let codex_home = fixture_root.join("codex-home");
    fs::create_dir_all(&codex_home).expect("create tampered fixture");

    let server = fixture_root.join("scripted-codex.ps1");
    let tampered_server = b"Write-Output 'not the LATTICE fixture'\n";
    fs::write(&server, tampered_server).expect("write tampered server");
    let server_sha256 = test_sha256(tampered_server);
    let launcher = fixture_root.join("scripted-codex.cmd");
    let launcher_bytes = test_scripted_launcher_bytes(&server_sha256);
    fs::write(&launcher, &launcher_bytes).expect("write exact wrapper");
    let launcher_sha256 = test_sha256(&launcher_bytes);
    let canonical_root = fs::canonicalize(&fixture_root).expect("canonical fixture root");
    let canonical_repository = fs::canonicalize(&repository_root).expect("canonical repository");
    let canonical_launcher = fs::canonicalize(&launcher).expect("canonical launcher");
    let canonical_server = fs::canonicalize(&server).expect("canonical server");
    fs::write(
        fixture_root.join(".lattice-delivery-fixture-v1.json"),
        serde_json::to_vec(&json!({
            "kind": "LATTICE_DELIVERY_SCRIPTED_ACCEPTANCE_V1",
            "fixture_id": fixture_id,
            "root": canonical_root.to_string_lossy(),
            "repository_root": canonical_repository.to_string_lossy(),
            "codex_mode": "SCRIPTED_ACCEPTANCE",
            "launcher_path": canonical_launcher.to_string_lossy(),
            "launcher_sha256": launcher_sha256,
            "server_path": canonical_server.to_string_lossy(),
            "server_sha256": server_sha256,
        }))
        .expect("marker json"),
    )
    .expect("write self-consistent marker");

    let config = LatticedDeliveryConfig::new(
        launcher,
        "codex-cli 0.144.6",
        launcher_sha256,
        fixture_root.join("schema"),
        codex_home,
        fixture_root.join("delivery"),
        fixture_root.join("git.exe"),
        Duration::from_secs(30),
        DeliveryRuntime::ScriptedAcceptance,
    )
    .expect("configuration remains inspectable");
    let mut service = LatticedDeliveryService::for_delivery(
        config,
        database("cccccccccccccccccccccccccccccccc"),
        "test-password".to_owned(),
    )
    .expect("service binding");

    let result = service.run_scripted_acceptance_json();
    fs::remove_dir_all(&repository_root).expect("remove owned tampered fixture");

    let error = result.expect_err("tampered server bytes must fail before database or process");
    assert_eq!(error.kind(), LatticedErrorKind::ScriptedFixtureRejected);
}

#[cfg(windows)]
fn test_sha256(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut output, "{byte:02x}").expect("write digest");
    }
    output
}

#[cfg(windows)]
fn test_scripted_launcher_bytes(server_sha256: &str) -> Vec<u8> {
    format!(
        concat!(
            "@echo off\r\n",
            "if \"%~1\"==\"--version\" if \"%~2\"==\"\" goto version\r\n",
            "if \"%~1\"==\"app-server\" if \"%~2\"==\"generate-json-schema\" if \"%~3\"==\"--out\" if \"%~4\" NEQ \"\" if \"%~5\"==\"\" goto schema\r\n",
            "if \"%~1\"==\"app-server\" if \"%~2\"==\"--listen\" if \"%~3\"==\"stdio://\" if \"%~4\"==\"\" goto server\r\n",
            "exit /b 11\r\n",
            ":version\r\n",
            "echo codex-cli 0.144.6\r\n",
            "exit /b 0\r\n",
            ":schema\r\n",
            "\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"%~dp0scripted-codex.ps1\" -ExpectedSelfSha256 \"{server_sha256}\" -Mode Schema -SchemaRoot \"%~4\"\r\n",
            "exit /b %ERRORLEVEL%\r\n",
            ":server\r\n",
            "\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"%~dp0scripted-codex.ps1\" -ExpectedSelfSha256 \"{server_sha256}\" -Mode Server\r\n",
            "exit /b %ERRORLEVEL%\r\n"
        ),
        server_sha256 = server_sha256
    )
    .into_bytes()
}

fn spawn_bounded_latticed() -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_latticed"))
        .env("LATTICE_DELIVERY_CODEX_MODE", "OFFICIAL_CODEX_APP_SERVER")
        .env("LATTICE_FULL_CHAIN_RUN_MODE", "RESUME_EXISTING")
        .env("LATTICE_DELIVERY_LAUNCHER", r"C:\tools\codex.exe")
        .env("LATTICE_DELIVERY_LAUNCHER_VERSION", "codex-cli 0.144.6")
        .env("LATTICE_DELIVERY_LAUNCHER_SHA256", "a".repeat(64))
        .env("LATTICE_DELIVERY_SCHEMA_DIR", r"C:\delivery\schema")
        .env("LATTICE_DELIVERY_CODEX_HOME", r"C:\delivery\codex-home")
        .env("LATTICE_DELIVERY_ROOT", r"C:\delivery\root")
        .env("LATTICE_DELIVERY_GIT_EXE", r"C:\tools\git.exe")
        .env("LATTICE_TASK019_HOST", "127.0.0.1")
        .env("LATTICE_TASK019_PORT", "1")
        .env("LATTICE_TASK019_RUN_ID", "0123456789abcdef0123456789abcdef")
        .env("LATTICE_TASK019_PASSWORD", "test-password")
        .env("LATTICE_DELIVERY_TIMEOUT_SECONDS", "1")
        .env("LATTICE_STORE_DAEMON_INSTANCE_ID", "test-daemon")
        .env("LATTICE_STORE_DAEMON_EPOCH", "1")
        .env("LATTICE_STORE_AUTHORITY_REVISION", "1")
        .env("LATTICE_STORE_OBSERVATION_DIGEST", "b".repeat(64))
        .env("LATTICE_STORE_AUTHORITY_HEAD_DIGEST", "c".repeat(64))
        .env(
            "LATTICE_TASK_INGRESS_KIND",
            "LOCAL_CANONICAL_MCP_ACCEPTANCE",
        )
        .env("LATTICE_TASK_INGRESS_PROFILE_SHA256", "d".repeat(64))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start latticed")
}

fn assert_safe_startup_diagnostics(stderr: &[u8], expected_stages: &[&str]) {
    let text = std::str::from_utf8(stderr).expect("stderr UTF-8");
    assert!(!text.contains("test-password"));
    assert!(!text.contains("127.0.0.1"));
    let records = text
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("startup diagnostic JSON"))
        .collect::<Vec<_>>();
    assert!(!records.is_empty());
    for record in &records {
        assert_eq!(record.as_object().expect("diagnostic object").len(), 7);
        assert_eq!(record["schema"], "lattice.latticed.startup-diagnostic.v1");
        assert!(record["stage"].is_string());
        assert!(record["last_completed_stage"].is_string());
        assert!(record["waiting_reason"].is_string());
        assert!(record["configuration_health"].is_string());
        assert!(record["dependency_health"].is_string());
        assert!(record["failure_classification"].is_string());
    }
    let stages = records
        .iter()
        .filter_map(|record| record["stage"].as_str())
        .collect::<Vec<_>>();
    for expected_stage in expected_stages {
        assert!(
            stages.contains(expected_stage),
            "missing {expected_stage}: {stages:?}"
        );
    }
}

#[test]
fn real_latticed_binary_serves_only_the_four_bounded_tools() {
    let mut child = spawn_bounded_latticed();
    let task_ref = fixed_gateway_submission()
        .expect("fixed submission")
        .binding()
        .task_spec_digest()
        .as_str()
        .to_owned();
    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"lattice_delivery_run","arguments":{}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"lattice_delivery_status"}}),
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"lattice_task_submit","arguments":{"client_request_id":"composition-test","intent":"CONTROLLED_CODEX_CANARY"}}}),
        json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"lattice_task_status","arguments":{"task_ref":task_ref}}}),
    ];
    let input = requests
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write MCP requests");
    let output = child.wait_with_output().expect("wait latticed");

    assert!(output.status.success());
    assert_safe_startup_diagnostics(
        &output.stderr,
        &[
            "CONFIGURATION_VALIDATION_STARTED",
            "CONFIGURATION_VALIDATED",
            "SERVICE_ASSEMBLY_STARTED",
            "SERVICE_ASSEMBLED",
            "STDIO_LOOP_ENTERED",
            "MCP_INITIALIZE_RECEIVED",
            "MCP_INITIALIZED_NOTIFICATION_RECEIVED",
            "MCP_TOOLS_LIST_RECEIVED",
            "MCP_END_OF_STREAM",
        ],
    );
    let responses = String::from_utf8(output.stdout)
        .expect("stdout utf8")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("JSON-RPC response"))
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 6);
    assert_eq!(responses[0]["result"]["capabilities"], json!({"tools": {}}));
    let tools = responses[1]["result"]["tools"]
        .as_array()
        .expect("tool list");
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect::<Vec<_>>(),
        [
            "lattice_delivery_run",
            "lattice_delivery_status",
            "lattice_task_submit",
            "lattice_task_status",
        ]
    );
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        if tool["name"] == "lattice_delivery_run" || tool["name"] == "lattice_delivery_status" {
            assert_eq!(tool["inputSchema"].as_object().expect("schema").len(), 2);
        } else {
            assert!(tool["inputSchema"]["properties"].is_object());
            assert!(tool["inputSchema"]["required"].is_array());
        }
        assert!(tool.get("annotations").is_none());
    }
    for response in &responses[2..] {
        assert_eq!(response["result"]["isError"], true);
        assert_ne!(
            response["result"]["structuredContent"]["code"],
            "LATTICE_FULL_CHAIN_BINDING_REJECTED"
        );
        assert_ne!(
            response["result"]["structuredContent"]["code"],
            "LATTICE_TASK_SUBMIT_UNAVAILABLE"
        );
        assert_ne!(
            response["result"]["structuredContent"]["code"],
            "LATTICE_TASK_STATUS_UNAVAILABLE"
        );
    }
}

#[test]
fn real_latticed_binary_supports_stateless_modern_discovery_and_calls() {
    let mut child = spawn_bounded_latticed();
    let task_ref = fixed_gateway_submission()
        .expect("fixed submission")
        .binding()
        .task_spec_digest()
        .as_str()
        .to_owned();
    let metadata = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":metadata.clone()}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"_meta":metadata.clone()}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"lattice_delivery_run","arguments":{},"_meta":metadata.clone()}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"lattice_delivery_status","_meta":metadata.clone()}}),
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"lattice_task_submit","arguments":{"client_request_id":"modern-composition-test","intent":"CONTROLLED_CODEX_CANARY"},"_meta":metadata.clone()}}),
        json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"lattice_task_status","arguments":{"task_ref":task_ref},"_meta":metadata}}),
    ];
    let input = requests
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write modern MCP requests");
    let output = child.wait_with_output().expect("wait latticed");

    assert!(output.status.success());
    assert_safe_startup_diagnostics(
        &output.stderr,
        &[
            "CONFIGURATION_VALIDATION_STARTED",
            "CONFIGURATION_VALIDATED",
            "SERVICE_ASSEMBLY_STARTED",
            "SERVICE_ASSEMBLED",
            "STDIO_LOOP_ENTERED",
            "MCP_TOOLS_LIST_RECEIVED",
            "MCP_END_OF_STREAM",
        ],
    );
    let responses = String::from_utf8(output.stdout)
        .expect("stdout utf8")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("JSON-RPC response"))
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 6);
    assert_eq!(
        responses[0]["result"]["supportedVersions"],
        json!(["2026-07-28"])
    );
    assert_eq!(responses[0]["result"]["capabilities"], json!({"tools": {}}));
    let tools = responses[1]["result"]["tools"]
        .as_array()
        .expect("tool list");
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect::<Vec<_>>(),
        [
            "lattice_delivery_run",
            "lattice_delivery_status",
            "lattice_task_submit",
            "lattice_task_status",
        ]
    );
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        assert!(tool["annotations"].is_object());
    }
    for response in &responses {
        assert_eq!(response["result"]["resultType"], "complete");
    }
    for response in &responses[2..] {
        assert_eq!(response["result"]["isError"], true);
        assert_ne!(
            response["result"]["structuredContent"]["code"],
            "LATTICE_FULL_CHAIN_BINDING_REJECTED"
        );
        assert_ne!(
            response["result"]["structuredContent"]["code"],
            "LATTICE_TASK_SUBMIT_UNAVAILABLE"
        );
        assert_ne!(
            response["result"]["structuredContent"]["code"],
            "LATTICE_TASK_STATUS_UNAVAILABLE"
        );
    }
}

#[test]
fn full_chain_binary_is_reachable_and_fails_closed_without_a_sealed_hermes_runner() {
    let output = Command::new(env!("CARGO_BIN_EXE_lattice-full-chain"))
        .env_clear()
        .env("LATTICE_DELIVERY_CODEX_MODE", "SCRIPTED_ACCEPTANCE")
        .env("LATTICE_TASK019_HOST", "not-a-database-host")
        .output()
        .expect("start full-chain entrypoint");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr utf8"),
        "LATTICE_HERMES_PREPARATION_REJECTED\n"
    );
}

#[test]
fn legacy_full_chain_entry_rejects_hermes_preflight() {
    let output = Command::new(env!("CARGO_BIN_EXE_lattice-full-chain"))
        .arg("--hermes-preflight")
        .env_clear()
        .output()
        .expect("start legacy full-chain entrypoint");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr utf8"),
        "LATTICE_FULL_CHAIN_ARGUMENTS_REJECTED\n"
    );
}

#[cfg(windows)]
#[test]
fn latticed_hermes_prepare_materializes_then_revalidates_without_launch_configuration() {
    const SECRET_SENTINEL: &str = "TASK057-SECRET-MUST-NOT-BE-READ-OR-RENDERED";

    struct FixtureCleanup(PathBuf);

    impl Drop for FixtureCleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    let unique = NEXT_SCRIPTED_GATE_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let fixture_root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "task057-hermes-prepare-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&fixture_root).expect("create test-owned preparation fixture");
    let _cleanup = FixtureCleanup(fixture_root.clone());
    let product_root = fixture_root.join("product");
    fs::create_dir(&product_root).expect("create protected product root");
    let preparation_root = fixture_root.join("prepared-assets");

    let invoke = || {
        Command::new(env!("CARGO_BIN_EXE_latticed"))
            .arg("--hermes-prepare")
            .env_clear()
            .env("LATTICE_HERMES_PREPARATION_ROOT", &preparation_root)
            .env("LATTICE_HERMES_PRODUCT_ROOT", &product_root)
            .env("LATTICE_HERMES_API_KEY", SECRET_SENTINEL)
            .output()
            .expect("start canonical latticed Hermes preparation")
    };

    let created = invoke();
    assert_eq!(created.status.code(), Some(0));
    assert!(created.stdout.is_empty());
    let created_stderr = String::from_utf8(created.stderr).expect("created stderr UTF-8");
    let created_digest = created_stderr
        .strip_prefix("LATTICE_HERMES_PREPARE_ASSETS_CREATED_UNVERIFIED:")
        .and_then(|value| value.strip_suffix('\n'))
        .expect("fixed created receipt");
    assert_eq!(created_digest.len(), 64);
    assert!(created_digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert!(!created_stderr.contains(SECRET_SENTINEL));

    let present = invoke();
    assert_eq!(present.status.code(), Some(0));
    assert!(present.stdout.is_empty());
    assert_eq!(
        String::from_utf8(present.stderr).expect("present stderr UTF-8"),
        format!("LATTICE_HERMES_PREPARE_ASSETS_PRESENT_UNVERIFIED:{created_digest}\n")
    );
    let mut file_names = fs::read_dir(&preparation_root)
        .expect("read preparation root")
        .map(|entry| {
            entry
                .expect("prepared entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    file_names.sort();
    assert_eq!(
        file_names,
        ["offline-runtime-manifest.json", "prepared-assets.json"]
    );
}

#[test]
fn latticed_hermes_preflight_reports_exact_missing_settings() {
    let output = Command::new(env!("CARGO_BIN_EXE_latticed"))
        .arg("--hermes-preflight")
        .env_clear()
        .output()
        .expect("start canonical latticed Hermes preflight");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr utf8"),
        concat!(
            "LATTICE_HERMES_PREFLIGHT_MISSING_CONFIGURATION:",
            "LATTICE_HERMES_PREPARATION_ROOT,",
            "LATTICE_HERMES_PREPARATION_RECEIPT_SHA256,",
            "LATTICE_HERMES_RUNTIME_MANIFEST,",
            "LATTICE_HERMES_RUNTIME_GUEST_ROOT,",
            "LATTICE_HERMES_API_KEY,",
            "LATTICE_HERMES_PRODUCT_ROOT,",
            "LATTICE_HERMES_WSL_EXE,",
            "LATTICE_HERMES_ISOLATION_ROOT,",
            "LATTICE_HERMES_BROKER_HELPER,",
            "LATTICE_HERMES_BROKER_HELPER_SHA256,",
            "LATTICE_HERMES_CODEX_LAUNCHER,",
            "LATTICE_HERMES_CODEX_HOME,",
            "LATTICE_HERMES_BROKER_ISOLATION_ROOT,",
            "LATTICE_HERMES_DEADLINE_SECONDS\n",
        )
    );
}

#[test]
fn latticed_hermes_preflight_rejects_unavailable_manifest_without_echoing_values() {
    const SECRET_SENTINEL: &str = "TASK056-SECRET-SENTINEL-DO-NOT-LEAK";
    const PATH_SENTINEL: &str = r"C:\TASK056-PATH-SENTINEL-DO-NOT-LEAK\manifest.json";
    #[cfg(windows)]
    let (preparation_root, product_root, preparation_receipt, _cleanup) =
        hermes_preparation_fixture("unavailable-manifest");
    let mut command = Command::new(env!("CARGO_BIN_EXE_latticed"));
    command
        .arg("--hermes-preflight")
        .env_clear()
        .env("LATTICE_HERMES_RUNTIME_MANIFEST", PATH_SENTINEL)
        .env("LATTICE_HERMES_RUNTIME_GUEST_ROOT", "/runtime")
        .env("LATTICE_HERMES_API_KEY", SECRET_SENTINEL)
        .env("LATTICE_HERMES_WSL_EXE", r"C:\Windows\System32\wsl.exe")
        .env("LATTICE_HERMES_ISOLATION_ROOT", r"C:\isolation")
        .env("LATTICE_HERMES_BROKER_HELPER", r"C:\broker\helper.exe")
        .env("LATTICE_HERMES_BROKER_HELPER_SHA256", "a".repeat(64))
        .env("LATTICE_HERMES_CODEX_LAUNCHER", r"C:\codex\codex.exe")
        .env("LATTICE_HERMES_CODEX_HOME", r"C:\codex\home")
        .env(
            "LATTICE_HERMES_BROKER_ISOLATION_ROOT",
            r"C:\broker\isolation",
        )
        .env("LATTICE_HERMES_DEADLINE_SECONDS", "30");
    #[cfg(windows)]
    command
        .env("LATTICE_HERMES_PREPARATION_ROOT", &preparation_root)
        .env(
            "LATTICE_HERMES_PREPARATION_RECEIPT_SHA256",
            &preparation_receipt,
        )
        .env("LATTICE_HERMES_PRODUCT_ROOT", &product_root);
    let output = command
        .output()
        .expect("start canonical latticed Hermes preflight");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("stderr utf8");
    assert_eq!(stderr, "LATTICE_HERMES_PREFLIGHT_CONFIGURATION_REJECTED\n");
    assert!(!stderr.contains(SECRET_SENTINEL));
    assert!(!stderr.contains(PATH_SENTINEL));
}

#[cfg(windows)]
#[test]
fn latticed_hermes_preflight_rejects_invalid_secret_after_exact_manifest_identity() {
    const MANIFEST_BYTES: &[u8] = br#"{"cpython_archive_bytes":111375313,"cpython_archive_sha256":"a140c0868258075d160fa0da51ddffd423efbc9dd350695abd33e7ce3ce94352","cpython_build_release":"20260804","cpython_provenance":"astral-sh/python-build-standalone","cpython_sha256sums_sha256":"eccfdcc61c9fe48b7fe61db8812925ce30f23943d16c60861001004a4ae8f55c","cpython_version":"3.12.13","hermes_archive_sha256":"a9a84a25999a23a859a9d17ef3134ea1c3371d8bf1984313eab839e939528152","hermes_commit":"3c27eb6234bf91b8ceee9e9071591b31e9b148cb","hermes_release":"v2026.8.3","payload_byte_count":722643145,"payload_file_count":14077,"payload_manifest_sha256":"cb0e331bcb2b4fe2fd0977401d246819aadb800b645ca31ec233ad4e25b96929","platform":"x86_64-unknown-linux-gnu","pyproject_sha256":"64d1085ee1c23caf0ae0d9e65c73e280f466362ed43fdda1531f18f3af1d9869","schema":"lattice.hermes.offline-runtime.v1","uv_lock_sha256":"aab3c83f71b683507a590b6315b23bdc0abd6b63b76b2349eae15bf00dfbaf2b"}"#;
    const MANIFEST_SHA256: &str =
        "e3a3272b6cead30cd2df1af755df031766475595fdacfb080d0886671b6d1fbb";
    const RUNTIME_GUEST_ROOT: &str = concat!(
        "/var/tmp/lattice-runtime-targets/",
        "hermes-v2026.8.3-cpython-3.12.13-pbs-20260804-errorfix-v1"
    );
    const SECRET_SENTINEL: &str = "TASK056-SECRET";

    struct FixtureCleanup(PathBuf);

    impl Drop for FixtureCleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    assert_eq!(test_sha256(MANIFEST_BYTES), MANIFEST_SHA256);
    let unique = NEXT_SCRIPTED_GATE_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let fixture_root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "task056-hermes-invalid-secret-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&fixture_root).expect("create test-owned Hermes fixture root");
    let _cleanup = FixtureCleanup(fixture_root.clone());
    let product_root = fixture_root.join("product");
    fs::create_dir(&product_root).expect("create protected product root");
    let preparation_root = fixture_root.join("prepared-assets");
    let preparation = materialize_official_preparation_bundle(&preparation_root, &product_root)
        .expect("materialize exact preparation gate fixture");
    let manifest_path = fixture_root.join("TASK056-PATH-SENTINEL-manifest.json");
    fs::write(&manifest_path, MANIFEST_BYTES).expect("write exact pinned manifest fixture");
    let manifest_path_text = manifest_path
        .to_str()
        .expect("test-owned manifest path UTF-8");

    let output = Command::new(env!("CARGO_BIN_EXE_latticed"))
        .arg("--hermes-preflight")
        .env_clear()
        .env("LATTICE_HERMES_PREPARATION_ROOT", &preparation_root)
        .env(
            "LATTICE_HERMES_PREPARATION_RECEIPT_SHA256",
            preparation.receipt().bundle_sha256(),
        )
        .env("LATTICE_HERMES_RUNTIME_MANIFEST", manifest_path_text)
        .env("LATTICE_HERMES_RUNTIME_GUEST_ROOT", RUNTIME_GUEST_ROOT)
        .env("LATTICE_HERMES_API_KEY", SECRET_SENTINEL)
        .env("LATTICE_HERMES_PRODUCT_ROOT", &product_root)
        .env("LATTICE_HERMES_WSL_EXE", r"C:\Windows\System32\wsl.exe")
        .env("LATTICE_HERMES_ISOLATION_ROOT", r"C:\isolation")
        .env("LATTICE_HERMES_BROKER_HELPER", r"C:\broker\helper.exe")
        .env("LATTICE_HERMES_BROKER_HELPER_SHA256", "a".repeat(64))
        .env("LATTICE_HERMES_CODEX_LAUNCHER", r"C:\codex\codex.exe")
        .env("LATTICE_HERMES_CODEX_HOME", r"C:\codex\home")
        .env(
            "LATTICE_HERMES_BROKER_ISOLATION_ROOT",
            r"C:\broker\isolation",
        )
        .env("LATTICE_HERMES_DEADLINE_SECONDS", "30")
        .output()
        .expect("start canonical latticed invalid-secret preflight");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("stderr utf8");
    assert_eq!(stderr, "LATTICE_HERMES_PREFLIGHT_CONFIGURATION_REJECTED\n");
    assert!(!stderr.contains(SECRET_SENTINEL));
    assert!(!stderr.contains(manifest_path_text));
}
