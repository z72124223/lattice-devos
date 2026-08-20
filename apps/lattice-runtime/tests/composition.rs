use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

#[cfg(windows)]
use std::fs;
#[cfg(windows)]
use std::sync::atomic::{AtomicU64, Ordering};

use lattice_contracts::{DeliveryProfile, DeliveryRuntime};
use lattice_runtime::composition::{
    LatticedDeliveryConfig, LatticedDeliveryService, LatticedErrorKind,
};
use lattice_runtime::delivery_ledger::DeliveryDatabaseBinding;
use lattice_runtime::mcp::DeliveryToolService;
use serde_json::{Value, json};
#[cfg(windows)]
use sha2::{Digest, Sha256};

#[cfg(windows)]
static NEXT_SCRIPTED_GATE_FIXTURE: AtomicU64 = AtomicU64::new(1);

fn database(run_id: &str) -> DeliveryDatabaseBinding {
    DeliveryDatabaseBinding::new("127.0.0.1", 55432, run_id).expect("database binding")
}

fn assert_mcp_service<T: DeliveryToolService>() {}

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
        delivery_config(
            r"C:\tools\codex.exe",
            "codex-cli 0.144.6",
            'a',
            r"C:\delivery\schema",
            r"C:\delivery\codex-home",
            r"C:\delivery\root",
            r"C:\tools\git.exe",
            Duration::from_secs(30),
            DeliveryRuntime::OfficialCodexAppServer,
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
    assert_mcp_service::<LatticedDeliveryService>();
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

#[test]
fn official_codex_live_is_blocked_before_database_or_process_effects() {
    let config = LatticedDeliveryConfig::new(
        PathBuf::from(r"C:\tools\codex.exe"),
        "codex-cli 0.144.6",
        "a".repeat(64),
        PathBuf::from(r"C:\delivery\schema"),
        PathBuf::from(r"C:\delivery\codex-home"),
        PathBuf::from(r"C:\delivery\root"),
        PathBuf::from(r"C:\tools\git.exe"),
        Duration::from_secs(30),
        DeliveryRuntime::OfficialCodexAppServer,
    )
    .expect("official configuration remains inspectable");
    let mut service = LatticedDeliveryService::for_delivery(
        config,
        database("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        "test-password".to_owned(),
    )
    .expect("service binding");

    let error = service
        .run_json()
        .expect_err("official live must remain incident-blocked");

    assert_eq!(error.kind(), LatticedErrorKind::OfficialLiveBlocked);
    assert_eq!(error.code(), "LATTICE_OFFICIAL_CODEX_FAILED_DIAGNOSTIC");
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
        .run_json()
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

    let result = service.run_json();
    fs::remove_dir_all(&repository_root).expect("remove owned tampered fixture");

    let error = result.expect_err("tampered server bytes must fail before database or process");
    assert_eq!(error.kind(), LatticedErrorKind::ScriptedFixtureRejected);
}

#[cfg(windows)]
#[test]
#[allow(clippy::too_many_lines)]
fn scripted_fixture_schema_marks_only_schema_and_server_writes_only_answer() {
    let unique = NEXT_SCRIPTED_GATE_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let fixture_root = std::env::temp_dir().join(format!(
        "lattice-scripted-protocol-{}-{unique}",
        std::process::id()
    ));
    let repository_root = fixture_root.join("repo");
    let schema_root = fixture_root.join("schema");
    let codex_home = fixture_root.join("codex-home");
    fs::create_dir_all(&repository_root).expect("repository root");
    fs::create_dir_all(&codex_home).expect("codex home");
    fs::write(
        codex_home.join(".lattice-codex-home-v1"),
        b"lattice.codex-home.v1\n",
    )
    .expect("codex home marker");
    let script = fixture_root.join("scripted-codex.ps1");
    fs::write(
        &script,
        include_bytes!("../src/fixtures/task032-scripted-codex.ps1"),
    )
    .expect("script");
    let hash = test_sha256(&fs::read(&script).expect("script bytes"));
    let system_root = std::env::var_os("SystemRoot").expect("Windows system root");
    let powershell = PathBuf::from(&system_root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let modules = PathBuf::from(system_root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("Modules");

    let schema = Command::new(&powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .args([
            "-ExpectedSelfSha256",
            &hash,
            "-Mode",
            "Schema",
            "-SchemaRoot",
        ])
        .arg(&schema_root)
        .env("LATTICE_DELIVERY_CODEX_MODE", "SCRIPTED_ACCEPTANCE")
        .env("PSModulePath", &modules)
        .current_dir(&repository_root)
        .output()
        .expect("schema fixture starts");
    assert!(
        schema.status.success(),
        "schema fixture exit={:?} stdout={} stderr={}",
        schema.status.code(),
        String::from_utf8_lossy(&schema.stdout),
        String::from_utf8_lossy(&schema.stderr)
    );
    assert_eq!(
        fs::read(schema_root.join("lattice-scripted-app-server.json")).expect("schema marker"),
        br#"{"title":"LATTICE scripted app-server","type":"object"}"#
    );
    assert!(!repository_root.join("answer.txt").exists());

    let mut server = Command::new(&powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .args(["-ExpectedSelfSha256", &hash, "-Mode", "Server"])
        .env("LATTICE_DELIVERY_CODEX_MODE", "SCRIPTED_ACCEPTANCE")
        .env("LATTICE_DELIVERY_CODEX_HOME", &codex_home)
        .env("CODEX_HOME", &codex_home)
        .env("PSModulePath", &modules)
        .current_dir(&repository_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("server fixture starts");
    server
        .stdin
        .take()
        .expect("server stdin")
        .write_all(
            concat!(
                "{\"method\":\"initialize\",\"id\":0}\n",
                "{\"method\":\"initialized\"}\n",
                "{\"method\":\"thread/start\",\"id\":1,\"params\":{\"cwd\":\".\",\"approvalPolicy\":\"never\",\"sandbox\":\"workspace-write\"}}\n",
                "{\"method\":\"turn/start\",\"id\":2,\"params\":{\"threadId\":\"thread-task032-scripted\",\"cwd\":\".\",\"approvalPolicy\":\"never\",\"sandboxPolicy\":{\"type\":\"workspaceWrite\",\"networkAccess\":false,\"writableRoots\":[\".\"]},\"input\":[{\"type\":\"text\",\"text\":\"Create answer.txt in the current repository with exactly the bytes LATTICE_DELIVERY_OK followed by one newline. Do not modify any other path. Do not stage or commit files and do not run Git commands.\"}]}}\n"
            )
            .as_bytes(),
        )
        .expect("server requests");
    let answer = repository_root.join("answer.txt");
    for _ in 0..60 {
        if answer.is_file() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let answer_bytes = fs::read(&answer);
    server.kill().expect("stop idle scripted server");
    server.wait().expect("server stopped");
    fs::remove_dir_all(&fixture_root).expect("remove owned fixture");
    assert_eq!(
        answer_bytes.expect("server answer"),
        b"LATTICE_DELIVERY_OK\n"
    );
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

#[test]
fn real_latticed_binary_serves_only_the_two_bounded_tools() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_latticed"))
        .env("LATTICE_DELIVERY_CODEX_MODE", "SCRIPTED_ACCEPTANCE")
        .env("LATTICE_DELIVERY_LAUNCHER", r"C:\tools\codex.exe")
        .env("LATTICE_DELIVERY_LAUNCHER_VERSION", "codex-cli 0.144.6")
        .env("LATTICE_DELIVERY_LAUNCHER_SHA256", "a".repeat(64))
        .env("LATTICE_DELIVERY_SCHEMA_DIR", r"C:\delivery\schema")
        .env("LATTICE_DELIVERY_CODEX_HOME", r"C:\delivery\codex-home")
        .env("LATTICE_DELIVERY_ROOT", r"C:\delivery\root")
        .env("LATTICE_DELIVERY_GIT_EXE", r"C:\tools\git.exe")
        .env("LATTICE_TASK019_HOST", "127.0.0.1")
        .env("LATTICE_TASK019_PORT", "55432")
        .env("LATTICE_TASK019_RUN_ID", "0123456789abcdef0123456789abcdef")
        .env("LATTICE_TASK019_PASSWORD", "test-password")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start latticed");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(
            concat!(
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test\",\"version\":\"1\"}}}\n",
                "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
                "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n"
            )
            .as_bytes(),
        )
        .expect("write MCP requests");
    let output = child.wait_with_output().expect("wait latticed");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let responses = String::from_utf8(output.stdout)
        .expect("stdout utf8")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("JSON-RPC response"))
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["result"]["capabilities"], json!({"tools": {}}));
    let tools = responses[1]["result"]["tools"]
        .as_array()
        .expect("tool list");
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect::<Vec<_>>(),
        ["lattice_delivery_run", "lattice_delivery_status"]
    );
    for tool in tools {
        assert_eq!(
            tool["inputSchema"],
            json!({"type":"object","additionalProperties":false})
        );
    }
}
