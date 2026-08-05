use std::path::PathBuf;
use std::time::Duration;

use lattice_codex_adapter::{
    AppServerRunConfig, AppServerRunErrorKind, CODEX_HOME_OWNERSHIP_MARKER_BYTES,
    CODEX_HOME_OWNERSHIP_MARKER_NAME, TurnStatus,
};

#[cfg(windows)]
use std::fs;
#[cfg(windows)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(windows)]
use std::time::Instant;

#[cfg(windows)]
use lattice_codex_adapter::{run_codex_app_server, run_codex_app_server_until};
#[cfg(windows)]
use sha2::{Digest, Sha256};

#[cfg(windows)]
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

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
    interrupt_log: PathBuf,
    effect_log: PathBuf,
    descendant_pid_log: PathBuf,
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
        let wrong_home = root.join("wrong-home");
        fs::create_dir_all(&codex_home).expect("create dedicated Codex home");
        fs::create_dir_all(&working_directory).expect("create isolated worktree fixture");
        fs::create_dir_all(&wrong_home).expect("create wrong Codex home fixture");
        fs::write(
            codex_home.join(CODEX_HOME_OWNERSHIP_MARKER_NAME),
            CODEX_HOME_OWNERSHIP_MARKER_BYTES,
        )
        .expect("mark dedicated Codex home as LATTICE-owned");
        fs::write(codex_home.join("auth.json"), b"{}\n")
            .expect("write inert fixture auth presence");
        fs::write(
            codex_home.join("config.toml"),
            concat!(
                "approval_policy = \"never\"\n",
                "sandbox_mode = \"workspace-write\"\n",
                "model = \"gpt-5.6-sol\"\n",
                "model_reasoning_effort = \"low\"\n",
                "\n",
                "[windows]\n",
                "sandbox = \"elevated\"\n",
            ),
        )
        .expect("write safe fixture configuration");
        let interrupt_log = root.join("interrupt.jsonl");
        let effect_log = root.join("thread-started.txt");
        let descendant_pid_log = root.join("descendant.pid");
        let descendant_effect_log = root.join("descendant-effect.txt");
        let server = root.join("fake-app-server.ps1");
        fs::write(
            &server,
            fake_launcher_script(
                mode,
                &codex_home,
                &wrong_home,
                &interrupt_log,
                &effect_log,
                &descendant_pid_log,
                &descendant_effect_log,
            ),
        )
        .expect("write scripted PowerShell app-server");
        let launcher = root.join("fake-codex.cmd");
        fs::write(
            &launcher,
            "@echo off\r\n\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"%~dp0fake-app-server.ps1\" %*\r\n",
        )
        .expect("write scripted app-server");
        let launcher_sha256 = sha256(&fs::read(&launcher).expect("read scripted app-server"));
        Self {
            root,
            launcher,
            codex_home,
            working_directory,
            interrupt_log,
            effect_log,
            descendant_pid_log,
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
        )
        .expect("valid scripted app-server config")
    }
}

#[cfg(windows)]
impl Drop for ProcessFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
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
}

#[cfg(windows)]
#[test]
fn scripted_malformed_eof_and_wrong_home_fail_closed() {
    for (mode, expected) in [
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
fn timeout_sends_interrupt_then_terminates_the_owned_tree() {
    let fixture = ProcessFixture::new(FakeMode::Timeout);
    let started = Instant::now();
    let error = run_codex_app_server(&fixture.config(Duration::from_secs(2)))
        .expect_err("timed out turn must not succeed");

    assert_eq!(error.kind(), AppServerRunErrorKind::Timeout);
    assert!(started.elapsed() < Duration::from_secs(10));
    let interrupt = fs::read_to_string(&fixture.interrupt_log)
        .expect("scripted child observed the interrupt request");
    assert!(
        interrupt.contains(r#""method":"turn/interrupt""#),
        "observed interrupt: {interrupt:?}"
    );
    assert!(
        interrupt.contains(r#""threadId":"thread-scripted""#),
        "observed interrupt: {interrupt:?}"
    );
    assert!(
        interrupt.contains(r#""turnId":"turn-scripted""#),
        "observed interrupt: {interrupt:?}"
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
    interrupt_log: &std::path::Path,
    effect_log: &std::path::Path,
    descendant_pid_log: &std::path::Path,
    descendant_effect_log: &std::path::Path,
) -> String {
    let quote = |path: &std::path::Path| path.display().to_string().replace('\'', "''");
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
        "$null = [Console]::In.ReadLine()\n$threadStart = [Console]::In.ReadLine()\nif ($threadStart -like '*\"method\":\"thread/start\"*') {{ [IO.File]::WriteAllText('{}', 'thread/start received') }}\n[Console]::Out.WriteLine('{{\"id\":1,\"result\":{{\"thread\":{{\"id\":\"thread-scripted\"}}}}}}')\n$null = [Console]::In.ReadLine()\n[Console]::Out.WriteLine('{{\"id\":2,\"result\":{{\"turn\":{{\"id\":\"turn-scripted\"}}}}}}')\n",
        quote(effect_log)
    );
    let tail = match mode {
        FakeMode::Success | FakeMode::WrongHome => concat!(
            "[Console]::Out.WriteLine('{\"method\":\"turn/completed\",\"params\":{\"threadId\":\"thread-scripted\",\"turn\":{\"id\":\"turn-scripted\",\"items\":[],\"status\":\"completed\",\"error\":null}}}')\n",
            "Start-Sleep -Seconds 60\n",
        )
        .to_owned(),
        FakeMode::Malformed => {
            "[Console]::Out.WriteLine('{not-json')\nStart-Sleep -Seconds 60\n".to_owned()
        }
        FakeMode::Eof => "exit 0\n".to_owned(),
        FakeMode::Timeout => format!(
            "$interrupt = [Console]::In.ReadLine()\n[IO.File]::WriteAllText('{}', $interrupt + [Environment]::NewLine)\n[Console]::Out.WriteLine('{{\"id\":3,\"result\":{{}}}}')\n[Console]::Out.WriteLine('{{\"method\":\"turn/completed\",\"params\":{{\"threadId\":\"thread-scripted\",\"turn\":{{\"id\":\"turn-scripted\",\"items\":[],\"status\":\"interrupted\",\"error\":null}}}}}}')\nStart-Sleep -Seconds 60\n",
            quote(interrupt_log)
        ),
        FakeMode::Premature => concat!(
            "[Console]::Out.WriteLine('{\"id\":1,\"result\":{\"thread\":{\"id\":\"thread-scripted\"}}}')\n",
            "[Console]::Out.WriteLine('{\"id\":2,\"result\":{\"turn\":{\"id\":\"turn-scripted\"}}}')\n",
            "[Console]::Out.WriteLine('{\"method\":\"turn/completed\",\"params\":{\"threadId\":\"thread-scripted\",\"turn\":{\"id\":\"turn-scripted\",\"items\":[],\"status\":\"completed\",\"error\":null}}}')\n",
            "Start-Sleep -Seconds 60\n",
        )
        .to_owned(),
        FakeMode::Orphan => format!(
            "$grandchild = \"Start-Sleep -Milliseconds 800; [IO.File]::WriteAllText('{}', 'survived')\"\n$encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($grandchild))\n$descendant = Start-Process -FilePath \"$PSHOME\\powershell.exe\" -WindowStyle Hidden -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-EncodedCommand',$encoded) -PassThru\n[IO.File]::WriteAllText('{}', [string]$descendant.Id)\n[Console]::Out.WriteLine('{{\"method\":\"turn/completed\",\"params\":{{\"threadId\":\"thread-scripted\",\"turn\":{{\"id\":\"turn-scripted\",\"items\":[],\"status\":\"completed\",\"error\":null}}}}}}')\nexit 0\n",
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
