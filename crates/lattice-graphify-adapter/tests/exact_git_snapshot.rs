use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, ContentDigest, GitObjectId, GraphMemoryRunRequest, Invocation,
    ProjectId, ProjectSnapshotId, RequestId, TaskId,
};
use lattice_graphify_adapter::{
    ExactGitSnapshotMaterializer, GRAPHIFY_LICENSE, GRAPHIFY_PACKAGE, GRAPHIFY_UPSTREAM_COMMIT,
    GRAPHIFY_VERSION, GRAPHIFY_WHEEL_SHA256, GRAPHIFY_WSL_EXECUTION_IDENTITY_SHA256,
    GRAPHIFY_WSL_GRAPHIFY_HELP_SHA256, GRAPHIFY_WSL_INSTALL_REPORT_SHA256,
    GRAPHIFY_WSL_LAUNCHER_SHA256, GRAPHIFY_WSL_RUNTIME_BYTE_COUNT, GRAPHIFY_WSL_RUNTIME_FILE_COUNT,
    GRAPHIFY_WSL_RUNTIME_MANIFEST_SHA256, GitSnapshotConfig, GraphOutputLimits,
    GraphifyRuntimeConfig, PinnedGraphifyAdapter, SnapshotBridge, SnapshotLimits,
};
use lattice_ports::{CodeSnapshotPort, GraphifyAnalysisPort};
use sha2::{Digest, Sha256};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);
const HEX: &[u8; 16] = b"0123456789abcdef";

#[test]
fn exact_git_snapshot_ignores_worktree_and_changes_only_with_commit() {
    let git = find_git();
    let git_sha256 = sha256_file(&git);
    let fixture = TestDirectory::new("exact-git-snapshot");
    let repository = fixture.path().join("repository");
    let snapshots = fixture.path().join("snapshots");
    fs::create_dir_all(repository.join("src")).expect("create fixture source directory");
    fs::create_dir_all(&snapshots).expect("create snapshot directory");

    git_ok(&git, &repository, &["init", "--quiet"]);
    git_ok(&git, &repository, &["config", "user.name", "LATTICE Test"]);
    git_ok(
        &git,
        &repository,
        &["config", "user.email", "lattice-test@example.invalid"],
    );
    git_ok(&git, &repository, &["config", "core.autocrlf", "false"]);

    fs::write(repository.join("src/z.rs"), b"pub fn z() -> u8 { 1 }\n").expect("write z source");
    fs::write(repository.join("src/a.rs"), b"pub fn a() -> u8 { 1 }\n").expect("write a source");
    fs::write(repository.join(".env"), b"API_TOKEN=tracked-secret\n")
        .expect("write tracked secret");
    fs::write(repository.join("README.md"), b"non-code\n").expect("write non-code file");
    git_ok(&git, &repository, &["add", "--all"]);
    git_ok(
        &git,
        &repository,
        &["commit", "--quiet", "-m", "fixture one"],
    );
    let commit_one = git_text(&git, &repository, &["rev-parse", "HEAD"]);

    let materializer = ExactGitSnapshotMaterializer::new(
        GitSnapshotConfig::new(
            &git,
            &git_sha256,
            &repository,
            &snapshots,
            SnapshotLimits::default(),
        )
        .expect("valid snapshot config"),
    );

    // Deliberately diverge the worktree before materialization. The snapshot
    // must still come exclusively from the exact commit's Git objects.
    fs::write(repository.join("src/a.rs"), b"pub fn a() -> u8 { 2 }\n")
        .expect("change tracked worktree source");
    fs::write(
        repository.join("src/untracked.rs"),
        b"pub const UNTRACKED_SECRET: &str = \"do-not-copy\";\n",
    )
    .expect("write untracked source");

    let first = materializer
        .materialize(&commit_one)
        .expect("materialize first exact commit");
    let first_repeat = materializer
        .materialize(&commit_one)
        .expect("repeat first exact commit");

    assert_eq!(first.commit_id(), commit_one);
    assert_eq!(
        first
            .sources()
            .iter()
            .map(lattice_graphify_adapter::SnapshotSource::relative_path)
            .collect::<Vec<_>>(),
        vec!["src/a.rs", "src/z.rs"]
    );
    assert_eq!(
        fs::read(first.root().join("src/a.rs")).expect("read snapshotted source"),
        b"pub fn a() -> u8 { 1 }\n"
    );
    assert!(!first.root().join(".env").exists());
    assert!(!first.root().join("README.md").exists());
    assert!(!first.root().join("src/untracked.rs").exists());
    assert!(first.excluded_path_count() >= 2);
    assert_eq!(first.manifest_sha256(), first_repeat.manifest_sha256());
    assert_eq!(first.exclusion_sha256(), first_repeat.exclusion_sha256());
    assert_eq!(first.snapshot_sha256(), first_repeat.snapshot_sha256());

    let snapshot_count_before_rejection = snapshot_directory_count(&snapshots);
    let missing_commit = "0".repeat(40);
    assert!(materializer.materialize(&missing_commit).is_err());
    assert_eq!(
        snapshot_directory_count(&snapshots),
        snapshot_count_before_rejection,
        "a rejected commit must not create a partial snapshot"
    );

    git_ok(&git, &repository, &["add", "src/a.rs"]);
    git_ok(
        &git,
        &repository,
        &["commit", "--quiet", "-m", "fixture two"],
    );
    let commit_two = git_text(&git, &repository, &["rev-parse", "HEAD"]);
    assert_ne!(commit_one, commit_two);

    let second = materializer
        .materialize(&commit_two)
        .expect("materialize changed exact commit");
    assert_eq!(second.commit_id(), commit_two);
    assert_ne!(first.tree_id(), second.tree_id());
    assert_ne!(first.manifest_sha256(), second.manifest_sha256());
    assert_ne!(first.snapshot_sha256(), second.snapshot_sha256());
    assert_eq!(
        fs::read(second.root().join("src/a.rs")).expect("read changed snapshotted source"),
        b"pub fn a() -> u8 { 2 }\n"
    );
    assert!(!second.root().join("src/untracked.rs").exists());
}

#[test]
fn exact_git_snapshot_disables_replace_objects_for_tree_and_blob_reads() {
    let git = find_git();
    let fixture = TestDirectory::new("replace-object-regression");
    let repository = fixture.path().join("repository");
    let snapshots = fixture.path().join("snapshots");
    fs::create_dir_all(repository.join("src")).expect("create replace fixture source directory");
    fs::create_dir_all(&snapshots).expect("create replace snapshot directory");

    git_ok(&git, &repository, &["init", "--quiet"]);
    git_ok(&git, &repository, &["config", "user.name", "LATTICE Test"]);
    git_ok(
        &git,
        &repository,
        &["config", "user.email", "lattice-test@example.invalid"],
    );
    git_ok(&git, &repository, &["config", "core.autocrlf", "false"]);
    fs::write(repository.join("src/lib.rs"), b"pub const VALUE: u8 = 1;\n")
        .expect("write original source");
    git_ok(&git, &repository, &["add", "src/lib.rs"]);
    git_ok(
        &git,
        &repository,
        &["commit", "--quiet", "-m", "original commit"],
    );
    let requested_commit = git_text(&git, &repository, &["rev-parse", "HEAD"]);
    let requested_tree = git_text(&git, &repository, &["rev-parse", "HEAD^{tree}"]);

    fs::write(
        repository.join("src/lib.rs"),
        b"pub const VALUE: u8 = 99;\n",
    )
    .expect("write replacement source");
    git_ok(&git, &repository, &["add", "src/lib.rs"]);
    git_ok(
        &git,
        &repository,
        &["commit", "--quiet", "-m", "replacement commit"],
    );
    let replacement_commit = git_text(&git, &repository, &["rev-parse", "HEAD"]);
    git_ok(
        &git,
        &repository,
        &["replace", &requested_commit, &replacement_commit],
    );

    let materializer = ExactGitSnapshotMaterializer::new(
        GitSnapshotConfig::new(
            &git,
            sha256_file(&git),
            &repository,
            &snapshots,
            SnapshotLimits::default(),
        )
        .expect("valid replace regression config"),
    );
    let snapshot = materializer
        .materialize(&requested_commit)
        .expect("materialize exact commit without replace substitution");

    assert_eq!(snapshot.commit_id(), requested_commit);
    assert_eq!(snapshot.tree_id(), requested_tree);
    assert_eq!(
        fs::read(snapshot.root().join("src/lib.rs")).expect("read exact original blob"),
        b"pub const VALUE: u8 = 1;\n"
    );
}

#[test]
#[ignore = "requires the reviewed WSL launcher and parent Graphify WSL runtime"]
// Keeping the live fixture, composition, and assertions together makes this
// ignored executable acceptance gate reproducible as one exact command.
#[allow(clippy::too_many_lines)]
fn pinned_graphify_live_typed_ports_are_provenance_bound_and_deterministic() {
    let wsl = std::env::var_os("LATTICE_TEST_WSL_EXE")
        .map(PathBuf::from)
        .expect("set LATTICE_TEST_WSL_EXE to the reviewed Windows WSL launcher");
    let wsl = fs::canonicalize(wsl).expect("resolve reviewed WSL launcher");
    let runtime = std::env::var_os("LATTICE_TEST_GRAPHIFY_WSL_RUNTIME")
        .map(PathBuf::from)
        .expect("set LATTICE_TEST_GRAPHIFY_WSL_RUNTIME to the parent wsl-runtime directory");
    let runtime = fs::canonicalize(runtime).expect("resolve reviewed Graphify WSL runtime");
    assert_eq!(
        sha256_file(&wsl),
        GRAPHIFY_WSL_LAUNCHER_SHA256,
        "the live launcher must match the reviewed Windows WSL artifact"
    );
    assert_eq!(GRAPHIFY_PACKAGE, "graphifyy");
    assert_eq!(GRAPHIFY_VERSION, "0.9.33");
    assert_eq!(
        GRAPHIFY_UPSTREAM_COMMIT,
        "4e7e6b1f7e0df10ed07d5f28f9189bbde42940f1"
    );
    assert_eq!(GRAPHIFY_LICENSE, "Apache-2.0");
    assert_eq!(GRAPHIFY_WSL_RUNTIME_FILE_COUNT, 2_184);
    assert_eq!(GRAPHIFY_WSL_RUNTIME_BYTE_COUNT, 159_411_927);
    assert_eq!(
        GRAPHIFY_WSL_RUNTIME_MANIFEST_SHA256,
        "8e21411001d9f44e90ae4cf13f5e5fc1e15604bd868a75def47ad17bd31cb9d3"
    );
    assert_eq!(
        GRAPHIFY_WSL_INSTALL_REPORT_SHA256,
        "9901209d4cf415c16b030b8e1adeea6b216953df61115e3d9d32686ddd25a45e"
    );
    assert_eq!(
        GRAPHIFY_WHEEL_SHA256,
        "c32b5792c783a6e66b1100b35bc65df3538e3f69b9df45fb098c9634c1b8eb01"
    );
    assert_eq!(
        GRAPHIFY_WSL_EXECUTION_IDENTITY_SHA256,
        "f270004749c7f4fc260dfc09925b52f3b7071bcc64ba5f7cbd9bd37ae1400dd5"
    );

    let git = find_git();
    let fixture = TestDirectory::new("live-typed-ports");
    let repository = fixture.path().join("repository");
    let snapshots = fixture.path().join("snapshots");
    let staging = fixture.path().join("staging");
    fs::create_dir_all(repository.join("src")).expect("create live fixture source directory");
    fs::create_dir_all(&snapshots).expect("create live snapshot root");
    fs::create_dir_all(&staging).expect("create live Graphify staging root");

    git_ok(&git, &repository, &["init", "--quiet"]);
    git_ok(&git, &repository, &["config", "user.name", "LATTICE Test"]);
    git_ok(
        &git,
        &repository,
        &["config", "user.email", "lattice-test@example.invalid"],
    );
    git_ok(&git, &repository, &["config", "core.autocrlf", "false"]);
    fs::write(
        repository.join("src/lib.rs"),
        concat!(
            "mod service;\n\n",
            "pub use service::{DeliverySummary, summarize_delivery};\n\n",
            "pub fn render_delivery(task_id: &str, changed_files: usize) -> String {\n",
            "    let summary = summarize_delivery(task_id, changed_files);\n",
            "    format!(\"{}:{}\", summary.task_id, summary.changed_files)\n",
            "}\n",
        ),
    )
    .expect("write live lib source");
    fs::write(
        repository.join("src/service.rs"),
        concat!(
            "#[derive(Debug, Clone, PartialEq, Eq)]\n",
            "pub struct DeliverySummary {\n",
            "    pub task_id: String,\n",
            "    pub changed_files: usize,\n",
            "}\n\n",
            "pub fn summarize_delivery(task_id: &str, changed_files: usize) -> DeliverySummary {\n",
            "    DeliverySummary { task_id: task_id.to_owned(), changed_files }\n",
            "}\n",
        ),
    )
    .expect("write live service source");
    fs::write(repository.join(".env"), b"PROVIDER_TOKEN=must-not-leak\n")
        .expect("write live tracked secret");
    git_ok(&git, &repository, &["add", "--all"]);
    git_ok(
        &git,
        &repository,
        &["commit", "--quiet", "-m", "live graphify fixture"],
    );
    let commit = git_text(&git, &repository, &["rev-parse", "HEAD"]);

    let request = graph_memory_request(&commit);
    let bridge = SnapshotBridge::new();
    let mut snapshot_port = ExactGitSnapshotMaterializer::with_bridge(
        GitSnapshotConfig::new(
            &git,
            sha256_file(&git),
            &repository,
            &snapshots,
            SnapshotLimits::default(),
        )
        .expect("valid live snapshot config"),
        bridge.clone(),
    );
    let graphify_config = GraphifyRuntimeConfig::new(
        &wsl,
        &runtime,
        &staging,
        Duration::from_mins(1),
        GraphOutputLimits::default(),
    )
    .expect("valid pinned Graphify runtime config");
    let expected_capability = graphify_config.capability_sha256();
    let mut graphify_port = PinnedGraphifyAdapter::new(graphify_config, bridge);

    let snapshot = snapshot_port
        .materialize_snapshot(&request)
        .expect("materialize typed exact snapshot");
    assert_eq!(snapshot.commit_id().as_str(), commit);
    assert_eq!(
        snapshot
            .sources()
            .iter()
            .map(lattice_contracts::TrackedSource::relative_path)
            .collect::<Vec<_>>(),
        vec!["src/lib.rs", "src/service.rs"]
    );

    let first = graphify_port
        .analyze(&request, &snapshot)
        .expect("run pinned Graphify through typed port");
    let repeat = graphify_port
        .analyze(&request, &snapshot)
        .expect("repeat pinned Graphify through typed port");

    assert!(!first.nodes().is_empty());
    assert_eq!(
        first, repeat,
        "identical exact input must yield identical evidence"
    );
    assert_eq!(first.identity().package(), "graphifyy");
    assert_eq!(first.identity().version(), "0.9.33");
    assert_eq!(first.identity().license(), "Apache-2.0");
    assert_eq!(
        first.identity().executable_digest().as_str(),
        GRAPHIFY_WSL_EXECUTION_IDENTITY_SHA256
    );
    assert_eq!(
        first.identity().cli_help_digest().as_str(),
        GRAPHIFY_WSL_GRAPHIFY_HELP_SHA256
    );
    assert_eq!(
        first.identity().capability_digest().as_str(),
        expected_capability
    );
    assert!(
        first
            .nodes()
            .iter()
            .all(|node| snapshot
                .sources()
                .iter()
                .any(
                    |source| source.relative_path() == node.provenance().relative_path()
                        && source.content_digest() == node.provenance().content_digest()
                ))
    );
    assert!(
        first
            .edges()
            .iter()
            .all(|edge| snapshot
                .sources()
                .iter()
                .any(
                    |source| source.relative_path() == edge.provenance().relative_path()
                        && source.content_digest() == edge.provenance().content_digest()
                ))
    );
}

#[test]
#[ignore = "copies the reviewed WSL runtime to prove dependency tamper rejection"]
fn pinned_graphify_runtime_rejects_tampered_dependency_before_identity_claim() {
    let wsl = std::env::var_os("LATTICE_TEST_WSL_EXE")
        .map(PathBuf::from)
        .expect("set reviewed Windows WSL launcher");
    let wsl = fs::canonicalize(wsl).expect("resolve reviewed WSL launcher");
    let source_runtime = std::env::var_os("LATTICE_TEST_GRAPHIFY_WSL_RUNTIME")
        .map(PathBuf::from)
        .expect("set parent Graphify WSL runtime");
    let source_runtime =
        fs::canonicalize(source_runtime).expect("resolve reviewed Graphify WSL runtime");
    let fixture = TestDirectory::new("tampered-runtime-identity");
    let copied_runtime = fixture.path().join("runtime");
    copy_tree(&source_runtime, &copied_runtime);

    let verified = GraphifyRuntimeConfig::new(
        &wsl,
        &copied_runtime,
        fixture.path().join("verified-staging"),
        Duration::from_mins(1),
        GraphOutputLimits::default(),
    )
    .expect("exact copied payload remains reviewed");
    assert_eq!(
        verified.expected_payload_manifest_sha256(),
        Some(GRAPHIFY_WSL_RUNTIME_MANIFEST_SHA256)
    );

    let tampered_module = copied_runtime.join("site-packages/networkx/__init__.py");
    let mut tampered_bytes = fs::read(&tampered_module).expect("read copied module");
    let first = tampered_bytes
        .first_mut()
        .expect("networkx dependency module is non-empty");
    *first ^= 1;
    fs::write(&tampered_module, tampered_bytes).expect("tamper copied module");
    let rejected = GraphifyRuntimeConfig::new(
        &wsl,
        &copied_runtime,
        fixture.path().join("rejected-staging"),
        Duration::from_mins(1),
        GraphOutputLimits::default(),
    )
    .expect_err("tampered payload must not create an official Graphify identity path");
    assert_eq!(
        rejected.kind(),
        lattice_graphify_adapter::GraphifyAdapterErrorKind::GraphifyIdentity
    );
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create copied payload directory");
    for entry in fs::read_dir(source).expect("enumerate source payload") {
        let entry = entry.expect("read source payload entry");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type().expect("read source payload type");
        if file_type.is_dir() {
            copy_tree(&source_path, &destination_path);
        } else if file_type.is_file() {
            copy_file(&source_path, &destination_path);
        } else {
            panic!("unexpected source payload entry type");
        }
    }
}

fn copy_file(source: &Path, destination: &Path) {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).expect("create copied payload parent");
    }
    fs::copy(source, destination).expect("copy payload file");
}

fn graph_memory_request(commit: &str) -> GraphMemoryRunRequest {
    let invocation = Invocation::new(
        CONTRACT_VERSION,
        RequestId::new("request-live-graphify").expect("request id"),
        TaskId::new("task-033-live-graphify").expect("task id"),
        AttemptId::new("attempt-one").expect("attempt id"),
        ProjectSnapshotId::new("snapshot-live-graphify").expect("project snapshot id"),
        digest_bytes(b"live-graphify-subject"),
    )
    .expect("valid invocation");
    GraphMemoryRunRequest::new(
        invocation,
        ProjectId::new("graphify-live-fixture").expect("project id"),
        GitObjectId::new(commit).expect("exact commit id"),
        digest_bytes(b"structural-code-query-v1"),
        digest_bytes(b"graphify-live-config-v1"),
        10,
    )
    .expect("valid graph-memory request")
}

fn digest_bytes(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_sha256(hex_digest(Sha256::digest(bytes))).expect("valid content digest")
}

fn find_git() -> PathBuf {
    let executable_name = if cfg!(windows) { "git.exe" } else { "git" };
    let path = std::env::var_os("PATH").expect("PATH is required for the Git integration test");
    std::env::split_paths(&path)
        .map(|directory| directory.join(executable_name))
        .find(|candidate| candidate.is_file())
        .and_then(|candidate| fs::canonicalize(candidate).ok())
        .expect("an absolute Git executable is required for the integration test")
}

fn git_ok(git: &Path, repository: &Path, arguments: &[&str]) {
    let output = Command::new(git)
        .current_dir(repository)
        .args(arguments)
        .output()
        .expect("start Git fixture command");
    assert!(
        output.status.success(),
        "Git fixture command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_text(git: &Path, repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new(git)
        .current_dir(repository)
        .args(arguments)
        .output()
        .expect("start Git fixture query");
    assert!(
        output.status.success(),
        "Git fixture query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("Git fixture output is UTF-8")
        .trim()
        .to_owned()
}

fn sha256_file(path: &Path) -> String {
    let bytes = fs::read(path).expect("read Git executable for identity binding");
    hex_digest(Sha256::digest(bytes))
}

fn hex_digest(digest: impl IntoIterator<Item = u8>) -> String {
    let mut output = String::with_capacity(64);
    for byte in digest {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn snapshot_directory_count(root: &Path) -> usize {
    fs::read_dir(root)
        .expect("enumerate snapshot root")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir())
                && entry.file_name().to_string_lossy().starts_with("snapshot-")
        })
        .count()
}

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new(label: &str) -> Self {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "lattice-graphify-{label}-{}-{sequence}",
            std::process::id()
        ));
        if path.exists() {
            make_writable(&path);
            fs::remove_dir_all(&path).expect("remove stale fixture directory");
        }
        fs::create_dir(&path).expect("create fixture directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        make_writable(&self.path);
        let _ = fs::remove_dir_all(&self.path);
    }
}

// Windows read-only is a file attribute, not Unix mode-bit widening.
#[cfg_attr(windows, allow(clippy::permissions_set_readonly_false))]
fn make_writable(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            make_writable(&path);
        }
        #[cfg(windows)]
        if let Ok(metadata) = fs::metadata(&path) {
            let mut permissions = metadata.permissions();
            permissions.set_readonly(false);
            let _ = fs::set_permissions(&path, permissions);
        }
    }
}
