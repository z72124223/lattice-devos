use std::path::PathBuf;
use std::time::Duration;

use lattice_codex_adapter::{
    AppServerRunConfig, AppServerRunErrorKind, CODEX_HOME_CONFIG_BYTES,
    CODEX_HOME_OWNERSHIP_MARKER_BYTES, CODEX_HOME_OWNERSHIP_MARKER_NAME,
    PinnedCodexResourceDigests, PinnedCodexResources, TurnStatus,
};

#[cfg(windows)]
use std::fs;
#[cfg(windows)]
use std::io::Read;
#[cfg(windows)]
use std::path::Path;
#[cfg(windows)]
use std::process::Command;
#[cfg(windows)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(windows)]
use std::time::Instant;

#[cfg(windows)]
use lattice_codex_adapter::{
    ManagedCodexSpawnIdentity, SupervisedDuplexChild, run_codex_app_server,
    run_codex_app_server_until,
};
#[cfg(windows)]
use sha2::{Digest, Sha256};

#[cfg(windows)]
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);
#[cfg(windows)]
const ENVIRONMENT_CONTROLLER: &str = "LATTICE_OWNED_ENVIRONMENT_CONTROLLER";
#[cfg(windows)]
const ENVIRONMENT_LEAF: &str = "LATTICE_OWNED_ENVIRONMENT_LEAF";
#[cfg(windows)]
const ENVIRONMENT_HOSTILE: &str = "LATTICE_OWNED_ENVIRONMENT_HOSTILE";
#[cfg(windows)]
const ENVIRONMENT_ALLOWED: &str = "LATTICE_OWNED_ENVIRONMENT_ALLOWED";
const DESCENDANT_DEADLINE_TRIGGER_NAME: &str = "descendant-deadline-trigger.txt";
const DESCENDANT_TRIGGER_NAME: &str = "descendant-trigger.txt";

#[cfg(windows)]
#[test]
fn owned_child_environment_modes_are_enforced_by_the_spawned_process() {
    let status = Command::new(std::env::current_exe().expect("current test executable"))
        .args([
            "--exact",
            "owned_child_environment_controller",
            "--nocapture",
        ])
        .env(ENVIRONMENT_CONTROLLER, "1")
        .env(ENVIRONMENT_HOSTILE, "parent-hostile-value")
        .status()
        .expect("spawn isolated environment controller");
    assert!(status.success(), "environment controller failed: {status}");
}

#[cfg(windows)]
#[test]
fn owned_child_environment_controller() {
    if std::env::var_os(ENVIRONMENT_CONTROLLER).is_none() {
        return;
    }

    let executable = std::env::current_exe().expect("current test executable");
    let leaf = || {
        let mut command = Command::new(&executable);
        command
            .args(["--exact", "owned_child_environment_leaf", "--nocapture"])
            .env(ENVIRONMENT_LEAF, "1");
        command
    };

    let mut cleared = leaf();
    cleared
        .env_clear()
        .env(ENVIRONMENT_LEAF, "1")
        .env(ENVIRONMENT_ALLOWED, "explicit-allowlist-value");
    let cleared = run_environment_leaf(cleared, true);
    assert!(
        cleared.contains("hostile=<missing>;allowed=explicit-allowlist-value"),
        "cleared child environment was not exact: {cleared:?}"
    );

    let inherited = run_environment_leaf(leaf(), false);
    assert!(
        inherited.contains("hostile=parent-hostile-value;allowed=<missing>"),
        "inherited child environment lost its parent value: {inherited:?}"
    );

    let mut removed = leaf();
    removed.env_remove(ENVIRONMENT_HOSTILE);
    let removed = run_environment_leaf(removed, false);
    assert!(
        removed.contains("hostile=<missing>;allowed=<missing>"),
        "explicit removal did not override inheritance: {removed:?}"
    );
}

#[cfg(windows)]
#[test]
fn owned_child_environment_leaf() {
    if std::env::var_os(ENVIRONMENT_LEAF).is_none() {
        return;
    }
    let hostile = std::env::var(ENVIRONMENT_HOSTILE).unwrap_or_else(|_| "<missing>".to_owned());
    let allowed = std::env::var(ENVIRONMENT_ALLOWED).unwrap_or_else(|_| "<missing>".to_owned());
    println!("hostile={hostile};allowed={allowed}");
}

#[cfg(windows)]
fn run_environment_leaf(mut command: Command, cleared: bool) -> String {
    let child = if cleared {
        SupervisedDuplexChild::spawn_cleared(&mut command)
    } else {
        SupervisedDuplexChild::spawn(&mut command)
    };
    let mut child = child.expect("spawn owned environment leaf");
    drop(child.take_stdin());
    let mut stdout = child.take_stdout().expect("owned environment leaf stdout");
    let mut output = String::new();
    stdout
        .read_to_string(&mut output)
        .expect("read owned environment leaf stdout");
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll owned environment leaf") {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "owned environment leaf timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    child
        .terminate_and_reap()
        .expect("reap owned environment leaf subtree");
    assert!(status.success(), "owned environment leaf failed: {status}");
    output
}

#[test]
fn pinned_resource_binding_requires_an_absolute_directory_and_exact_digests() {
    let error = PinnedCodexResources::new(
        PathBuf::from("managed-package"),
        PathBuf::from("codex-resources"),
        PinnedCodexResourceDigests::new(
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64),
            "e".repeat(64),
            "f".repeat(64),
        )
        .expect("valid digest bundle"),
    )
    .expect_err("a caller-relative resource directory must fail closed");

    assert_eq!(error.kind(), AppServerRunErrorKind::InvalidPinnedResources);
}

#[test]
fn run_config_requires_exact_absolute_inputs_and_a_bounded_timeout() {
    assert_eq!(
        AppServerRunConfig::new(
            PathBuf::from(r"C:\tools\codex.exe"),
            "A".repeat(64),
            PathBuf::from(r"C:\codex-home"),
            PathBuf::from(r"C:\fixture"),
            "create answer.txt",
            Duration::from_secs(30),
            None,
        )
        .expect_err("launcher digest must be canonical")
        .kind(),
        AppServerRunErrorKind::InvalidLauncherSha256
    );

    assert_eq!(
        AppServerRunConfig::new(
            PathBuf::from(r"C:\tools\codex.exe"),
            "a".repeat(64),
            PathBuf::from("codex-home"),
            PathBuf::from(r"C:\fixture"),
            "create answer.txt",
            Duration::from_secs(30),
            None,
        )
        .expect_err("Codex home must be absolute")
        .kind(),
        AppServerRunErrorKind::InvalidCodexHome
    );

    assert_eq!(
        AppServerRunConfig::new(
            PathBuf::from("codex.exe"),
            "a".repeat(64),
            PathBuf::from(r"C:\codex-home"),
            PathBuf::from(r"C:\fixture"),
            "create answer.txt",
            Duration::from_secs(30),
            None,
        )
        .expect_err("launcher must be absolute")
        .kind(),
        AppServerRunErrorKind::InvalidLauncher
    );

    assert_eq!(
        AppServerRunConfig::new(
            PathBuf::from(r"C:\tools\codex.exe"),
            "a".repeat(64),
            PathBuf::from(r"C:\codex-home"),
            PathBuf::from("fixture"),
            "create answer.txt",
            Duration::from_secs(30),
            None,
        )
        .expect_err("working directory must be absolute")
        .kind(),
        AppServerRunErrorKind::InvalidWorkingDirectory
    );

    assert_eq!(
        AppServerRunConfig::new(
            PathBuf::from(r"C:\tools\codex.exe"),
            "a".repeat(64),
            PathBuf::from(r"C:\codex-home"),
            PathBuf::from(r"C:\fixture"),
            "   ",
            Duration::from_secs(30),
            None,
        )
        .expect_err("prompt must not be blank")
        .kind(),
        AppServerRunErrorKind::InvalidPrompt
    );

    assert_eq!(
        AppServerRunConfig::new(
            PathBuf::from(r"C:\tools\codex.exe"),
            "a".repeat(64),
            PathBuf::from(r"C:\codex-home"),
            PathBuf::from(r"C:\fixture"),
            "create answer.txt",
            Duration::ZERO,
            None,
        )
        .expect_err("timeout must be positive")
        .kind(),
        AppServerRunErrorKind::InvalidTimeout
    );
}

#[test]
fn run_config_preserves_the_exact_task_binding() {
    let config = AppServerRunConfig::new(
        PathBuf::from(r"C:\tools\codex.exe"),
        "a".repeat(64),
        PathBuf::from(r"C:\codex-home"),
        PathBuf::from(r"C:\fixture"),
        "create answer.txt",
        Duration::from_secs(90),
        None,
    )
    .expect("valid bounded run config");

    assert_eq!(config.launcher(), PathBuf::from(r"C:\tools\codex.exe"));
    assert_eq!(config.expected_launcher_sha256(), "a".repeat(64));
    assert_eq!(config.codex_home(), PathBuf::from(r"C:\codex-home"));
    assert_eq!(config.working_directory(), PathBuf::from(r"C:\fixture"));
    assert_eq!(config.prompt(), "create answer.txt");
    assert_eq!(config.timeout(), Duration::from_secs(90));
}

#[cfg(windows)]
#[derive(Clone, Copy)]
enum FakeMode {
    Success,
    Yielded,
    Malformed,
    Eof,
    Timeout,
    WrongHome,
    Premature,
    Orphan,
}

#[cfg(windows)]
struct ProcessFixture {
    root: PathBuf,
    launcher: PathBuf,
    codex_home: PathBuf,
    working_directory: PathBuf,
    effect_log: PathBuf,
    descendant_pid_log: PathBuf,
    descendant_trigger: PathBuf,
    descendant_effect_log: PathBuf,
    launcher_sha256: String,
}

#[cfg(windows)]
impl ProcessFixture {
    fn new(mode: FakeMode) -> Self {
        let unique = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "lattice-codex-process-{}-{unique}",
            std::process::id()
        ));
        let codex_home = root.join("codex-home");
        let working_directory = root.join("worktree");
        fs::create_dir_all(&codex_home).expect("create dedicated Codex home");
        fs::create_dir_all(&working_directory).expect("create isolated worktree fixture");
        fs::write(
            codex_home.join(CODEX_HOME_OWNERSHIP_MARKER_NAME),
            CODEX_HOME_OWNERSHIP_MARKER_BYTES,
        )
        .expect("mark dedicated Codex home as LATTICE-owned");
        fs::write(codex_home.join("config.toml"), CODEX_HOME_CONFIG_BYTES)
            .expect("write exact keyring-only fixture configuration");
        let effect_log = root.join("thread-started.txt");
        let descendant_pid_log = root.join("descendant.pid");
        let descendant_trigger = root.join(DESCENDANT_TRIGGER_NAME);
        let descendant_effect_log = root.join("descendant-effect.txt");
        fs::write(root.join("native-process-mode.txt"), native_mode(mode))
            .expect("write native process-fault mode");
        let launcher = native_fixture_helper();
        wait_for_native_fixture_helper(&launcher);
        let launcher_sha256 = sha256(&fs::read(&launcher).expect("read scripted app-server"));
        Self {
            root,
            launcher,
            codex_home,
            working_directory,
            effect_log,
            descendant_pid_log,
            descendant_trigger,
            descendant_effect_log,
            launcher_sha256,
        }
    }

    fn config(&self, timeout: Duration) -> AppServerRunConfig {
        AppServerRunConfig::new(
            self.launcher.clone(),
            self.launcher_sha256.clone(),
            self.codex_home.clone(),
            self.working_directory.clone(),
            "Create answer.txt",
            timeout,
            None,
        )
        .expect("valid scripted app-server config")
    }
}

#[cfg(windows)]
#[test]
fn managed_bridge_identity_rechecks_owned_home_and_rejects_external_connector_config() {
    let fixture = ProcessFixture::new(FakeMode::Success);
    let identity = ManagedCodexSpawnIdentity::capture(
        fixture.launcher.clone(),
        &fixture.codex_home,
        &fixture.working_directory,
    )
    .expect("capture exact managed bridge identity");
    assert_eq!(
        identity.launcher(),
        fs::canonicalize(&fixture.launcher).expect("canonical fixture launcher")
    );
    assert_eq!(identity.launcher_sha256(), fixture.launcher_sha256);
    fs::write(
        fixture.codex_home.join("config.toml"),
        b"approval_policy = \"never\"\n[mcp_servers.untrusted]\ncommand = \"outside.exe\"\n",
    )
    .expect("tamper managed home with external connector");
    assert_eq!(
        identity
            .verify(&fixture.codex_home, &fixture.working_directory)
            .expect_err("external connector configuration is not managed state")
            .kind(),
        AppServerRunErrorKind::InvalidCodexHome
    );
}

#[cfg(windows)]
#[test]
fn managed_identity_accepts_only_a_keyring_home_without_plaintext_auth() {
    let fixture = ProcessFixture::new(FakeMode::Success);

    let identity = ManagedCodexSpawnIdentity::capture(
        fixture.launcher.clone(),
        &fixture.codex_home,
        &fixture.working_directory,
    )
    .expect("keyring-only managed home must be admissible");
    assert!(
        identity
            .codex_home_digest()
            .starts_with("codex-home:sha256:")
    );
    assert_eq!(identity.codex_home_digest().len(), 82);
    assert!(identity.config_digest().starts_with("codex-config:sha256:"));
    assert_eq!(identity.config_digest().len(), 84);
}

#[test]
fn managed_codex_config_is_keyring_only_with_a_closed_shell_environment() {
    let config = std::str::from_utf8(CODEX_HOME_CONFIG_BYTES).expect("managed config is UTF-8");
    assert!(config.starts_with("cli_auth_credentials_store = \"keyring\"\n"));
    assert!(config.contains("[shell_environment_policy]\ninherit = \"all\"\n"));
    assert!(config.contains("ignore_default_excludes = false\n"));
    assert!(config.contains(concat!(
        "include_only = [\"SystemRoot\", \"WINDIR\", \"ComSpec\", \"PATH\", ",
        "\"PATHEXT\", \"PROCESSOR_ARCHITECTURE\", \"NUMBER_OF_PROCESSORS\", ",
        "\"TEMP\", \"TMP\", \"LANG\", \"LC_ALL\"]\n",
    )));
    for forbidden in [
        "CODEX_HOME",
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "OPENAI_API_KEY",
        "CODEX_ACCESS_TOKEN",
        "DATABASE_URL",
    ] {
        assert!(
            !config
                .lines()
                .any(|line| line.starts_with("include_only") && line.contains(forbidden)),
            "managed Codex shell environment admitted {forbidden}"
        );
    }
}

#[test]
fn official_runtime_home_provisioning_never_reads_or_copies_plaintext_auth() {
    let source =
        include_str!("../../../scripts/start-lattice-runtime-postgres.ps1").replace("\r\n", "\n");
    let provisioning = source
        .split("function Initialize-LatticeRuntimeCodexHome {")
        .nth(1)
        .expect("runtime Codex-home provisioning function")
        .split("function Start-LatticePostgres {")
        .next()
        .expect("bounded provisioning function body");

    assert!(provisioning.contains("'cli_auth_credentials_store = \"keyring\"'"));
    assert!(provisioning.contains("'[shell_environment_policy]'"));
    assert!(provisioning.contains("throw 'LATTICE_RUNTIME_CODEX_PLAINTEXT_AUTH_DENIED'"));
    assert!(provisioning.contains("[IO.File]::ReadAllBytes($configPath)"));
    assert!(!provisioning.contains("$authSource"));
    assert!(!provisioning.contains("[IO.File]::Copy"));
    assert!(!provisioning.contains("[IO.File]::ReadAllBytes($authPath)"));
    assert!(source.contains("'LATTICE\\runtime-codex-home-keyring-v1'"));
}

#[test]
fn delivery_runner_uses_the_same_keyring_only_home_contract() {
    let source = include_str!("../../../scripts/run-lattice-delivery.ps1").replace("\r\n", "\n");
    let config = std::str::from_utf8(CODEX_HOME_CONFIG_BYTES).expect("managed config is UTF-8");
    for line in config.lines().filter(|line| !line.is_empty()) {
        assert!(
            source.contains(&format!("    '{line}'")),
            "delivery runner is missing exact managed config line: {line}"
        );
    }
    assert!(source.contains("throw 'LATTICE_DELIVERY_PLAINTEXT_CODEX_AUTH_DENIED'"));
    assert!(source.contains("auth_present = $false"));
    assert!(!source.contains("(Join-Path $codexHome 'auth.json')"));
}

#[cfg(windows)]
#[test]
fn managed_identity_rejects_any_plaintext_auth_file_before_spawn() {
    let fixture = ProcessFixture::new(FakeMode::Success);
    fs::write(
        fixture.codex_home.join("auth.json"),
        b"fixture-must-not-be-read",
    )
    .expect("write forbidden plaintext auth marker");

    let error = ManagedCodexSpawnIdentity::capture(
        fixture.launcher.clone(),
        &fixture.codex_home,
        &fixture.working_directory,
    )
    .expect_err("auth.json presence must be denied without reading its contents");
    assert_eq!(
        error.kind(),
        AppServerRunErrorKind::PlaintextCodexAuthDenied
    );
    assert!(!fixture.effect_log.exists());
}

#[cfg(windows)]
fn native_mode(mode: FakeMode) -> &'static [u8] {
    match mode {
        FakeMode::Success => b"success\n",
        FakeMode::Yielded => b"yielded\n",
        FakeMode::Malformed => b"malformed\n",
        FakeMode::Eof => b"eof\n",
        FakeMode::Timeout => b"timeout\n",
        FakeMode::WrongHome => b"wrong-home\n",
        FakeMode::Premature => b"premature\n",
        FakeMode::Orphan => b"orphan\n",
    }
}

#[cfg(windows)]
fn native_fixture_helper() -> PathBuf {
    PathBuf::from(
        std::env::var("CARGO_BIN_EXE_lattice-codex-test-app-server")
            .expect("Cargo must provide the native test app-server binary path"),
    )
}

#[cfg(windows)]
fn wait_for_native_fixture_helper(helper: &Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if Command::new(helper)
            .args(["app-server", "--listen", "stdio://"])
            .status()
            .is_ok()
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "native test app-server did not become executable: {helper:?}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
impl Drop for ProcessFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(windows)]
struct PinnedProcessFixture {
    root: PathBuf,
    launcher: PathBuf,
    codex_home: PathBuf,
    working_directory: PathBuf,
    resources: PinnedCodexResources,
    sandbox_setup: PathBuf,
    command_runner: PathBuf,
    code_mode_host: PathBuf,
    rg: PathBuf,
    sandbox_preflight_mode: PathBuf,
    sandbox_preflight_log: PathBuf,
    sandbox_preflight_descendant_pid_log: PathBuf,
    sandbox_preflight_descendant_effect_log: PathBuf,
    readiness_mode: PathBuf,
    readiness_descendant_pid_log: PathBuf,
    readiness_descendant_effect_log: PathBuf,
    turn_start_log: PathBuf,
    sandbox_temp: PathBuf,
    launch_log: PathBuf,
    launcher_sha256: String,
}

#[cfg(windows)]
struct PinnedServerScript<'a> {
    launch_log: &'a Path,
    sandbox_preflight_log: &'a Path,
    managed_root: &'a Path,
    resources_directory: &'a Path,
    sandbox_temp: &'a Path,
    sandbox_setup: &'a Path,
    codex_home: &'a Path,
    readiness_mode: &'a Path,
    readiness_descendant_pid_log: &'a Path,
    readiness_descendant_effect_log: &'a Path,
    turn_start_log: &'a Path,
}

#[cfg(windows)]
fn pinned_app_server_script(paths: &PinnedServerScript<'_>) -> String {
    let quote = |path: &Path| path.display().to_string().replace('\'', "''");
    let reported_home = paths.codex_home.display().to_string().replace('\\', r"\\");
    format!(
        "$ErrorActionPreference = 'Stop'\n$log = '{}'\nif ([IO.File]::ReadAllText('{}') -ne 'completed') {{ exit 57 }}\nif ($args.Count -ne 3 -or $args[0] -ne 'app-server' -or $args[1] -ne '--listen' -or $args[2] -ne 'stdio://') {{ exit 58 }}\n[IO.File]::WriteAllText($log, 'spawned')\nif ($env:CODEX_HOME -ne '{}') {{ [IO.File]::WriteAllText($log, 'home:' + $env:CODEX_HOME); exit 59 }}\nif ($env:CODEX_MANAGED_PACKAGE_ROOT -ne '{}') {{ [IO.File]::WriteAllText($log, 'root:' + $env:CODEX_MANAGED_PACKAGE_ROOT); exit 51 }}\n$first = ($env:PATH -split ';')[0]\nif ($first -ne '{}') {{ [IO.File]::WriteAllText($log, 'path:' + $first); exit 52 }}\nif ($env:TMP -ne '{}' -or $env:TEMP -ne '{}' -or $env:TMPDIR -ne '{}') {{ [IO.File]::WriteAllText($log, 'temp'); exit 56 }}\n$matches = @(Get-Command 'codex-windows-sandbox-setup.exe' -CommandType Application -ErrorAction Stop)\nif ($matches.Count -ne 1) {{ [IO.File]::WriteAllText($log, 'matches:' + $matches.Count); exit 54 }}\n$resolved = $matches[0].Source\nif ($resolved -ne '{}') {{ [IO.File]::WriteAllText($log, 'resolved:' + $resolved); exit 53 }}\n[IO.File]::WriteAllText($log, 'verified')\n$null = [Console]::In.ReadLine()\n[Console]::Out.WriteLine('{{\"id\":0,\"result\":{{\"userAgent\":\"codex_cli_rs/0.146.0\",\"platformFamily\":\"windows\",\"platformOs\":\"windows\",\"codexHome\":\"{}\"}}}}')\n$null = [Console]::In.ReadLine()\n$readiness = [Console]::In.ReadLine() | ConvertFrom-Json\nif ($readiness.method -ne 'windowsSandbox/readiness' -or $readiness.id -ne 3) {{ exit 55 }}\n$mode = [IO.File]::ReadAllText('{}').Trim()\nif ($mode -eq 'hang') {{\n  Start-Sleep -Milliseconds 250\n  $grandchild = \"Start-Sleep -Seconds 31; [IO.File]::WriteAllText('{}', 'survived')\"\n  $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($grandchild))\n  $descendant = Start-Process -FilePath \"$PSHOME\\powershell.exe\" -WindowStyle Hidden -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-EncodedCommand',$encoded) -PassThru\n  [IO.File]::WriteAllText('{}', [string]$descendant.Id)\n  Start-Sleep -Seconds 60\n}}\nif ($mode -eq 'update-required') {{\n  [Console]::Out.WriteLine('{{\"id\":3,\"result\":{{\"status\":\"updateRequired\"}}}}')\n  Start-Sleep -Seconds 60\n}}\n[Console]::Out.WriteLine('{{\"id\":3,\"result\":{{\"status\":\"ready\"}}}}')\n$threadStart = [Console]::In.ReadLine()\n[IO.File]::WriteAllText('{}', $threadStart)\n[Console]::Out.WriteLine('{{\"id\":1,\"result\":{{\"thread\":{{\"id\":\"thread-pinned\"}}}}}}')\n$null = [Console]::In.ReadLine()\n[Console]::Out.WriteLine('{{\"id\":2,\"result\":{{\"turn\":{{\"id\":\"turn-pinned\"}}}}}}')\n[Console]::Out.WriteLine('{{\"method\":\"turn/started\",\"params\":{{\"threadId\":\"thread-pinned\",\"turn\":{{\"id\":\"turn-pinned\",\"status\":\"inProgress\"}}}}}}')\n[Console]::Out.WriteLine('{{\"method\":\"item/completed\",\"params\":{{\"threadId\":\"thread-pinned\",\"turnId\":\"turn-pinned\",\"item\":{{\"arguments\":{{\"command\":\"apply fixture\"}},\"id\":\"tool-apply\",\"status\":\"completed\",\"success\":true,\"tool\":\"exec\",\"type\":\"dynamicToolCall\"}},\"completedAtMs\":1}}}}')\n[Console]::Out.WriteLine('{{\"method\":\"item/completed\",\"params\":{{\"threadId\":\"thread-pinned\",\"turnId\":\"turn-pinned\",\"item\":{{\"arguments\":{{\"command\":\"verify fixture\"}},\"id\":\"tool-verify\",\"status\":\"completed\",\"success\":true,\"tool\":\"exec\",\"type\":\"dynamicToolCall\"}},\"completedAtMs\":2}}}}')\n[Console]::Out.WriteLine('{{\"method\":\"turn/completed\",\"params\":{{\"threadId\":\"thread-pinned\",\"turn\":{{\"id\":\"turn-pinned\",\"items\":[{{\"id\":\"agent-final\",\"text\":\"Delivery complete.\",\"type\":\"agentMessage\"}}],\"itemsView\":\"summary\",\"status\":\"completed\",\"error\":null}}}}}}')\nStart-Sleep -Seconds 60\n",
        quote(paths.launch_log),
        quote(paths.sandbox_preflight_log),
        quote(paths.codex_home),
        quote(paths.managed_root),
        quote(paths.resources_directory),
        quote(paths.sandbox_temp),
        quote(paths.sandbox_temp),
        quote(paths.sandbox_temp),
        quote(paths.sandbox_setup),
        reported_home,
        quote(paths.readiness_mode),
        quote(paths.readiness_descendant_effect_log),
        quote(paths.readiness_descendant_pid_log),
        quote(paths.turn_start_log),
    )
    .replace(
        r#""status":"completed","success":true,"tool":"exec""#,
        r#""contentItems":[{"text":"Script completed\nExit code: 0","type":"inputText"}],"status":"completed","success":true,"tool":"exec""#,
    )
}

#[cfg(windows)]
struct PinnedSandboxPreflightScript<'a> {
    log: &'a Path,
    mode: &'a Path,
    managed_root: &'a Path,
    resources_directory: &'a Path,
    sandbox_setup: &'a Path,
    sandbox_temp: &'a Path,
    codex_home: &'a Path,
    working_directory: &'a Path,
    descendant_pid_log: &'a Path,
    descendant_effect_log: &'a Path,
}

#[cfg(windows)]
fn pinned_sandbox_preflight_script(paths: &PinnedSandboxPreflightScript<'_>) -> String {
    let quote = |path: &Path| path.display().to_string().replace('\'', "''");
    format!(
        "$ErrorActionPreference = 'Stop'\n$log = '{}'\nif ($args.Count -ne 9 -or $args[0] -ne 'sandbox' -or $args[1] -ne '-P' -or $args[2] -ne ':workspace' -or $args[3] -ne '-C' -or $args[4] -ne '{}' -or $args[5] -ne 'cmd.exe' -or $args[6] -ne '/d' -or $args[7] -ne '/c' -or $args[8] -ne 'exit 0') {{ [IO.File]::WriteAllText($log, 'args'); exit 61 }}\nif ((Get-Location).Path -ne '{}') {{ [IO.File]::WriteAllText($log, 'cwd'); exit 62 }}\nif ($env:CODEX_HOME -ne '{}' -or $env:CODEX_MANAGED_PACKAGE_ROOT -ne '{}') {{ [IO.File]::WriteAllText($log, 'identity'); exit 63 }}\n$first = ($env:PATH -split ';')[0]\nif ($first -ne '{}') {{ [IO.File]::WriteAllText($log, 'path'); exit 64 }}\nif ($env:TMP -ne '{}' -or $env:TEMP -ne '{}' -or $env:TMPDIR -ne '{}') {{ [IO.File]::WriteAllText($log, 'temp'); exit 65 }}\n$matches = @(Get-Command 'codex-windows-sandbox-setup.exe' -CommandType Application -ErrorAction Stop)\nif ($matches.Count -ne 1 -or $matches[0].Source -ne '{}') {{ [IO.File]::WriteAllText($log, 'helper'); exit 66 }}\n[IO.File]::WriteAllText($log, 'started')\n$mode = [IO.File]::ReadAllText('{}').Trim()\nif ($mode -eq 'hang') {{\n  Start-Sleep -Milliseconds 250\n  $grandchild = \"Start-Sleep -Seconds 61; [IO.File]::WriteAllText('{}', 'survived')\"\n  $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($grandchild))\n  $descendant = Start-Process -FilePath \"$PSHOME\\powershell.exe\" -WindowStyle Hidden -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-EncodedCommand',$encoded) -PassThru\n  [IO.File]::WriteAllText('{}', [string]$descendant.Id)\n  Start-Sleep -Seconds 70\n}}\nif ($mode -eq 'fail') {{ exit 67 }}\nif ($mode -eq 'tamper-after-preflight') {{\n  Start-Sleep -Milliseconds 250\n  [IO.File]::WriteAllText($matches[0].Source, 'tampered')\n  [IO.File]::WriteAllText($log, 'completed')\n  exit 0\n}}\nif ($mode -ne 'ready') {{ exit 68 }}\n[IO.File]::WriteAllText($log, 'completed')\nexit 0\n",
        quote(paths.log),
        quote(paths.working_directory),
        quote(paths.working_directory),
        quote(paths.codex_home),
        quote(paths.managed_root),
        quote(paths.resources_directory),
        quote(paths.sandbox_temp),
        quote(paths.sandbox_temp),
        quote(paths.sandbox_temp),
        quote(paths.sandbox_setup),
        quote(paths.mode),
        quote(paths.descendant_effect_log),
        quote(paths.descendant_pid_log),
    )
}

#[cfg(windows)]
fn write_pinned_codex_home(codex_home: &Path) {
    fs::write(
        codex_home.join(CODEX_HOME_OWNERSHIP_MARKER_NAME),
        CODEX_HOME_OWNERSHIP_MARKER_BYTES,
    )
    .expect("write Codex home marker");
    fs::write(codex_home.join("config.toml"), CODEX_HOME_CONFIG_BYTES)
        .expect("write exact keyring-only fixture config");
}

#[cfg(windows)]
struct PinnedFixtureResources {
    binding: PinnedCodexResources,
    sandbox_setup: PathBuf,
    command_runner: PathBuf,
    code_mode_host: PathBuf,
    rg: PathBuf,
}

#[cfg(windows)]
fn write_pinned_resources(
    managed_root: &Path,
    bundle_root: &Path,
    bin: &Path,
    resources_directory: &Path,
    codex_path_directory: &Path,
) -> PinnedFixtureResources {
    let sandbox_setup = resources_directory.join("codex-windows-sandbox-setup.exe");
    let command_runner = resources_directory.join("codex-command-runner.exe");
    let package_manifest = bundle_root.join("codex-package.json");
    let managed_package_manifest = managed_root.join("package.json");
    let code_mode_host = bin.join("codex-code-mode-host.exe");
    let rg = codex_path_directory.join("rg.exe");
    fs::write(&sandbox_setup, b"pinned sandbox setup").expect("write pinned setup helper");
    fs::write(&command_runner, b"pinned command runner").expect("write pinned runner helper");
    fs::write(&code_mode_host, b"pinned code mode host").expect("write pinned code mode host");
    fs::write(&rg, b"pinned rg").expect("write pinned rg");
    fs::write(&package_manifest, b"{\"version\":\"0.146.0\"}\n")
        .expect("write pinned package manifest");
    fs::write(&managed_package_manifest, b"{\"version\":\"0.146.0\"}\n")
        .expect("write managed package manifest");
    let binding = PinnedCodexResources::new(
        managed_root.to_path_buf(),
        resources_directory.to_path_buf(),
        PinnedCodexResourceDigests::new(
            sha256(&fs::read(&sandbox_setup).expect("read setup helper")),
            sha256(&fs::read(&command_runner).expect("read runner helper")),
            sha256(&fs::read(&code_mode_host).expect("read code mode host")),
            sha256(&fs::read(&rg).expect("read rg")),
            sha256(&fs::read(&package_manifest).expect("read package manifest")),
            sha256(&fs::read(&managed_package_manifest).expect("read managed package manifest")),
        )
        .expect("bind pinned resource digests"),
    )
    .expect("bind pinned resource identity");
    PinnedFixtureResources {
        binding,
        sandbox_setup,
        command_runner,
        code_mode_host,
        rg,
    }
}

#[cfg(windows)]
struct PinnedFixtureControls {
    launcher: PathBuf,
    launcher_sha256: String,
    launch_log: PathBuf,
    sandbox_preflight_mode: PathBuf,
    sandbox_preflight_log: PathBuf,
    sandbox_preflight_descendant_pid_log: PathBuf,
    sandbox_preflight_descendant_effect_log: PathBuf,
    readiness_mode: PathBuf,
    readiness_descendant_pid_log: PathBuf,
    readiness_descendant_effect_log: PathBuf,
    turn_start_log: PathBuf,
    sandbox_temp: PathBuf,
}

#[cfg(windows)]
fn write_pinned_control_scripts(
    root: &Path,
    bin: &Path,
    managed_root: &Path,
    resources_directory: &Path,
    sandbox_setup: &Path,
    codex_home: &Path,
    working_directory: &Path,
) -> PinnedFixtureControls {
    let launch_log = root.join("child-launched.txt");
    let sandbox_preflight_mode = root.join("sandbox-preflight-mode.txt");
    let sandbox_preflight_log = root.join("sandbox-preflight.txt");
    let sandbox_preflight_descendant_pid_log = root.join("sandbox-preflight-descendant.pid");
    let sandbox_preflight_descendant_effect_log =
        root.join("sandbox-preflight-descendant-effect.txt");
    let readiness_mode = root.join("readiness-mode.txt");
    let readiness_descendant_pid_log = root.join("readiness-descendant.pid");
    let readiness_descendant_effect_log = root.join("readiness-descendant-effect.txt");
    let turn_start_log = root.join("turn-started.txt");
    let sandbox_temp = codex_home.join(".lattice-fs-sandbox-temp-v1");
    fs::write(&sandbox_preflight_mode, b"ready\n").expect("write sandbox preflight mode");
    fs::write(&readiness_mode, b"ready\n").expect("write readiness mode");
    let sandbox_preflight = bin.join("fake-sandbox-preflight.ps1");
    let preflight_script = pinned_sandbox_preflight_script(&PinnedSandboxPreflightScript {
        log: &sandbox_preflight_log,
        mode: &sandbox_preflight_mode,
        managed_root,
        resources_directory,
        sandbox_setup,
        sandbox_temp: &sandbox_temp,
        codex_home,
        working_directory,
        descendant_pid_log: &sandbox_preflight_descendant_pid_log,
        descendant_effect_log: &sandbox_preflight_descendant_effect_log,
    });
    fs::write(&sandbox_preflight, preflight_script).expect("write fake sandbox preflight");
    let server = bin.join("fake-app-server.ps1");
    let script = pinned_app_server_script(&PinnedServerScript {
        launch_log: &launch_log,
        sandbox_preflight_log: &sandbox_preflight_log,
        managed_root,
        resources_directory,
        sandbox_temp: &sandbox_temp,
        sandbox_setup,
        codex_home,
        readiness_mode: &readiness_mode,
        readiness_descendant_pid_log: &readiness_descendant_pid_log,
        readiness_descendant_effect_log: &readiness_descendant_effect_log,
        turn_start_log: &turn_start_log,
    });
    fs::write(&server, script).expect("write pinned fake app-server");
    let launcher = bin.join("fake-codex.cmd");
    fs::write(
        &launcher,
        "@echo off\r\nif /I \"%~1\"==\"sandbox\" goto sandbox\r\n\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"%~dp0fake-app-server.ps1\" %*\r\nexit /b %ERRORLEVEL%\r\n:sandbox\r\n\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"%~dp0fake-sandbox-preflight.ps1\" %*\r\nexit /b %ERRORLEVEL%\r\n",
    )
    .expect("write pinned fake launcher");
    let launcher_sha256 = sha256(&fs::read(&launcher).expect("read pinned launcher"));
    PinnedFixtureControls {
        launcher,
        launcher_sha256,
        launch_log,
        sandbox_preflight_mode,
        sandbox_preflight_log,
        sandbox_preflight_descendant_pid_log,
        sandbox_preflight_descendant_effect_log,
        readiness_mode,
        readiness_descendant_pid_log,
        readiness_descendant_effect_log,
        turn_start_log,
        sandbox_temp,
    }
}

#[cfg(windows)]
impl PinnedProcessFixture {
    fn new() -> Self {
        let unique = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "lattice-codex-pinned-process-{}-{unique}",
            std::process::id()
        ));
        let package_scope = root.join("node_modules").join("@openai");
        let managed_root = package_scope.join("codex");
        let bundle_root = package_scope
            .join("codex-win32-x64")
            .join("vendor")
            .join("x86_64-pc-windows-msvc");
        let bin = bundle_root.join("bin");
        let resources_directory = bundle_root.join("codex-resources");
        let codex_path_directory = bundle_root.join("codex-path");
        let codex_home = root.join("codex-home");
        let working_directory = root.join("worktree");
        for directory in [
            &managed_root,
            &bin,
            &resources_directory,
            &codex_path_directory,
            &codex_home,
            &working_directory,
        ] {
            fs::create_dir_all(directory).expect("create pinned process fixture directory");
        }
        write_pinned_codex_home(&codex_home);
        let PinnedFixtureResources {
            binding: resources,
            sandbox_setup,
            command_runner,
            code_mode_host,
            rg,
        } = write_pinned_resources(
            &managed_root,
            &bundle_root,
            &bin,
            &resources_directory,
            &codex_path_directory,
        );
        let controls = write_pinned_control_scripts(
            &root,
            &bin,
            &managed_root,
            &resources_directory,
            &sandbox_setup,
            &codex_home,
            &working_directory,
        );
        Self {
            root,
            launcher: controls.launcher,
            codex_home,
            working_directory,
            resources,
            sandbox_setup,
            command_runner,
            code_mode_host,
            rg,
            sandbox_preflight_mode: controls.sandbox_preflight_mode,
            sandbox_preflight_log: controls.sandbox_preflight_log,
            sandbox_preflight_descendant_pid_log: controls.sandbox_preflight_descendant_pid_log,
            sandbox_preflight_descendant_effect_log: controls
                .sandbox_preflight_descendant_effect_log,
            readiness_mode: controls.readiness_mode,
            readiness_descendant_pid_log: controls.readiness_descendant_pid_log,
            readiness_descendant_effect_log: controls.readiness_descendant_effect_log,
            turn_start_log: controls.turn_start_log,
            sandbox_temp: controls.sandbox_temp,
            launch_log: controls.launch_log,
            launcher_sha256: controls.launcher_sha256,
        }
    }

    fn config(&self) -> AppServerRunConfig {
        AppServerRunConfig::new(
            self.launcher.clone(),
            self.launcher_sha256.clone(),
            self.codex_home.clone(),
            self.working_directory.clone(),
            "Create answer.txt",
            Duration::from_mins(2),
            Some(self.resources.clone()),
        )
        .expect("valid pinned app-server config")
    }
}

#[cfg(windows)]
impl Drop for PinnedProcessFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(windows)]
#[test]
fn pinned_official_child_uses_managed_root_and_rejects_helper_drift_before_spawn() {
    let fixture = PinnedProcessFixture::new();
    let evidence = run_codex_app_server(&fixture.config()).unwrap_or_else(|error| {
        let launch =
            fs::read_to_string(&fixture.launch_log).unwrap_or_else(|_| "absent".to_owned());
        panic!("pinned managed package child failed: {error:?}; launch={launch}")
    });
    assert_eq!(evidence.thread_id(), "thread-pinned");
    assert_eq!(
        fs::read_to_string(&fixture.sandbox_preflight_log).expect("read preflight evidence"),
        "completed"
    );
    assert!(fixture.launch_log.is_file());
    assert!(fixture.turn_start_log.is_file());
    assert!(!fixture.sandbox_temp.exists());

    fs::remove_file(&fixture.launch_log).expect("reset child launch evidence");
    fs::remove_file(&fixture.sandbox_preflight_log).expect("reset sandbox preflight evidence");
    let cross_version_root = fixture.root.join("managed-package-0.144.6");
    fs::create_dir(&cross_version_root).expect("create cross-version package root");
    let cross_version = PinnedCodexResources::new(
        cross_version_root,
        fixture.resources.resources_directory().to_path_buf(),
        PinnedCodexResourceDigests::new(
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64),
            "e".repeat(64),
            "f".repeat(64),
        )
        .expect("valid cross-version digest claim"),
    )
    .expect("syntactically valid cross-version resource claim");
    let cross_version_config = AppServerRunConfig::new(
        fixture.launcher.clone(),
        fixture.launcher_sha256.clone(),
        fixture.codex_home.clone(),
        fixture.working_directory.clone(),
        "Create answer.txt",
        Duration::from_secs(5),
        Some(cross_version),
    )
    .expect("cross-version claim reaches live identity validation");
    let error = run_codex_app_server(&cross_version_config)
        .expect_err("a managed root from another package version must fail before spawn");
    assert_eq!(error.kind(), AppServerRunErrorKind::InvalidPinnedResources);
    assert!(!fixture.launch_log.exists());

    fs::write(&fixture.sandbox_setup, b"tampered helper").expect("tamper setup helper");
    let error = run_codex_app_server(&fixture.config())
        .expect_err("helper digest drift must fail before child spawn");
    assert_eq!(
        error.kind(),
        AppServerRunErrorKind::PinnedResourcesDigestMismatch
    );
    assert!(!fixture.launch_log.exists());

    fs::write(&fixture.sandbox_setup, b"pinned sandbox setup")
        .expect("restore pinned setup helper");
    fs::remove_file(&fixture.command_runner).expect("remove command runner helper");
    let error = run_codex_app_server(&fixture.config())
        .expect_err("a missing helper must fail before child spawn");
    assert_eq!(error.kind(), AppServerRunErrorKind::PinnedResourcesMissing);
    assert!(!fixture.launch_log.exists());

    fs::write(&fixture.command_runner, b"pinned command runner")
        .expect("restore command runner helper");
    fs::write(&fixture.code_mode_host, b"tampered code mode host").expect("tamper code mode host");
    let error = run_codex_app_server(&fixture.config())
        .expect_err("code mode host digest drift must fail before child spawn");
    assert_eq!(
        error.kind(),
        AppServerRunErrorKind::PinnedResourcesDigestMismatch
    );
    assert!(!fixture.launch_log.exists());

    fs::write(&fixture.code_mode_host, b"pinned code mode host").expect("restore code mode host");
    fs::remove_file(&fixture.rg).expect("remove pinned rg");
    let error = run_codex_app_server(&fixture.config())
        .expect_err("a missing pinned rg must fail before child spawn");
    assert_eq!(error.kind(), AppServerRunErrorKind::PinnedResourcesMissing);
    assert!(!fixture.launch_log.exists());
    fs::write(&fixture.rg, b"pinned rg").expect("restore pinned rg");
    assert!(!fixture.sandbox_preflight_log.exists());
}

#[cfg(windows)]
#[test]
fn sandbox_preflight_failure_prevents_app_server_spawn() {
    let fixture = PinnedProcessFixture::new();
    fs::write(&fixture.sandbox_preflight_mode, b"fail\n")
        .expect("select failing sandbox preflight");

    let error = run_codex_app_server(&fixture.config())
        .expect_err("failed official sandbox preflight must stop before app-server spawn");

    assert_eq!(
        error.kind(),
        AppServerRunErrorKind::FsSandboxBootstrapFailed
    );
    assert_eq!(
        fs::read_to_string(&fixture.sandbox_preflight_log).expect("read preflight start evidence"),
        "started"
    );
    assert!(!fixture.launch_log.exists());
    assert!(!fixture.sandbox_temp.exists());
}

#[cfg(windows)]
#[test]
fn resource_drift_after_preflight_is_rejected_before_app_server_spawn() {
    let fixture = PinnedProcessFixture::new();
    fs::write(&fixture.sandbox_preflight_mode, b"tamper-after-preflight\n")
        .expect("select post-preflight resource drift");

    let error = run_codex_app_server(&fixture.config())
        .expect_err("resource drift after preflight must stop before app-server spawn");

    assert_eq!(error.kind(), AppServerRunErrorKind::PinnedResourcesChanged);
    assert_eq!(
        fs::read_to_string(&fixture.sandbox_preflight_log)
            .expect("read completed preflight evidence"),
        "completed"
    );
    assert!(!fixture.launch_log.exists());
    assert!(!fixture.sandbox_temp.exists());
}

#[cfg(windows)]
#[test]
fn sandbox_preflight_timeout_reaps_owned_tree_before_app_server_spawn() {
    let fixture = PinnedProcessFixture::new();
    fs::write(&fixture.sandbox_preflight_mode, b"hang\n")
        .expect("select hanging sandbox preflight");
    let started = Instant::now();

    let error = run_codex_app_server(&fixture.config())
        .expect_err("hanging official sandbox preflight must time out");

    assert_eq!(error.kind(), AppServerRunErrorKind::FsSandboxHelperTimeout);
    assert!(started.elapsed() >= Duration::from_mins(1));
    assert!(started.elapsed() < Duration::from_secs(65));
    assert!(!fixture.launch_log.exists());
    let descendant_pid = fs::read_to_string(&fixture.sandbox_preflight_descendant_pid_log)
        .expect("preflight spawned a descendant before hanging");
    assert!(descendant_pid.trim().parse::<u32>().is_ok());
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        !fixture.sandbox_preflight_descendant_effect_log.exists(),
        "preflight descendant survived Job Object close"
    );
    assert!(!fixture.sandbox_temp.exists());
}

#[cfg(windows)]
#[test]
fn caller_deadline_remains_distinct_from_preflight_watchdog() {
    let fixture = PinnedProcessFixture::new();
    fs::write(&fixture.sandbox_preflight_mode, b"hang\n")
        .expect("select hanging sandbox preflight");
    let deadline = Instant::now() + Duration::from_secs(1);

    let error = run_codex_app_server_until(&fixture.config(), deadline)
        .expect_err("the shorter caller deadline must remain authoritative");

    assert_eq!(error.kind(), AppServerRunErrorKind::Timeout);
    assert!(!fixture.launch_log.exists());
    assert!(!fixture.sandbox_temp.exists());
}

#[cfg(windows)]
#[test]
fn resources_directory_junction_is_rejected_before_child_effect() {
    let fixture = PinnedProcessFixture::new();
    let resources_directory = fixture.resources.resources_directory();
    fs::remove_dir_all(resources_directory).expect("remove resources before junction fixture");
    let reparse_target = fixture.root.join("reparse-resources-target");
    fs::create_dir(&reparse_target).expect("create resources junction target");
    fs::write(
        reparse_target.join("codex-windows-sandbox-setup.exe"),
        b"pinned sandbox setup",
    )
    .expect("write setup junction target");
    fs::write(
        reparse_target.join("codex-command-runner.exe"),
        b"pinned command runner",
    )
    .expect("write runner junction target");
    let cmd = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
        .join("System32")
        .join("cmd.exe");
    let status = std::process::Command::new(cmd)
        .args(["/d", "/c", "mklink", "/J"])
        .arg(resources_directory)
        .arg(&reparse_target)
        .status()
        .expect("run mklink for directory junction");
    assert!(status.success(), "create resources directory junction");

    let error = run_codex_app_server(&fixture.config())
        .expect_err("a resources directory reparse point must fail before child spawn");

    assert_eq!(error.kind(), AppServerRunErrorKind::InvalidPinnedResources);
    assert!(!fixture.launch_log.exists());
}

#[cfg(windows)]
#[test]
fn codex_path_junction_is_rejected_before_child_effect() {
    let fixture = PinnedProcessFixture::new();
    let codex_path = fixture.rg.parent().expect("rg has codex-path parent");
    let junction_target = fixture.root.join("codex-path-target");
    fs::rename(codex_path, &junction_target).expect("move real codex-path directory");
    let cmd = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
        .join("System32")
        .join("cmd.exe");
    let status = std::process::Command::new(cmd)
        .args(["/d", "/c", "mklink", "/J"])
        .arg(codex_path)
        .arg(&junction_target)
        .status()
        .expect("run mklink for codex-path junction");
    assert!(status.success(), "create codex-path directory junction");

    let error = run_codex_app_server(&fixture.config())
        .expect_err("a codex-path reparse point must fail before child spawn");

    assert_eq!(error.kind(), AppServerRunErrorKind::InvalidPinnedResources);
    assert!(!fixture.launch_log.exists());
}

#[cfg(windows)]
#[test]
fn sandbox_readiness_must_be_ready_before_thread_start() {
    let fixture = PinnedProcessFixture::new();
    fs::write(&fixture.readiness_mode, b"update-required\n")
        .expect("select update-required readiness");

    let error = run_codex_app_server(&fixture.config())
        .expect_err("a non-ready sandbox must fail before thread/start");

    assert_eq!(
        error.kind(),
        AppServerRunErrorKind::FsSandboxBootstrapFailed
    );
    assert!(!fixture.turn_start_log.exists());
}

#[cfg(windows)]
#[test]
fn sandbox_temp_must_be_fresh_empty_and_non_reparse_before_spawn() {
    let stale = PinnedProcessFixture::new();
    fs::create_dir(&stale.sandbox_temp).expect("create stale sandbox temp");
    fs::write(stale.sandbox_temp.join("ambient.tmp"), b"ambient")
        .expect("populate stale sandbox temp");
    let error = run_codex_app_server(&stale.config())
        .expect_err("a non-fresh sandbox temp must fail before spawn");
    assert_eq!(error.kind(), AppServerRunErrorKind::InvalidCodexHome);
    assert!(!stale.launch_log.exists());

    let reparse = PinnedProcessFixture::new();
    let target = reparse.root.join("sandbox-temp-target");
    fs::create_dir(&target).expect("create sandbox temp junction target");
    let cmd = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
        .join("System32")
        .join("cmd.exe");
    let status = std::process::Command::new(cmd)
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&reparse.sandbox_temp)
        .arg(&target)
        .status()
        .expect("run mklink for sandbox temp junction");
    assert!(status.success(), "create sandbox temp directory junction");
    let error = run_codex_app_server(&reparse.config())
        .expect_err("a sandbox temp reparse point must fail before spawn");
    assert_eq!(error.kind(), AppServerRunErrorKind::InvalidCodexHome);
    assert!(!reparse.launch_log.exists());
}

#[cfg(windows)]
#[test]
fn sandbox_readiness_timeout_reaps_the_owned_tree_before_thread_start() {
    let fixture = PinnedProcessFixture::new();
    fs::write(&fixture.readiness_mode, b"hang\n").expect("select hanging readiness");
    let started = Instant::now();

    let error = run_codex_app_server(&fixture.config())
        .expect_err("a hanging sandbox readiness request must time out");

    assert_eq!(error.kind(), AppServerRunErrorKind::FsSandboxHelperTimeout);
    assert!(started.elapsed() >= Duration::from_secs(30));
    assert!(started.elapsed() < Duration::from_secs(35));
    assert!(!fixture.turn_start_log.exists());
    let descendant_pid = fs::read_to_string(&fixture.readiness_descendant_pid_log)
        .expect("readiness handler spawned a descendant before hanging");
    assert!(descendant_pid.trim().parse::<u32>().is_ok());
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        !fixture.readiness_descendant_effect_log.exists(),
        "readiness descendant survived Job Object close"
    );
    assert!(!fixture.sandbox_temp.exists());
}

#[cfg(windows)]
#[test]
fn caller_deadline_remains_distinct_from_the_readiness_watchdog() {
    let fixture = PinnedProcessFixture::new();
    fs::write(&fixture.readiness_mode, b"hang\n").expect("select hanging readiness");
    let deadline = Instant::now() + Duration::from_secs(1);

    let error = run_codex_app_server_until(&fixture.config(), deadline)
        .expect_err("the shorter caller deadline must remain authoritative");

    assert_eq!(error.kind(), AppServerRunErrorKind::Timeout);
    assert!(!fixture.turn_start_log.exists());
}

#[cfg(windows)]
#[test]
fn launcher_bin_junction_is_rejected_before_child_effect() {
    let fixture = PinnedProcessFixture::new();
    let launcher_bin = fixture
        .launcher
        .parent()
        .expect("pinned launcher has a bin directory")
        .to_path_buf();
    let junction_target = fixture.root.join("launcher-bin-target");
    fs::rename(&launcher_bin, &junction_target).expect("move real launcher bin directory");
    let cmd = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
        .join("System32")
        .join("cmd.exe");
    let status = std::process::Command::new(cmd)
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&launcher_bin)
        .arg(&junction_target)
        .status()
        .expect("run mklink for launcher bin junction");
    assert!(status.success(), "create launcher bin directory junction");

    let error = run_codex_app_server(&fixture.config())
        .expect_err("a launcher bin reparse point must fail before child spawn");

    assert_eq!(error.kind(), AppServerRunErrorKind::InvalidPinnedResources);
    assert!(!fixture.launch_log.exists());
}

#[cfg(windows)]
#[test]
fn scripted_app_server_completes_one_exact_dedicated_home_turn() {
    let fixture = ProcessFixture::new(FakeMode::Success);
    let evidence = run_codex_app_server(&fixture.config(Duration::from_secs(5)))
        .expect("scripted app-server completes");

    assert_eq!(evidence.thread_id(), "thread-scripted");
    assert_eq!(evidence.turn_id(), "turn-scripted");
    assert_eq!(evidence.outcome().status, TurnStatus::Completed);
    assert_eq!(evidence.initialize().codex_home, fixture.codex_home);
}

#[cfg(windows)]
#[test]
fn expired_absolute_deadline_rejects_before_spawning_the_child() {
    let fixture = ProcessFixture::new(FakeMode::Success);
    let deadline = Instant::now();

    let error = run_codex_app_server_until(&fixture.config(Duration::from_secs(30)), deadline)
        .expect_err("an expired composition deadline must reject before spawn");

    assert_eq!(error.kind(), AppServerRunErrorKind::Timeout);
    assert!(!fixture.effect_log.exists());
}

#[cfg(windows)]
#[test]
fn rejects_unowned_or_worktree_overlapping_codex_home_before_spawn() {
    let fixture = ProcessFixture::new(FakeMode::Success);
    fs::remove_file(fixture.codex_home.join(CODEX_HOME_OWNERSHIP_MARKER_NAME))
        .expect("remove fixture ownership marker");
    let error = run_codex_app_server(&fixture.config(Duration::from_secs(5)))
        .expect_err("an unowned Codex home must be denied");
    assert_eq!(
        error.kind(),
        AppServerRunErrorKind::CodexHomeOwnershipMissing
    );

    let overlap = ProcessFixture::new(FakeMode::Success);
    fs::write(
        overlap.root.join(CODEX_HOME_OWNERSHIP_MARKER_NAME),
        CODEX_HOME_OWNERSHIP_MARKER_BYTES,
    )
    .expect("mark overlap fixture root");
    let config = AppServerRunConfig::new(
        overlap.launcher.clone(),
        overlap.launcher_sha256.clone(),
        overlap.root.clone(),
        overlap.root.clone(),
        "Create answer.txt",
        Duration::from_secs(5),
        None,
    )
    .expect("overlap is rejected at live admission");
    let error = run_codex_app_server(&config)
        .expect_err("Codex home must not overlap the writable worktree");
    assert_eq!(error.kind(), AppServerRunErrorKind::CodexHomeOverlap);
}

#[cfg(windows)]
#[test]
fn rejects_unsafe_isolated_home_config_before_spawn() {
    let unsafe_config = ProcessFixture::new(FakeMode::Success);
    fs::write(
        unsafe_config.codex_home.join("config.toml"),
        "approval_policy = \"never\"\nsandbox_mode = \"danger-full-access\"\n",
    )
    .expect("replace fixture config");
    let error = run_codex_app_server(&unsafe_config.config(Duration::from_secs(5)))
        .expect_err("unsafe config must fail before spawn");
    assert_eq!(error.kind(), AppServerRunErrorKind::InvalidCodexHome);
    assert!(!unsafe_config.effect_log.exists());

    let elevated = ProcessFixture::new(FakeMode::Success);
    fs::write(
        elevated.codex_home.join("config.toml"),
        concat!(
            "approval_policy = \"never\"\n",
            "sandbox_mode = \"workspace-write\"\n",
            "model = \"gpt-5.6-sol\"\n",
            "model_reasoning_effort = \"low\"\n",
            "\n",
            "[windows]\n",
            "sandbox = \"elevated\"\n",
            "\n",
            "[features]\n",
            "plugins = false\n",
        ),
    )
    .expect("replace fixture with elevated config");
    let error = run_codex_app_server(&elevated.config(Duration::from_secs(5)))
        .expect_err("implicit elevated setup must fail before spawn");
    assert_eq!(error.kind(), AppServerRunErrorKind::InvalidCodexHome);
    assert!(!elevated.effect_log.exists());

    let plugins_enabled = ProcessFixture::new(FakeMode::Success);
    fs::write(
        plugins_enabled.codex_home.join("config.toml"),
        concat!(
            "approval_policy = \"never\"\n",
            "sandbox_mode = \"workspace-write\"\n",
            "model = \"gpt-5.6-sol\"\n",
            "model_reasoning_effort = \"low\"\n",
            "\n",
            "[windows]\n",
            "sandbox = \"unelevated\"\n",
        ),
    )
    .expect("replace fixture with plugin-capable config");
    let error = run_codex_app_server(&plugins_enabled.config(Duration::from_secs(5)))
        .expect_err("plugin-capable config must fail before spawn");
    assert_eq!(error.kind(), AppServerRunErrorKind::InvalidCodexHome);
    assert!(!plugins_enabled.effect_log.exists());
}

#[cfg(windows)]
#[test]
fn rejects_and_preserves_any_codex_project_trust_entry() {
    let fixture = ProcessFixture::new(FakeMode::Success);
    let trust_path = fixture
        .codex_home
        .parent()
        .expect("Codex home has a fixture root")
        .join("runtime-delivery")
        .join(format!("task-{}", "a".repeat(64)))
        .join("repo")
        .to_string_lossy()
        .to_ascii_lowercase();
    fs::write(
        fixture.codex_home.join("config.toml"),
        format!(
            "{}[projects.'{}']\ntrust_level = \"trusted\"\n",
            String::from_utf8_lossy(CODEX_HOME_CONFIG_BYTES),
            trust_path,
        ),
    )
    .expect("write Codex-generated current-worktree trust entry");

    let exact = fs::read(fixture.codex_home.join("config.toml")).expect("read tampered config");
    let error = run_codex_app_server(&fixture.config(Duration::from_secs(5)))
        .expect_err("any project trust entry must be rejected before spawn");
    assert_eq!(error.kind(), AppServerRunErrorKind::InvalidCodexHome);
    assert!(!fixture.effect_log.exists());
    assert_eq!(
        fs::read(fixture.codex_home.join("config.toml")).expect("re-read tampered config"),
        exact,
        "validation must not rewrite rejected Codex state"
    );
}

#[cfg(windows)]
#[test]
fn rejects_a_codex_trust_entry_for_another_worktree() {
    let fixture = ProcessFixture::new(FakeMode::Success);
    fs::write(
        fixture.codex_home.join("config.toml"),
        concat!(
            "approval_policy = \"never\"\n",
            "sandbox_mode = \"workspace-write\"\n",
            "model = \"gpt-5.6-sol\"\n",
            "model_reasoning_effort = \"low\"\n",
            "\n",
            "[windows]\n",
            "sandbox = \"unelevated\"\n",
            "\n",
            "[features]\n",
            "plugins = false\n",
            "\n",
            "[projects.'c:\\untrusted-worktree']\n",
            "trust_level = \"trusted\"\n",
        ),
    )
    .expect("write foreign Codex trust entry");

    let error = run_codex_app_server(&fixture.config(Duration::from_secs(5)))
        .expect_err("a foreign worktree trust entry must be rejected");
    assert_eq!(error.kind(), AppServerRunErrorKind::InvalidCodexHome);
    assert!(!fixture.effect_log.exists());
}

#[cfg(windows)]
#[test]
fn scripted_malformed_eof_and_wrong_home_fail_closed() {
    for (mode, expected) in [
        (
            FakeMode::Yielded,
            AppServerRunErrorKind::IncompleteToolExecution,
        ),
        (FakeMode::Malformed, AppServerRunErrorKind::ProtocolFailed),
        (FakeMode::Eof, AppServerRunErrorKind::AmbiguousEof),
        (FakeMode::Premature, AppServerRunErrorKind::ProtocolFailed),
        (
            FakeMode::WrongHome,
            AppServerRunErrorKind::CodexHomeMismatch,
        ),
    ] {
        let fixture = ProcessFixture::new(mode);
        let error = run_codex_app_server(&fixture.config(Duration::from_secs(5)))
            .expect_err("unsafe scripted outcome must fail closed");
        assert_eq!(error.kind(), expected);
        if matches!(mode, FakeMode::WrongHome) {
            assert!(
                !fixture.effect_log.exists(),
                "a mismatched Codex home must be rejected before thread/start"
            );
        }
    }
}

#[cfg(windows)]
#[test]
fn timeout_immediately_terminates_and_reaps_the_owned_tree() {
    let fixture = ProcessFixture::new(FakeMode::Timeout);
    let started = Instant::now();
    let error = run_codex_app_server(&fixture.config(Duration::from_secs(10)))
        .expect_err("timed out turn must not succeed");

    assert_eq!(
        error.kind(),
        AppServerRunErrorKind::Timeout,
        "native timeout fixture error: {error:?}; launcher={:?}; exists={}",
        fixture.launcher,
        fixture.launcher.exists(),
    );
    assert!(started.elapsed() < Duration::from_secs(20));
    let descendant_pid = fs::read_to_string(&fixture.descendant_pid_log)
        .expect("the timed-out turn proved that it spawned a writable descendant");
    assert!(
        descendant_pid.trim().parse::<u32>().is_ok(),
        "descendant PID: {descendant_pid:?}"
    );
    fs::write(&fixture.descendant_trigger, b"post-return\n")
        .expect("release a surviving descendant only after cleanup returned");
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        !fixture.descendant_effect_log.exists(),
        "the timed-out turn left a writable descendant after cleanup returned"
    );
}

#[cfg(windows)]
#[test]
fn job_object_kills_a_descendant_after_the_launcher_parent_exits() {
    let fixture = ProcessFixture::new(FakeMode::Orphan);
    let evidence = run_codex_app_server(&fixture.config(Duration::from_secs(5)))
        .expect("the scripted terminal is observed before its parent exits");

    assert_eq!(evidence.outcome().status, TurnStatus::Completed);
    let descendant_pid = fs::read_to_string(&fixture.descendant_pid_log)
        .expect("the scripted server proved that it spawned a descendant");
    assert!(
        descendant_pid.trim().parse::<u32>().is_ok(),
        "descendant PID: {descendant_pid:?}"
    );
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        !fixture.descendant_effect_log.exists(),
        "the descendant survived Job Object close and produced a delayed effect"
    );
}

#[cfg(windows)]
fn fake_launcher_script(
    mode: FakeMode,
    codex_home: &std::path::Path,
    wrong_home: &std::path::Path,
    effect_log: &std::path::Path,
    descendant_pid_log: &std::path::Path,
    descendant_effect_log: &std::path::Path,
) -> String {
    let quote = |path: &std::path::Path| path.display().to_string().replace('\'', "''");
    let descendant_deadline_trigger =
        descendant_effect_log.with_file_name(DESCENDANT_DEADLINE_TRIGGER_NAME);
    let descendant_trigger = descendant_effect_log.with_file_name(DESCENDANT_TRIGGER_NAME);
    let configured_home = quote(codex_home);
    let reported_home = match mode {
        FakeMode::WrongHome => wrong_home,
        _ => codex_home,
    }
    .display()
    .to_string()
    .replace('\\', r"\\");
    let common = format!(
        "$ErrorActionPreference = 'Stop'\nif ($env:CODEX_HOME -ne '{configured_home}') {{ exit 41 }}\n$null = [Console]::In.ReadLine()\n[Console]::Out.WriteLine('{{\"id\":0,\"result\":{{\"userAgent\":\"codex_cli_rs/0.144.6\",\"platformFamily\":\"windows\",\"platformOs\":\"windows\",\"codexHome\":\"{reported_home}\"}}}}')\n"
    );
    let lifecycle = format!(
        "$null = [Console]::In.ReadLine()\n$threadStart = [Console]::In.ReadLine()\nif ($threadStart -like '*\"method\":\"thread/start\"*') {{ [IO.File]::WriteAllText('{}', 'thread/start received') }}\n[Console]::Out.WriteLine('{{\"id\":1,\"result\":{{\"thread\":{{\"id\":\"thread-scripted\"}}}}}}')\n$null = [Console]::In.ReadLine()\n[Console]::Out.WriteLine('{{\"id\":2,\"result\":{{\"turn\":{{\"id\":\"turn-scripted\"}}}}}}')\n[Console]::Out.WriteLine('{{\"method\":\"turn/started\",\"params\":{{\"threadId\":\"thread-scripted\",\"turn\":{{\"id\":\"turn-scripted\",\"status\":\"inProgress\"}}}}}}')\n",
        quote(effect_log)
    );
    let apply_completed = r#"{"method":"item/completed","params":{"threadId":"thread-scripted","turnId":"turn-scripted","item":{"arguments":{"command":"nested shell write fixture"},"contentItems":[{"text":"Script completed\nExit code: 0","type":"inputText"}],"id":"tool-shell-write","status":"completed","success":true,"tool":"exec","type":"dynamicToolCall"},"completedAtMs":1}}"#;
    let verify_completed = r#"{"method":"item/completed","params":{"threadId":"thread-scripted","turnId":"turn-scripted","item":{"arguments":{"command":"nested shell verify fixture"},"contentItems":[{"text":"Script completed\nExit code: 0","type":"inputText"}],"id":"tool-shell-verify","status":"completed","success":true,"tool":"exec","type":"dynamicToolCall"},"completedAtMs":2}}"#;
    let completed = r#"{"method":"turn/completed","params":{"threadId":"thread-scripted","turn":{"id":"turn-scripted","items":[{"id":"agent-final","text":"Delivery complete.","type":"agentMessage"}],"itemsView":"summary","status":"completed","error":null}}}"#;
    let tail = match mode {
        FakeMode::Success | FakeMode::WrongHome => format!(
            "[Console]::Out.WriteLine('{apply_completed}')\n[Console]::Out.WriteLine('{verify_completed}')\n[Console]::Out.WriteLine('{completed}')\nStart-Sleep -Seconds 60\n"
        ),
        FakeMode::Yielded => format!(
            r#"[Console]::Out.WriteLine('{{"method":"item/completed","params":{{"threadId":"thread-scripted","turnId":"turn-scripted","item":{{"arguments":{{}},"contentItems":[{{"text":"Script running with cell ID cell-7","type":"inputText"}}],"id":"tool-exec","status":"completed","success":true,"tool":"exec","type":"dynamicToolCall"}},"completedAtMs":1}}}}')
[Console]::Out.WriteLine('{completed}')
Start-Sleep -Seconds 60
"#
        ),
        FakeMode::Malformed => {
            "[Console]::Out.WriteLine('{not-json')\nStart-Sleep -Seconds 60\n".to_owned()
        }
        FakeMode::Eof => "exit 0\n".to_owned(),
        FakeMode::Timeout => format!(
            "$grandchild = \"`$stop = [DateTime]::UtcNow.AddSeconds(30); while (!(Test-Path -LiteralPath '{}') -and !(Test-Path -LiteralPath '{}')) {{ if ([DateTime]::UtcNow -ge `$stop) {{ exit 91 }}; Start-Sleep -Milliseconds 10 }}; if (Test-Path -LiteralPath '{}') {{ Start-Sleep -Milliseconds 250 }}; [IO.File]::WriteAllText('{}', 'survived')\"\n$encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($grandchild))\n$descendant = Start-Process -FilePath \"$PSHOME\\powershell.exe\" -WindowStyle Hidden -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-EncodedCommand',$encoded) -PassThru\n[IO.File]::WriteAllText('{}', [string]$descendant.Id)\n$null = [Console]::In.ReadLine()\n[IO.File]::WriteAllText('{}', 'deadline')\n[Console]::Out.WriteLine('{{\"id\":4,\"result\":{{}}}}')\nStart-Sleep -Seconds 60\n",
            quote(&descendant_deadline_trigger),
            quote(&descendant_trigger),
            quote(&descendant_deadline_trigger),
            quote(descendant_effect_log),
            quote(descendant_pid_log),
            quote(&descendant_deadline_trigger)
        ),
        FakeMode::Premature => concat!(
            "[Console]::Out.WriteLine('{\"id\":1,\"result\":{\"thread\":{\"id\":\"thread-scripted\"}}}')\n",
            "[Console]::Out.WriteLine('{\"id\":2,\"result\":{\"turn\":{\"id\":\"turn-scripted\"}}}')\n",
            "[Console]::Out.WriteLine('{\"method\":\"turn/completed\",\"params\":{\"threadId\":\"thread-scripted\",\"turn\":{\"id\":\"turn-scripted\",\"items\":[],\"status\":\"completed\",\"error\":null}}}')\n",
            "Start-Sleep -Seconds 60\n",
        )
        .to_owned(),
        FakeMode::Orphan => format!(
            "$grandchild = \"Start-Sleep -Milliseconds 800; [IO.File]::WriteAllText('{}', 'survived')\"\n$encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($grandchild))\n$descendant = Start-Process -FilePath \"$PSHOME\\powershell.exe\" -WindowStyle Hidden -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-EncodedCommand',$encoded) -PassThru\n[IO.File]::WriteAllText('{}', [string]$descendant.Id)\n[Console]::Out.WriteLine('{apply_completed}')\n[Console]::Out.WriteLine('{verify_completed}')\n[Console]::Out.WriteLine('{completed}')\nexit 0\n",
            quote(descendant_effect_log),
            quote(descendant_pid_log)
        ),
    };
    match mode {
        FakeMode::Malformed | FakeMode::Eof | FakeMode::Premature => {
            format!("{common}{tail}")
        }
        _ => format!("{common}{lifecycle}{tail}"),
    }
}

#[cfg(windows)]
fn sha256(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let mut digest = String::with_capacity(64);
    for byte in hasher.finalize() {
        write!(&mut digest, "{byte:02x}").expect("write digest");
    }
    digest
}
