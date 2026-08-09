use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use lattice_contracts::{
    AttemptId, ContentDigest, DaemonEpoch, GitRefIdentity, HolderProcessId, ProjectClass,
    ProjectId, RuntimeAdmissionMode, TaskId,
};
use lattice_project_registry::{
    CommandId, FakeProjectRegistry, RegistryCommand, RepositoryObservation,
};
use lattice_workspace_git::{
    ChangeOperation, ChangeState, CommandGitExecutor, GitExecutor, GitOutput, GitRequest,
    GitWorkspace, IntegrationOutcome, PathKind, WorkspaceAuthority, WorkspaceError,
};
use lattice_writer_lease::test_support::{digest, observation as lease_observation};
use lattice_writer_lease::{
    AcquireClaim, AcquireCommand, FakeWriterLease, HeartbeatCommand, WriterLeaseCommand,
};

static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Default)]
struct RecordingExecutor {
    requests: Arc<Mutex<Vec<Vec<OsString>>>>,
}

impl GitExecutor for RecordingExecutor {
    fn execute(&self, request: &GitRequest) -> Result<GitOutput, WorkspaceError> {
        self.requests
            .lock()
            .expect("recording lock")
            .push(request.args().to_vec());
        CommandGitExecutor.execute(request)
    }
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let serial = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "lattice-workspace-git-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create unique disposable root");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        if self.0.starts_with(std::env::temp_dir()) {
            fs::remove_dir_all(&self.0).expect("remove disposable fixture");
        }
    }
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run fixture git");
    assert!(
        output.status.success(),
        "fixture git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf8 fixture output")
        .trim()
        .to_owned()
}

fn git_fails(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run fixture git");
    assert!(
        !output.status.success(),
        "fixture Git command unexpectedly passed"
    );
}

fn disposable_repository(root: &Path) -> (PathBuf, String) {
    let repository = root.join("repository");
    fs::create_dir(&repository).expect("repository directory");
    git(&repository, &["init", "-b", "main"]);
    git(&repository, &["config", "user.name", "LATTICE Test"]);
    git(
        &repository,
        &["config", "user.email", "lattice-test@invalid.example"],
    );
    fs::write(repository.join("tracked.txt"), "base\n").expect("tracked fixture");
    fs::write(repository.join("old-name.txt"), "rename me\n").expect("rename fixture");
    fs::write(repository.join("delete-me.txt"), "delete me\n").expect("delete fixture");
    fs::write(repository.join("shared.txt"), "base\n").expect("shared fixture");
    fs::write(repository.join(".gitignore"), "*.ignored\n").expect("ignore fixture");
    git(&repository, &["add", "."]);
    git(&repository, &["commit", "-m", "initial"]);
    let base = git(&repository, &["rev-parse", "HEAD"]);
    (repository, base)
}

struct OwnerHarness {
    registry_observation: RepositoryObservation,
    registry_receipt: lattice_contracts::ProjectAuthorityReceipt,
    registry_head: lattice_contracts::ProjectAuthorityHead,
    project_id: ProjectId,
    lease_owner: FakeWriterLease,
}

impl OwnerHarness {
    fn authority(&self) -> WorkspaceAuthority {
        WorkspaceAuthority::bind(
            &self.registry_observation,
            self.registry_receipt.clone(),
            &self.registry_head,
            self.lease_owner
                .current_receipt(&self.project_id)
                .expect("lease receipt"),
            &self
                .lease_owner
                .current_head(&self.project_id)
                .expect("lease head"),
        )
        .expect("bind owner authority")
    }
}

fn owner_harness(repository: &Path) -> OwnerHarness {
    let project_id = ProjectId::new("project-a").expect("project id");
    let repository_root = repository.to_str().expect("utf8 fixture path");
    let registry_observation = RepositoryObservation::new(
        repository_root,
        digest('a'),
        digest('b'),
        digest('c'),
        GitRefIdentity::new("refs/heads/main", digest('d')).expect("primary ref"),
    )
    .expect("repository observation");
    let mut registry = FakeProjectRegistry::new();
    registry
        .execute(RegistryCommand::register(
            CommandId::new("register-project-a").expect("command id"),
            project_id.clone(),
            ProjectClass::UserProject,
            registry_observation.clone(),
        ))
        .expect("register project");
    let registry_receipt = registry
        .latest(&project_id)
        .expect("registry receipt")
        .clone();
    let registry_head = registry.current_head(&project_id).expect("registry head");

    let mut leases = FakeWriterLease::new();
    leases
        .execute(WriterLeaseCommand::Acquire(AcquireCommand {
            command_id: "acquire-project-a".to_owned(),
            expected_head: None,
            claim: AcquireClaim {
                project_id: project_id.clone(),
                project_snapshot_id: registry_receipt.project_snapshot_id().clone(),
                task_id: TaskId::new("TASK-026").expect("task id"),
                task_revision: "1".to_owned(),
                task_spec_digest: ContentDigest::from_sha256("1".repeat(64))
                    .expect("task spec digest"),
                attempt_id: AttemptId::new("attempt-026").expect("attempt id"),
                lease_id: "lease-026".to_owned(),
                lease_holder_id: "implementer-026".to_owned(),
                worktree_id: "worktree-026".to_owned(),
                holder_process_id: HolderProcessId::new(42).expect("process id"),
                holder_process_start_identity: digest('2'),
                daemon_instance_id: "daemon-026".to_owned(),
                daemon_epoch: DaemonEpoch::new(7).expect("daemon epoch"),
            },
            observation: lease_observation(RuntimeAdmissionMode::Active, "2026-08-09T00:00:00Z"),
            expires_at: "2026-08-09T00:10:00Z".to_owned(),
        }))
        .expect("acquire writer lease");

    OwnerHarness {
        registry_observation,
        registry_receipt,
        registry_head,
        project_id,
        lease_owner: leases,
    }
}

fn owner_authority(repository: &Path) -> WorkspaceAuthority {
    owner_harness(repository).authority()
}

#[test]
fn creates_exact_base_worktree_only_under_owned_root() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let authority = owner_authority(&repository);
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");

    let owned = workspace
        .create_worktree(&authority, &base)
        .expect("create exact worktree");

    assert!(owned.path().starts_with(&workspace_root));
    assert_eq!(git(owned.path(), &["rev-parse", "HEAD"]), base);
    assert_eq!(
        git(owned.path(), &["symbolic-ref", "--quiet", "HEAD"]),
        owned.branch_ref()
    );
    assert_eq!(git(owned.path(), &["status", "--porcelain=v1"]), "");
}

#[test]
fn every_git_process_uses_protected_argument_arrays() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let authority = owner_authority(&repository);
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let executor = RecordingExecutor::default();
    let requests = Arc::clone(&executor.requests);
    let workspace = GitWorkspace::new(&workspace_root, executor).expect("workspace boundary");

    workspace
        .create_worktree(&authority, &base)
        .expect("create exact worktree");

    let requests = requests.lock().expect("recorded requests");
    assert!(!requests.is_empty());
    for args in requests.iter() {
        let strings: Vec<String> = args
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(strings.first().map(String::as_str), Some("--no-pager"));
        assert_eq!(strings.get(1).map(String::as_str), Some("-c"));
        assert!(
            strings
                .get(2)
                .is_some_and(|value| value.starts_with("core.hooksPath="))
        );
        assert_eq!(
            &strings[3..7],
            ["-c", "core.fsmonitor=false", "-c", "diff.external="]
        );
        assert!(!strings.iter().any(|argument| matches!(
            argument.as_str(),
            "reset" | "clean" | "stash" | "--force" | "-f"
        )));
    }
}

#[test]
fn unsafe_local_git_configuration_fails_before_worktree_creation() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    git(
        &repository,
        &["config", "filter.danger.smudge", "dangerous-program"],
    );
    let authority = owner_authority(&repository);
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");

    let error = workspace
        .create_worktree(&authority, &base)
        .expect_err("external filter must fail closed");

    assert_eq!(error, WorkspaceError::UnsafeGitConfiguration);
    assert_eq!(
        fs::read_dir(workspace_root.join(".lattice-guards"))
            .expect("guard directory")
            .count(),
        0
    );
}

#[test]
fn reports_stable_complete_change_evidence_including_ignored_and_rename() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let authority = owner_authority(&repository);
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");
    let owned = workspace
        .create_worktree(&authority, &base)
        .expect("create exact worktree");

    fs::write(owned.path().join("tracked.txt"), "base\nunstaged\n").expect("unstaged modification");
    git(owned.path(), &["mv", "old-name.txt", "renamed.txt"]);
    fs::remove_file(owned.path().join("delete-me.txt")).expect("delete tracked fixture");
    fs::write(owned.path().join("staged.txt"), "staged\n").expect("staged fixture");
    git(owned.path(), &["add", "staged.txt"]);
    fs::write(owned.path().join("staged.txt"), "staged\nunstaged\n").expect("mixed staged fixture");
    fs::write(owned.path().join("untracked.txt"), "untracked\n").expect("untracked fixture");
    fs::write(owned.path().join("hidden.ignored"), "ignored\n").expect("ignored fixture");

    let first = workspace
        .changed_manifest(&authority, &owned)
        .expect("changed-path manifest");
    let second = workspace
        .changed_manifest(&authority, &owned)
        .expect("stable repeat manifest");

    assert_eq!(first, second);
    assert_eq!(first.base_commit().as_str(), base);
    assert_eq!(first.digest().as_str().len(), 64);
    let changes = first.changes();
    assert!(
        changes
            .windows(2)
            .all(|pair| pair[0].sort_key() <= pair[1].sort_key())
    );

    let tracked = changes
        .iter()
        .find(|change| change.destination_path() == Some("tracked.txt"))
        .expect("tracked modification");
    assert_eq!(tracked.operation(), ChangeOperation::Modify);
    assert_eq!(tracked.states(), &[ChangeState::Unstaged]);
    assert_eq!(tracked.destination_kind(), Some(PathKind::Regular));

    let rename = changes
        .iter()
        .find(|change| change.operation() == ChangeOperation::Rename)
        .expect("rename evidence");
    assert_eq!(rename.source_path(), Some("old-name.txt"));
    assert_eq!(rename.destination_path(), Some("renamed.txt"));
    assert_eq!(rename.source_kind(), Some(PathKind::Regular));
    assert_eq!(rename.destination_kind(), Some(PathKind::Regular));
    assert_eq!(rename.states(), &[ChangeState::Staged]);

    let deleted = changes
        .iter()
        .find(|change| change.operation() == ChangeOperation::Delete)
        .expect("delete evidence");
    assert_eq!(deleted.source_path(), Some("delete-me.txt"));
    assert_eq!(deleted.source_kind(), Some(PathKind::Regular));
    assert_eq!(deleted.states(), &[ChangeState::Unstaged]);

    let staged = changes
        .iter()
        .find(|change| change.destination_path() == Some("staged.txt"))
        .expect("mixed staged evidence");
    assert_eq!(staged.operation(), ChangeOperation::Create);
    assert_eq!(
        staged.states(),
        &[ChangeState::Staged, ChangeState::Unstaged]
    );

    let untracked = changes
        .iter()
        .find(|change| change.destination_path() == Some("untracked.txt"))
        .expect("untracked evidence");
    assert_eq!(untracked.states(), &[ChangeState::Untracked]);

    let ignored = changes
        .iter()
        .find(|change| change.destination_path() == Some("hidden.ignored"))
        .expect("ignored evidence");
    assert_eq!(ignored.states(), &[ChangeState::Ignored]);
}

#[test]
fn changed_manifest_preserves_git_symlink_kind_without_following_it() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let authority = owner_authority(&repository);
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");
    let owned = workspace
        .create_worktree(&authority, &base)
        .expect("feature worktree");
    fs::write(owned.path().join("link.txt"), "tracked.txt").expect("link payload");
    let object = git(owned.path(), &["hash-object", "-w", "link.txt"]);
    git(
        owned.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("120000,{object},link.txt"),
        ],
    );

    let manifest = workspace
        .changed_manifest(&authority, &owned)
        .expect("symlink-mode evidence");
    let link = manifest
        .changes()
        .iter()
        .find(|change| change.destination_path() == Some("link.txt"))
        .expect("link change");

    assert_eq!(link.operation(), ChangeOperation::Create);
    assert_eq!(link.destination_kind(), Some(PathKind::Symlink));
}

#[test]
fn changed_manifest_reports_type_change_evidence() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let authority = owner_authority(&repository);
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");
    let owned = workspace
        .create_worktree(&authority, &base)
        .expect("feature worktree");

    let tracked_object = git(owned.path(), &["hash-object", "-w", "tracked.txt"]);
    git(
        owned.path(),
        &[
            "update-index",
            "--cacheinfo",
            &format!("120000,{tracked_object},tracked.txt"),
        ],
    );
    let type_manifest = workspace
        .changed_manifest(&authority, &owned)
        .expect("type-change evidence");
    let type_change = type_manifest
        .changes()
        .iter()
        .find(|change| change.destination_path() == Some("tracked.txt"))
        .expect("tracked type change");
    assert_eq!(type_change.operation(), ChangeOperation::TypeChange);
    assert_eq!(type_change.source_kind(), Some(PathKind::Regular));
    assert_eq!(type_change.destination_kind(), Some(PathKind::Symlink));
}

#[test]
fn changed_manifest_reports_unmerged_evidence() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let authority = owner_authority(&repository);
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");
    let owned = workspace
        .create_worktree(&authority, &base)
        .expect("feature worktree");
    fs::write(owned.path().join("shared.txt"), "feature\n").expect("feature conflict");
    git(owned.path(), &["add", "shared.txt"]);
    git(owned.path(), &["commit", "-m", "feature conflict"]);
    fs::write(repository.join("shared.txt"), "target\n").expect("target conflict");
    git(&repository, &["add", "shared.txt"]);
    git(&repository, &["commit", "-m", "target conflict"]);
    let target = git(&repository, &["rev-parse", "HEAD"]);
    git_fails(owned.path(), &["merge", "--no-edit", &target]);

    let conflict_manifest = workspace
        .changed_manifest(&authority, &owned)
        .expect("unmerged evidence");
    let unmerged = conflict_manifest
        .changes()
        .iter()
        .find(|change| change.destination_path() == Some("shared.txt"))
        .expect("unmerged path");
    assert_eq!(unmerged.operation(), ChangeOperation::Unmerged);
}

#[test]
fn merge_readiness_reports_conflict_without_changing_checkout_or_refs() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let authority = owner_authority(&repository);
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");
    let mut owned = workspace
        .create_worktree(&authority, &base)
        .expect("feature worktree");

    fs::write(owned.path().join("shared.txt"), "feature\n").expect("feature conflict");
    git(owned.path(), &["add", "shared.txt"]);
    git(owned.path(), &["commit", "-m", "feature change"]);
    let feature = git(owned.path(), &["rev-parse", "HEAD"]);

    fs::write(repository.join("shared.txt"), "target\n").expect("target conflict");
    git(&repository, &["add", "shared.txt"]);
    git(&repository, &["commit", "-m", "target change"]);
    let target = git(&repository, &["rev-parse", "HEAD"]);
    let refs_before = git(
        &repository,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
    );

    let readiness = workspace
        .analyze_integration(&authority, &target, &feature)
        .expect("conflict is evidence, not an execution error");

    assert_eq!(readiness.outcome(), IntegrationOutcome::Conflict);
    assert_eq!(readiness.conflicts(), &["shared.txt"]);
    assert_eq!(git(&repository, &["rev-parse", "HEAD"]), target);
    assert_eq!(git(&repository, &["status", "--porcelain=v1"]), "");
    assert_eq!(
        git(
            &repository,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        ),
        refs_before
    );

    workspace
        .remove_owned_worktree(&authority, &mut owned)
        .expect("remove exact clean feature worktree");
    assert!(!owned.path().exists());
}

#[test]
fn merge_readiness_reports_stable_clean_evidence_without_updating_refs() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let authority = owner_authority(&repository);
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");
    let mut owned = workspace
        .create_worktree(&authority, &base)
        .expect("feature worktree");
    fs::write(owned.path().join("feature.txt"), "feature\n").expect("feature file");
    git(owned.path(), &["add", "feature.txt"]);
    git(owned.path(), &["commit", "-m", "feature change"]);
    let feature = git(owned.path(), &["rev-parse", "HEAD"]);
    let refs_before = git(
        &repository,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
    );

    let first = workspace
        .analyze_integration(&authority, &base, &feature)
        .expect("clean readiness");
    let second = workspace
        .analyze_integration(&authority, &base, &feature)
        .expect("stable clean readiness");

    assert_eq!(first, second);
    assert_eq!(first.outcome(), IntegrationOutcome::Clean);
    assert!(first.conflicts().is_empty());
    assert_eq!(first.digest().as_str().len(), 64);
    assert_eq!(git(&repository, &["rev-parse", "HEAD"]), base);
    assert_eq!(
        git(
            &repository,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        ),
        refs_before
    );

    workspace
        .remove_owned_worktree(&authority, &mut owned)
        .expect("remove exact clean feature worktree");
}

#[test]
fn dirty_owned_worktree_is_never_removed() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let authority = owner_authority(&repository);
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");
    let mut owned = workspace
        .create_worktree(&authority, &base)
        .expect("feature worktree");
    fs::write(owned.path().join("tracked.txt"), "dirty\n").expect("dirty fixture");

    let error = workspace
        .remove_owned_worktree(&authority, &mut owned)
        .expect_err("dirty worktree must remain");

    assert_eq!(error, WorkspaceError::WorktreeDirty);
    assert!(owned.path().exists());
}

#[test]
fn newer_formal_lease_head_invalidates_the_owned_worktree_handle() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let mut owners = owner_harness(&repository);
    let initial_authority = owners.authority();
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");
    let owned = workspace
        .create_worktree(&initial_authority, &base)
        .expect("feature worktree");
    let current_head = owners
        .lease_owner
        .current_head(&owners.project_id)
        .expect("current lease head");
    owners
        .lease_owner
        .execute(WriterLeaseCommand::Heartbeat(HeartbeatCommand {
            command_id: "heartbeat-project-a".to_owned(),
            project_id: owners.project_id.clone(),
            expected_head: current_head,
            observation: lease_observation(RuntimeAdmissionMode::Active, "2026-08-09T00:05:00Z"),
            expires_at: "2026-08-09T00:15:00Z".to_owned(),
        }))
        .expect("advance formal lease head");
    let newer_authority = owners.authority();

    let error = workspace
        .changed_manifest(&newer_authority, &owned)
        .expect_err("old process handle must not cross an authority-head change");

    assert_eq!(error, WorkspaceError::WorktreeIdentityMismatch);
}
