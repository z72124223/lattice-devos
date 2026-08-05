#![cfg(windows)]

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

const DISTRO: &str = "Ubuntu";
const BWRAP: &str = "/usr/bin/bwrap";
const PYTHON: &str = "/usr/bin/python3.14";

#[test]
#[ignore = "requires the reviewed local WSL2 Ubuntu and /usr/bin/bwrap containment runtime"]
#[allow(clippy::too_many_lines)]
fn bwrap_child_can_write_output_but_not_bound_inputs_or_unbound_host() {
    let fixture = TestDirectory::new("access");
    let runtime = fixture.path().join("runtime");
    let snapshot = fixture.path().join("snapshot");
    let output = fixture.path().join("output");
    let sibling = fixture.path().join("host-sibling-secret.txt");
    fs::create_dir_all(runtime.join("site-packages")).expect("create read-only runtime fixture");
    fs::create_dir_all(&snapshot).expect("create read-only snapshot fixture");
    fs::create_dir_all(&output).expect("create writable output fixture");
    fs::write(
        runtime.join("site-packages/identity.txt"),
        b"reviewed-runtime\n",
    )
    .expect("write runtime marker");
    fs::write(runtime.join("install-report.json"), b"{}\n").expect("write install report marker");
    fs::write(snapshot.join("source.rs"), b"pub fn exact() {}\n").expect("write exact source");
    fs::write(&sibling, b"must remain outside the sandbox\n").expect("write host sibling");

    let sibling_wsl = windows_path_to_wsl(&sibling);
    let script = r"
import os
import pathlib
import socket
import sys

source = pathlib.Path('/source/source.rs')
runtime = pathlib.Path('/runtime/site-packages/identity.txt')
output = pathlib.Path('/output/inside.txt')
host_sibling = pathlib.Path(sys.argv[1])

output.write_text('inside sandbox\n', encoding='utf-8')
if source.read_text(encoding='utf-8') != 'pub fn exact() {}\n':
    raise SystemExit(40)
try:
    source.write_text('tampered\n', encoding='utf-8')
except OSError:
    pass
else:
    raise SystemExit(41)
try:
    runtime.write_text('tampered\n', encoding='utf-8')
except OSError:
    pass
else:
    raise SystemExit(46)
if host_sibling.exists():
    raise SystemExit(42)
if os.environ.get('OPENAI_API_KEY') or os.environ.get('LATTICE_TEST_HOST_SENTINEL'):
    raise SystemExit(43)

mounts = {}
for line in pathlib.Path('/proc/self/mountinfo').read_text(encoding='utf-8').splitlines():
    fields = line.split()
    if len(fields) > 5 and fields[4] in {'/runtime/site-packages', '/source', '/output'}:
        mounts[fields[4]] = fields[5].split(',')[0]
if mounts != {'/runtime/site-packages': 'ro', '/source': 'ro', '/output': 'rw'}:
    raise SystemExit(47)

interfaces = []
for line in pathlib.Path('/proc/net/dev').read_text(encoding='utf-8').splitlines()[2:]:
    interfaces.append(line.split(':', 1)[0].strip())
if any(interface != 'lo' for interface in interfaces):
    raise SystemExit(44)

probe = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
probe.settimeout(0.25)
try:
    result = probe.connect_ex(('1.1.1.1', 53))
finally:
    probe.close()
if result == 0:
    raise SystemExit(45)
";

    let mut command = fixed_bwrap_command(&runtime, &snapshot, &output);
    command
        .args([
            OsString::from(PYTHON),
            OsString::from("-I"),
            OsString::from("-c"),
        ])
        .arg(script)
        .arg(sibling_wsl)
        .env("OPENAI_API_KEY", "must-not-cross-clearenv")
        .env("LATTICE_TEST_HOST_SENTINEL", "must-not-cross-clearenv");
    let outcome = run_bounded(command, fixture.path(), Duration::from_secs(15));

    assert!(!outcome.timed_out, "containment probe exceeded its bound");
    assert!(
        outcome.status.success(),
        "containment probe failed: status={:?} stdout={} stderr={}",
        outcome.status.code(),
        String::from_utf8_lossy(&outcome.stdout),
        String::from_utf8_lossy(&outcome.stderr)
    );
    assert_eq!(
        fs::read(output.join("inside.txt")).expect("read sandbox-owned output"),
        b"inside sandbox\n"
    );
    assert_eq!(
        fs::read(snapshot.join("source.rs")).expect("read exact source after probe"),
        b"pub fn exact() {}\n",
        "the read-only source bind must remain unchanged"
    );
    assert_eq!(
        fs::read(runtime.join("site-packages/identity.txt")).expect("read runtime after probe"),
        b"reviewed-runtime\n",
        "the read-only runtime bind must remain unchanged"
    );
    assert_eq!(
        fs::read(&sibling).expect("read host sibling after probe"),
        b"must remain outside the sandbox\n",
        "an unbound host sibling must remain unchanged"
    );
}

#[test]
#[ignore = "requires the reviewed local WSL2 Ubuntu and /usr/bin/bwrap containment runtime"]
fn killing_wsl_client_reaps_bwrap_pid_namespace_before_late_marker() {
    let fixture = TestDirectory::new("timeout");
    let runtime = fixture.path().join("runtime");
    let snapshot = fixture.path().join("snapshot");
    let output = fixture.path().join("output");
    fs::create_dir_all(runtime.join("site-packages")).expect("create runtime fixture");
    fs::write(runtime.join("install-report.json"), b"{}\n").expect("install report fixture");
    fs::create_dir_all(&snapshot).expect("create snapshot fixture");
    fs::create_dir_all(&output).expect("create output fixture");

    // Both processes have short natural lifetimes so a failing assertion cannot
    // leave an unbounded process behind in the user's WSL distribution.
    let script = r#"
import pathlib
import subprocess
import sys
import time

marker = '/output/late-marker.txt'
descendant = "import pathlib,time; time.sleep(2); pathlib.Path('/output/late-marker.txt').write_text('survived\\n', encoding='utf-8')"
subprocess.Popen([sys.executable, '-I', '-c', descendant], close_fds=True)
pathlib.Path('/output/started.txt').write_text('started\n', encoding='utf-8')
time.sleep(4)
"#;
    let mut command = fixed_bwrap_command(&runtime, &snapshot, &output);
    command
        .args([
            OsString::from(PYTHON),
            OsString::from("-I"),
            OsString::from("-c"),
        ])
        .arg(script)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .expect("spawn direct WSL/bwrap timeout probe");

    wait_for_file_or_child_exit(
        &mut child,
        &output.join("started.txt"),
        Duration::from_secs(10),
    );
    child
        .kill()
        .expect("terminate the exact WSL client process");
    wait_for_exit(&mut child, Duration::from_secs(3));
    thread::sleep(Duration::from_millis(2_500));

    assert!(
        !output.join("late-marker.txt").exists(),
        "a descendant survived termination and wrote the late marker"
    );
}

#[test]
#[ignore = "requires LATTICE_TEST_GRAPHIFY_WSL_RUNTIME_ROOT plus reviewed WSL2 Ubuntu/bwrap"]
fn pinned_graphify_extract_runs_in_the_fixed_bwrap_mount_shape() {
    let runtime = std::env::var_os("LATTICE_TEST_GRAPHIFY_WSL_RUNTIME_ROOT")
        .map(PathBuf::from)
        .expect("set LATTICE_TEST_GRAPHIFY_WSL_RUNTIME_ROOT to the pinned Linux runtime root");
    let runtime = fs::canonicalize(runtime).expect("resolve pinned Linux Graphify runtime");
    let fixture = TestDirectory::new("graphify-live");
    let snapshot = fixture.path().join("snapshot");
    let output = fixture.path().join("output");
    fs::create_dir_all(snapshot.join("src")).expect("create live exact snapshot");
    fs::create_dir_all(&output).expect("create live output root");
    fs::write(
        snapshot.join("src/lib.rs"),
        b"pub fn render_delivery(task: &str) -> String { task.to_owned() }\n",
    )
    .expect("write live source fixture");

    let mut command = fixed_bwrap_command(&runtime, &snapshot, &output);
    command.args([
        OsString::from(PYTHON),
        OsString::from("-P"),
        OsString::from("-B"),
        OsString::from("-m"),
        OsString::from("graphify"),
        OsString::from("extract"),
        OsString::from("/source"),
        OsString::from("--code-only"),
        OsString::from("--no-cluster"),
        OsString::from("--max-workers"),
        OsString::from("1"),
        OsString::from("--out"),
        // Graphify treats --out as the parent output root and appends its
        // fixed `graphify-out/` directory itself.
        OsString::from("/output"),
    ]);
    let outcome = run_bounded(command, fixture.path(), Duration::from_mins(1));

    assert!(
        !outcome.timed_out,
        "pinned Graphify extract exceeded 60 seconds"
    );
    assert!(
        outcome.status.success(),
        "pinned Graphify extract failed: status={:?} stdout={} stderr={}",
        outcome.status.code(),
        String::from_utf8_lossy(&outcome.stdout),
        String::from_utf8_lossy(&outcome.stderr)
    );
    let graph = output.join("graphify-out/graph.json");
    assert!(
        graph.is_file(),
        "Graphify must emit the fixed graph.json path"
    );
    let graph_bytes = fs::read(graph).expect("read live graph output");
    assert!(
        graph_bytes
            .windows(b"nodes".len())
            .any(|part| part == b"nodes"),
        "live graph output must contain the nodes field"
    );
    assert_eq!(
        fs::read(snapshot.join("src/lib.rs")).expect("read source after live extraction"),
        b"pub fn render_delivery(task: &str) -> String { task.to_owned() }\n"
    );
}

fn fixed_bwrap_command(runtime: &Path, snapshot: &Path, output: &Path) -> Command {
    let mut command = Command::new(fixed_wsl_executable());
    command.args([
        OsString::from("-d"),
        OsString::from(DISTRO),
        OsString::from("--exec"),
        OsString::from(BWRAP),
        OsString::from("--die-with-parent"),
        OsString::from("--unshare-all"),
        OsString::from("--unshare-user"),
        OsString::from("--disable-userns"),
        OsString::from("--assert-userns-disabled"),
        OsString::from("--new-session"),
        OsString::from("--cap-drop"),
        OsString::from("ALL"),
        OsString::from("--ro-bind"),
        OsString::from("/usr"),
        OsString::from("/usr"),
        OsString::from("--ro-bind"),
        OsString::from("/lib"),
        OsString::from("/lib"),
        OsString::from("--ro-bind"),
        OsString::from("/lib64"),
        OsString::from("/lib64"),
        OsString::from("--proc"),
        OsString::from("/proc"),
        OsString::from("--dev"),
        OsString::from("/dev"),
        OsString::from("--tmpfs"),
        OsString::from("/tmp"),
        OsString::from("--dir"),
        OsString::from("/home"),
        OsString::from("--dir"),
        OsString::from("/home/lattice"),
        OsString::from("--dir"),
        OsString::from("/runtime"),
        OsString::from("--ro-bind"),
        OsString::from(windows_path_to_wsl(&runtime.join("site-packages"))),
        OsString::from("/runtime/site-packages"),
        OsString::from("--ro-bind"),
        OsString::from(windows_path_to_wsl(&runtime.join("install-report.json"))),
        OsString::from("/runtime/install-report.json"),
        OsString::from("--ro-bind"),
        OsString::from(windows_path_to_wsl(snapshot)),
        OsString::from("/source"),
        OsString::from("--bind"),
        OsString::from(windows_path_to_wsl(output)),
        OsString::from("/output"),
        OsString::from("--clearenv"),
        OsString::from("--setenv"),
        OsString::from("PATH"),
        OsString::from("/usr/bin:/bin"),
        OsString::from("--setenv"),
        OsString::from("HOME"),
        OsString::from("/home/lattice"),
        OsString::from("--setenv"),
        OsString::from("PYTHONPATH"),
        OsString::from("/runtime/site-packages"),
        OsString::from("--setenv"),
        OsString::from("PYTHONPYCACHEPREFIX"),
        OsString::from("/tmp/pycache"),
        OsString::from("--setenv"),
        OsString::from("PYTHONDONTWRITEBYTECODE"),
        OsString::from("1"),
        OsString::from("--setenv"),
        OsString::from("PYTHONSAFEPATH"),
        OsString::from("1"),
        OsString::from("--setenv"),
        OsString::from("GRAPHIFY_QUERY_LOG_DISABLE"),
        OsString::from("1"),
        OsString::from("--chdir"),
        OsString::from("/output"),
    ]);
    command
}

fn fixed_wsl_executable() -> PathBuf {
    let system_root = std::env::var_os("SystemRoot").expect("SystemRoot is required on Windows");
    let candidate = PathBuf::from(system_root).join("System32/wsl.exe");
    fs::canonicalize(candidate).expect("resolve the absolute Windows wsl.exe")
}

fn windows_path_to_wsl(path: &Path) -> String {
    let canonical = fs::canonicalize(path).expect("resolve Windows bind source");
    let text = canonical.to_str().expect("bind source must be Unicode");
    let text = text.strip_prefix(r"\\?\").unwrap_or(text);
    let bytes = text.as_bytes();
    assert!(
        bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/'),
        "only absolute local drive paths may be translated for the containment fixture"
    );
    let drive = char::from(bytes[0].to_ascii_lowercase());
    let tail = text[3..].replace('\\', "/");
    format!("/mnt/{drive}/{tail}")
}

struct BoundedOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
}

fn run_bounded(mut command: Command, log_root: &Path, timeout: Duration) -> BoundedOutput {
    let stdout_path = log_root.join("wsl-stdout.log");
    let stderr_path = log_root.join("wsl-stderr.log");
    command.stdout(Stdio::from(
        File::create(&stdout_path).expect("create bounded stdout log"),
    ));
    command.stderr(Stdio::from(
        File::create(&stderr_path).expect("create bounded stderr log"),
    ));
    let mut child = command
        .spawn()
        .expect("spawn direct fixed WSL/bwrap command");
    let deadline = Instant::now()
        .checked_add(timeout)
        .expect("bounded timeout deadline");
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait().expect("poll fixed WSL/bwrap command") {
            break (status, false);
        }
        if Instant::now() >= deadline {
            child.kill().expect("terminate timed-out fixed WSL client");
            break (child.wait().expect("reap timed-out fixed WSL client"), true);
        }
        thread::sleep(Duration::from_millis(25));
    };
    BoundedOutput {
        status,
        stdout: read_file(&stdout_path),
        stderr: read_file(&stderr_path),
        timed_out,
    }
}

fn wait_for_file_or_child_exit(child: &mut Child, marker: &Path, timeout: Duration) {
    let deadline = Instant::now()
        .checked_add(timeout)
        .expect("bounded marker deadline");
    loop {
        if marker.is_file() {
            return;
        }
        if let Some(status) = child.try_wait().expect("poll timeout probe") {
            panic!("timeout probe exited before start marker: {status}");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("timeout probe did not create its start marker within {timeout:?}");
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn wait_for_exit(child: &mut Child, timeout: Duration) {
    let deadline = Instant::now()
        .checked_add(timeout)
        .expect("bounded reap deadline");
    loop {
        if child
            .try_wait()
            .expect("poll terminated WSL client")
            .is_some()
        {
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("terminated WSL client was not reaped within {timeout:?}");
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn read_file(path: &Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    File::open(path)
        .expect("open bounded process log")
        .read_to_end(&mut bytes)
        .expect("read bounded process log");
    bytes
}

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "lattice-wsl-bwrap-{label}-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("create WSL/bwrap test root");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.path.is_dir() {
            fs::remove_dir_all(&self.path).expect("remove bounded WSL/bwrap test root");
        }
    }
}

#[test]
fn command_shape_constants_are_fixed_and_not_shell_entrypoints() {
    assert_eq!(DISTRO, "Ubuntu");
    assert_eq!(BWRAP, "/usr/bin/bwrap");
    assert_eq!(PYTHON, "/usr/bin/python3.14");
    for executable in [BWRAP, PYTHON] {
        assert!(!matches!(
            Path::new(executable).file_name().and_then(OsStr::to_str),
            Some("sh" | "bash" | "dash" | "zsh")
        ));
    }
}
