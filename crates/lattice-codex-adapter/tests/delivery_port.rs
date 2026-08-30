use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use lattice_codex_adapter::{
    CODEX_HOME_CONFIG_BYTES, CODEX_HOME_OWNERSHIP_MARKER_BYTES, CODEX_HOME_OWNERSHIP_MARKER_NAME,
    CodexDeliveryAdapter, CodexDeliveryAdapterConfig, CodexIdentityExpectation,
};
use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, CodexDeliveryRequest, ContentDigest, DaemonEpoch, DeliveryProfile,
    DeliveryRunRequest, DeliveryRuntime, DurableIntentEvidence, FencingToken, HolderProcessId,
    Invocation, PreparedWorkspaceEvidence, ProjectId, ProjectSnapshotId, RequestId,
    RuntimeAdmissionMode, RuntimeKind, TaskId, WRITER_LEASE_PRODUCER_ID,
    WRITER_LEASE_PRODUCER_VERSION, WriterLeaseAuthorityHead, WriterLeaseAuthorityReceipt,
    WriterLeaseIdentity, WriterLeaseRevision, WriterLeaseStatus,
};
use lattice_ports::{DeliveryCodexPort, DeliveryFailureCertainty, PortErrorKind};
use sha2::{Digest, Sha256};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

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
        None,
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
        None,
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

#[test]
fn official_adapter_requires_a_runtime_verified_pinned_resource_binding() {
    let error = CodexDeliveryAdapterConfig::new(
        CodexIdentityExpectation::new(
            PathBuf::from(r"C:\pinned\bin\codex.exe"),
            "codex-cli 0.146.0",
            "1".repeat(64),
        ),
        PathBuf::from(r"C:\delivery\schema"),
        PathBuf::from(r"C:\delivery\codex-home"),
        "Apply the fixed TASK-032 delivery change.",
        Duration::from_secs(5),
        DeliveryRuntime::OfficialCodexAppServer,
        None,
    )
    .expect_err("official mode must fail closed without the pinned managed package");

    assert_eq!(error.code(), "CODEX_CONFIG_PINNED_RESOURCES_MISSING");
    assert_eq!(error.certainty(), DeliveryFailureCertainty::Known);
}

#[test]
fn governed_writer_fencing_head_changes_the_codex_output_digest() {
    let fixture = GovernedDigestFixture::new();
    let first_request = fixture.request(1);
    let second_request = fixture.request(2);
    assert_eq!(
        first_request
            .writer_authority()
            .expect("governed authority")
            .identity()
            .fencing_token()
            .get(),
        1
    );
    assert_eq!(
        second_request
            .writer_authority()
            .expect("governed authority")
            .identity()
            .fencing_token()
            .get(),
        2
    );
    let mut first_adapter = fixture.adapter("schema-fence-1");
    let mut second_adapter = fixture.adapter("schema-fence-2");

    let first = first_adapter
        .run_delivery(first_request)
        .expect("first governed scripted turn");
    let second = second_adapter
        .run_delivery(second_request)
        .expect("second governed scripted turn");

    assert_eq!(first.thread_id(), second.thread_id());
    assert_eq!(first.turn_id(), second.turn_id());
    assert_ne!(first.output_digest(), second.output_digest());
}

struct GovernedDigestFixture {
    root: PathBuf,
    launcher: PathBuf,
    launcher_sha256: String,
    codex_home: PathBuf,
    workspace: PathBuf,
}

impl GovernedDigestFixture {
    fn new() -> Self {
        let unique = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "lattice-codex-governed-digest-{}-{unique}",
            std::process::id()
        ));
        let codex_home = root.join("codex-home");
        let workspace = root.join("worktree");
        fs::create_dir_all(&codex_home).expect("create Codex home");
        fs::create_dir_all(&workspace).expect("create worktree");
        fs::write(
            codex_home.join(CODEX_HOME_OWNERSHIP_MARKER_NAME),
            CODEX_HOME_OWNERSHIP_MARKER_BYTES,
        )
        .expect("write Codex home marker");
        fs::write(codex_home.join("config.toml"), CODEX_HOME_CONFIG_BYTES)
            .expect("write exact keyring-only Codex config");
        let launcher = write_governed_launcher(&root, &codex_home);
        let launcher_sha256 = sha256(&fs::read(&launcher).expect("read launcher"));
        Self {
            root,
            launcher,
            launcher_sha256,
            codex_home,
            workspace,
        }
    }

    fn adapter(&self, schema_name: &str) -> CodexDeliveryAdapter {
        let config = CodexDeliveryAdapterConfig::new(
            CodexIdentityExpectation::new(
                self.launcher.clone(),
                "codex-cli 0.144.6",
                self.launcher_sha256.clone(),
            ),
            self.root.join(schema_name),
            self.codex_home.clone(),
            "Apply and verify the fixed governed delivery.",
            Duration::from_secs(20),
            DeliveryRuntime::ScriptedAcceptance,
            None,
        )
        .expect("governed adapter config");
        CodexDeliveryAdapter::new(config)
    }

    fn request(&self, fence: u64) -> CodexDeliveryRequest {
        let invocation = Invocation::new(
            CONTRACT_VERSION,
            RequestId::new("governed-delivery-request").expect("request"),
            TaskId::new("TASK-038").expect("task"),
            AttemptId::new("attempt-1").expect("attempt"),
            ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
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
            self.workspace.to_string_lossy(),
            "1".repeat(40),
            digest('d'),
        )
        .expect("workspace");
        CodexDeliveryRequest::new_governed(run, intent, workspace, writer_authority(fence))
            .expect("governed request")
    }
}

impl Drop for GovernedDigestFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn writer_authority(fence: u64) -> WriterLeaseAuthorityHead {
    let identity = WriterLeaseIdentity::new(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        TaskId::new("TASK-038").expect("task"),
        "1",
        digest('a'),
        AttemptId::new("attempt-1").expect("attempt"),
        "lease-1",
        "codex-writer-1",
        "worktree-1",
        HolderProcessId::new(42).expect("process"),
        digest('6'),
        "daemon-1",
        DaemonEpoch::new(1).expect("daemon epoch"),
        FencingToken::new(fence).expect("fence"),
    )
    .expect("writer identity");
    WriterLeaseAuthorityReceipt::new(
        CONTRACT_VERSION,
        WRITER_LEASE_PRODUCER_ID,
        WRITER_LEASE_PRODUCER_VERSION,
        RuntimeKind::Live,
        identity,
        WriterLeaseStatus::Active,
        WriterLeaseRevision::new(1).expect("revision"),
        RuntimeAdmissionMode::Active,
        "2026-08-09T00:00:00Z",
        "2026-08-09T00:00:30Z",
        "2026-08-09T00:05:00Z",
        digest('1'),
        digest('2'),
        digest('3'),
        digest('4'),
    )
    .expect("writer receipt")
    .head()
}

#[cfg(windows)]
fn write_governed_launcher(root: &Path, codex_home: &Path) -> PathBuf {
    let server = root.join("governed-app-server.ps1");
    let configured_home = codex_home.display().to_string().replace('\'', "''");
    let reported_home = codex_home.display().to_string().replace('\\', r"\\");
    let script = format!(
        r#"param([Parameter(ValueFromRemainingArguments=$true)][string[]]$RemainingArgs)
$ErrorActionPreference = 'Stop'
if ($RemainingArgs.Count -eq 1 -and $RemainingArgs[0] -eq '--version') {{
    [Console]::Out.WriteLine('codex-cli 0.144.6')
    exit 0
}}
if ($RemainingArgs.Count -ge 4 -and $RemainingArgs[0] -eq 'app-server' -and $RemainingArgs[1] -eq 'generate-json-schema') {{
    $schemaOutput = $RemainingArgs[$RemainingArgs.Count - 1]
    [IO.Directory]::CreateDirectory($schemaOutput) | Out-Null
    [IO.File]::WriteAllText((Join-Path $schemaOutput 'schema.json'), '{{"type":"object"}}')
    exit 0
}}
if ($RemainingArgs.Count -lt 1 -or $RemainingArgs[0] -ne 'app-server') {{ exit 11 }}
if ($env:CODEX_HOME -ne '{configured_home}') {{ exit 41 }}
$null = [Console]::In.ReadLine()
[Console]::Out.WriteLine('{{"id":0,"result":{{"userAgent":"codex_cli_rs/0.144.6","platformFamily":"windows","platformOs":"windows","codexHome":"{reported_home}"}}}}')
$null = [Console]::In.ReadLine()
$null = [Console]::In.ReadLine()
[Console]::Out.WriteLine('{{"id":1,"result":{{"thread":{{"id":"thread-governed"}}}}}}')
$null = [Console]::In.ReadLine()
[Console]::Out.WriteLine('{{"id":2,"result":{{"turn":{{"id":"turn-governed"}}}}}}')
[Console]::Out.WriteLine('{{"method":"turn/started","params":{{"threadId":"thread-governed","turn":{{"id":"turn-governed","status":"inProgress"}}}}}}')
[Console]::Out.WriteLine('{{"method":"item/completed","params":{{"threadId":"thread-governed","turnId":"turn-governed","item":{{"arguments":{{"command":"controlled apply"}},"contentItems":[{{"text":"Script completed\nExit code: 0","type":"inputText"}}],"id":"tool-apply","status":"completed","success":true,"tool":"exec","type":"dynamicToolCall"}},"completedAtMs":1}}}}')
[Console]::Out.WriteLine('{{"method":"item/completed","params":{{"threadId":"thread-governed","turnId":"turn-governed","item":{{"arguments":{{"command":"controlled verify"}},"contentItems":[{{"text":"Script completed\nExit code: 0","type":"inputText"}}],"id":"tool-verify","status":"completed","success":true,"tool":"exec","type":"dynamicToolCall"}},"completedAtMs":2}}}}')
[Console]::Out.WriteLine('{{"method":"turn/completed","params":{{"threadId":"thread-governed","turn":{{"id":"turn-governed","items":[{{"id":"agent-final","text":"Delivery complete.","type":"agentMessage"}}],"itemsView":"summary","status":"completed","error":null}}}}}}')
Start-Sleep -Seconds 60
"#
    );
    fs::write(&server, script).expect("write fake governed app-server");
    let launcher = root.join("fake-governed-codex.cmd");
    fs::write(
        &launcher,
        concat!(
            "@echo off\r\n",
            "if \"%~1\"==\"--version\" (\r\n",
            "  echo codex-cli 0.144.6\r\n",
            "  exit /b 0\r\n",
            ")\r\n",
            "if \"%~1\"==\"app-server\" if \"%~2\"==\"generate-json-schema\" if \"%~3\"==\"--out\" (\r\n",
            "  mkdir \"%~4\"\r\n",
            "  >\"%~4\\schema.json\" echo {\"type\":\"object\"}\r\n",
            "  exit /b 0\r\n",
            ")\r\n",
            "\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"%~dp0governed-app-server.ps1\" %*\r\n",
        ),
    )
    .expect("write fake governed launcher");
    launcher
}

#[cfg(unix)]
fn write_governed_launcher(root: &Path, codex_home: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let launcher = root.join("fake-governed-codex");
    let script = format!(
        r#"#!/bin/sh
if [ "$#" -eq 1 ] && [ "$1" = "--version" ]; then
  printf '%s\n' 'codex-cli 0.144.6'
  exit 0
fi
if [ "$#" -eq 4 ] && [ "$1" = "app-server" ] && [ "$2" = "generate-json-schema" ] && [ "$3" = "--out" ]; then
  mkdir -p "$4"
  printf '%s\n' '{{"type":"object"}}' > "$4/schema.json"
  exit 0
fi
if [ "$#" -lt 1 ] || [ "$1" != "app-server" ]; then exit 11; fi
if [ "$CODEX_HOME" != "{}" ]; then exit 41; fi
read -r _
printf '%s\n' '{{"id":0,"result":{{"userAgent":"codex_cli_rs/0.144.6","platformFamily":"unix","platformOs":"linux","codexHome":"{}"}}}}'
read -r _
read -r _
printf '%s\n' '{{"id":1,"result":{{"thread":{{"id":"thread-governed"}}}}}}'
read -r _
printf '%s\n' '{{"id":2,"result":{{"turn":{{"id":"turn-governed"}}}}}}'
printf '%s\n' '{{"method":"turn/started","params":{{"threadId":"thread-governed","turn":{{"id":"turn-governed","status":"inProgress"}}}}}}'
printf '%s\n' '{{"method":"item/completed","params":{{"threadId":"thread-governed","turnId":"turn-governed","item":{{"arguments":{{"command":"controlled apply"}},"contentItems":[{{"text":"Script completed\\nExit code: 0","type":"inputText"}}],"id":"tool-apply","status":"completed","success":true,"tool":"exec","type":"dynamicToolCall"}},"completedAtMs":1}}}}'
printf '%s\n' '{{"method":"item/completed","params":{{"threadId":"thread-governed","turnId":"turn-governed","item":{{"arguments":{{"command":"controlled verify"}},"contentItems":[{{"text":"Script completed\\nExit code: 0","type":"inputText"}}],"id":"tool-verify","status":"completed","success":true,"tool":"exec","type":"dynamicToolCall"}},"completedAtMs":2}}}}'
printf '%s\n' '{{"method":"turn/completed","params":{{"threadId":"thread-governed","turn":{{"id":"turn-governed","items":[{{"id":"agent-final","text":"Delivery complete.","type":"agentMessage"}}],"itemsView":"summary","status":"completed","error":null}}}}}}'
sleep 60
"#,
        codex_home.display(),
        codex_home.display(),
    );
    fs::write(&launcher, script).expect("write fake governed launcher");
    let mut permissions = fs::metadata(&launcher)
        .expect("launcher metadata")
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&launcher, permissions).expect("make launcher executable");
    launcher
}

fn sha256(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let mut output = String::with_capacity(64);
    for byte in hasher.finalize() {
        write!(&mut output, "{byte:02x}").expect("write digest");
    }
    output
}
