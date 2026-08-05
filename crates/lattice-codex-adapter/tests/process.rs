use std::path::PathBuf;
use std::time::Duration;

use lattice_codex_adapter::{
    AppServerRunConfig, AppServerRunErrorKind, CODEX_HOME_OWNERSHIP_MARKER_BYTES,
    CODEX_HOME_OWNERSHIP_MARKER_NAME, PinnedCodexResources, TurnStatus,
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
fn pinned_resource_binding_requires_an_absolute_directory_and_exact_digests() {
    let error = PinnedCodexResources::new(
        PathBuf::from("managed-package"),
        PathBuf::from("codex-resources"),
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
        "d".repeat(64),
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
            None,
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
struct PinnedProcessFixture {
    root: PathBuf,
    launcher: PathBuf,
    codex_home: PathBuf,
    working_directory: PathBuf,
    resources: PinnedCodexResources,
    sandbox_setup: PathBuf,
    command_runner: PathBuf,
    launch_log: PathBuf,
    launcher_sha256: String,
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
        let codex_home = root.join("codex-home");
        let working_directory = root.join("worktree");
        for directory in [
            &managed_root,
            &bin,
            &resources_directory,
            &codex_home,
            &working_directory,
        ] {
            fs::create_dir_all(directory).expect("create pinned process fixture directory");
        }
        fs::write(
            codex_home.join(CODEX_HOME_OWNERSHIP_MARKER_NAME),
            CODEX_HOME_OWNERSHIP_MARKER_BYTES,
        )
        .expect("write Codex home marker");
        fs::write(codex_home.join("auth.json"), b"{}\n").expect("write inert fixture auth");
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
        .expect("write exact safe fixture config");
        let sandbox_setup = resources_directory.join("codex-windows-sandbox-setup.exe");
        let command_runner = resources_directory.join("codex-command-runner.exe");
        let package_manifest = bundle_root.join("codex-package.json");
        let managed_package_manifest = managed_root.join("package.json");
        fs::write(&sandbox_setup, b"pinned sandbox setup").expect("write pinned setup helper");
        fs::write(&command_runner, b"pinned command runner").expect("write pinned runner helper");
        fs::write(&package_manifest, b"{\"version\":\"0.146.0\"}\n")
            .expect("write pinned package manifest");
        fs::write(&managed_package_manifest, b"{\"version\":\"0.146.0\"}\n")
            .expect("write managed package manifest");
        let resources = PinnedCodexResources::new(
            managed_root.clone(),
            resources_directory.clone(),
            sha256(&fs::read(&sandbox_setup).expect("read setup helper")),
            sha256(&fs::read(&command_runner).expect("read runner helper")),
            sha256(&fs::read(&package_manifest).expect("read package manifest")),
            sha256(&fs::read(&managed_package_manifest).expect("read managed package manifest")),
        )
        .expect("bind pinned resource identity");
        let launch_log = root.join("child-launched.txt");
        let server = bin.join("fake-app-server.ps1");
        let quote = |path: &std::path::Path| path.display().to_string().replace('\'', "''");
        let reported_home = codex_home.display().to_string().replace('\\', r"\\");
        let script = format!(
            "$ErrorActionPreference = 'Stop'\n$log = '{}'\n[IO.File]::WriteAllText($log, 'spawned')\nif ($env:CODEX_MANAGED_PACKAGE_ROOT -ne '{}') {{ [IO.File]::WriteAllText($log, 'root:' + $env:CODEX_MANAGED_PACKAGE_ROOT); exit 51 }}\n$first = ($env:PATH -split ';')[0]\nif ($first -ne '{}') {{ [IO.File]::WriteAllText($log, 'path:' + $first); exit 52 }}\n$matches = @(Get-Command 'codex-windows-sandbox-setup.exe' -CommandType Application -ErrorAction Stop)\nif ($matches.Count -ne 1) {{ [IO.File]::WriteAllText($log, 'matches:' + $matches.Count); exit 54 }}\n$resolved = $matches[0].Source\nif ($resolved -ne '{}') {{ [IO.File]::WriteAllText($log, 'resolved:' + $resolved); exit 53 }}\n[IO.File]::WriteAllText($log, 'verified')\n$null = [Console]::In.ReadLine()\n[Console]::Out.WriteLine('{{\"id\":0,\"result\":{{\"userAgent\":\"codex_cli_rs/0.146.0\",\"platformFamily\":\"windows\",\"platformOs\":\"windows\",\"codexHome\":\"{}\"}}}}')\n$null = [Console]::In.ReadLine()\n$null = [Console]::In.ReadLine()\n[Console]::Out.WriteLine('{{\"id\":1,\"result\":{{\"thread\":{{\"id\":\"thread-pinned\"}}}}}}')\n$null = [Console]::In.ReadLine()\n[Console]::Out.WriteLine('{{\"id\":2,\"result\":{{\"turn\":{{\"id\":\"turn-pinned\"}}}}}}')\n[Console]::Out.WriteLine('{{\"method\":\"turn/completed\",\"params\":{{\"threadId\":\"thread-pinned\",\"turn\":{{\"id\":\"turn-pinned\",\"items\":[],\"status\":\"completed\",\"error\":null}}}}}}')\nStart-Sleep -Seconds 60\n",
            quote(&launch_log),
            quote(&managed_root),
            quote(&resources_directory),
            quote(&sandbox_setup),
            reported_home,
        );
        fs::write(&server, script).expect("write pinned fake app-server");
        let launcher = bin.join("fake-codex.cmd");
        fs::write(
            &launcher,
            "@echo off\r\n\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"%~dp0fake-app-server.ps1\" %*\r\n",
        )
        .expect("write pinned fake launcher");
        let launcher_sha256 = sha256(&fs::read(&launcher).expect("read pinned launcher"));
        Self {
            root,
            launcher,
            codex_home,
            working_directory,
            resources,
            sandbox_setup,
            command_runner,
            launch_log,
            launcher_sha256,
        }
    }

    fn config(&self) -> AppServerRunConfig {
        AppServerRunConfig::new(
            self.launcher.clone(),
            self.launcher_sha256.clone(),
            self.codex_home.clone(),
            self.working_directory.clone(),
            "Create answer.txt",
            Duration::from_secs(5),
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
    assert!(fixture.launch_log.is_file());

    fs::remove_file(&fixture.launch_log).expect("reset child launch evidence");
    let cross_version_root = fixture.root.join("managed-package-0.144.6");
    fs::create_dir(&cross_version_root).expect("create cross-version package root");
    let cross_version = PinnedCodexResources::new(
        cross_version_root,
        fixture.resources.resources_directory().to_path_buf(),
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
        "d".repeat(64),
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
    let completed = r#"{"method":"turn/completed","params":{"threadId":"thread-scripted","turn":{"id":"turn-scripted","items":[{"arguments":{},"id":"tool-apply","status":"completed","success":true,"tool":"exec","type":"dynamicToolCall"},{"arguments":{},"id":"tool-verify","status":"completed","success":true,"tool":"exec","type":"dynamicToolCall"}],"itemsView":"full","status":"completed","error":null}}}"#;
    let tail = match mode {
        FakeMode::Success | FakeMode::WrongHome => format!(
            "[Console]::Out.WriteLine('{completed}')\nStart-Sleep -Seconds 60\n"
        ),
        FakeMode::Yielded => concat!(
            r#"[Console]::Out.WriteLine('{"method":"turn/completed","params":{"threadId":"thread-scripted","turn":{"id":"turn-scripted","items":[{"arguments":{},"contentItems":[{"text":"Script running with cell ID cell-7","type":"inputText"}],"id":"tool-exec","status":"completed","success":true,"tool":"exec","type":"dynamicToolCall"}],"itemsView":"full","status":"completed","error":null}}}')"#,
            "\n",
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
            "$grandchild = \"Start-Sleep -Milliseconds 800; [IO.File]::WriteAllText('{}', 'survived')\"\n$encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($grandchild))\n$descendant = Start-Process -FilePath \"$PSHOME\\powershell.exe\" -WindowStyle Hidden -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-EncodedCommand',$encoded) -PassThru\n[IO.File]::WriteAllText('{}', [string]$descendant.Id)\n[Console]::Out.WriteLine('{completed}')\nexit 0\n",
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
