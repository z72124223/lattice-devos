use std::cell::Cell;
use std::ffi::OsString;
use std::fs;
use std::ops::Deref;
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
    GitWorkspace, IntegrationOutcome, PathKind, WorkspaceAuthority, WorkspaceAuthorityLookup,
    WorkspaceError,
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

#[cfg(unix)]
fn create_directory_symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("create disposable directory symlink");
}

#[cfg(windows)]
fn create_directory_symlink(target: &Path, link: &Path) {
    if std::os::windows::fs::symlink_dir(target, link).is_ok() {
        return;
    }
    let output = Command::new("cmd.exe")
        .args(["/d", "/s", "/c", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .expect("run disposable junction fixture command");
    assert!(
        output.status.success(),
        "create disposable junction: {}",
        String::from_utf8_lossy(&output.stderr)
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

impl WorkspaceAuthorityLookup for OwnerHarness {
    fn current_repository_observation(
        &self,
        project_id: &ProjectId,
    ) -> Result<RepositoryObservation, WorkspaceError> {
        if project_id != &self.project_id {
            return Err(WorkspaceError::ProjectObservationMismatch);
        }
        Ok(self.registry_observation.clone())
    }

    fn current_project_head(
        &self,
        project_id: &ProjectId,
    ) -> Result<lattice_contracts::ProjectAuthorityHead, WorkspaceError> {
        if project_id != &self.project_id {
            return Err(WorkspaceError::ProjectHeadMismatch);
        }
        Ok(self.registry_head.clone())
    }

    fn current_writer_lease_head(
        &self,
        project_id: &ProjectId,
    ) -> Result<lattice_contracts::WriterLeaseAuthorityHead, WorkspaceError> {
        self.lease_owner
            .current_head(project_id)
            .ok_or(WorkspaceError::LeaseHeadMismatch)
    }
}

struct CurrentAuthority {
    authority: WorkspaceAuthority,
    owners: OwnerHarness,
}

struct ExpiringLookup<'a> {
    owners: &'a OwnerHarness,
    observation_calls: Cell<u8>,
    allowed_observation_calls: u8,
}

impl WorkspaceAuthorityLookup for ExpiringLookup<'_> {
    fn current_repository_observation(
        &self,
        project_id: &ProjectId,
    ) -> Result<RepositoryObservation, WorkspaceError> {
        let calls = self.observation_calls.get();
        self.observation_calls.set(calls + 1);
        if calls < self.allowed_observation_calls {
            self.owners.current_repository_observation(project_id)
        } else {
            Err(WorkspaceError::ProjectObservationMismatch)
        }
    }

    fn current_project_head(
        &self,
        project_id: &ProjectId,
    ) -> Result<lattice_contracts::ProjectAuthorityHead, WorkspaceError> {
        self.owners.current_project_head(project_id)
    }

    fn current_writer_lease_head(
        &self,
        project_id: &ProjectId,
    ) -> Result<lattice_contracts::WriterLeaseAuthorityHead, WorkspaceError> {
        self.owners.current_writer_lease_head(project_id)
    }
}

impl Deref for CurrentAuthority {
    type Target = WorkspaceAuthority;

    fn deref(&self) -> &Self::Target {
        &self.authority
    }
}

impl WorkspaceAuthorityLookup for CurrentAuthority {
    fn current_repository_observation(
        &self,
        project_id: &ProjectId,
    ) -> Result<RepositoryObservation, WorkspaceError> {
        self.owners.current_repository_observation(project_id)
    }

    fn current_project_head(
        &self,
        project_id: &ProjectId,
    ) -> Result<lattice_contracts::ProjectAuthorityHead, WorkspaceError> {
        self.owners.current_project_head(project_id)
    }

    fn current_writer_lease_head(
        &self,
        project_id: &ProjectId,
    ) -> Result<lattice_contracts::WriterLeaseAuthorityHead, WorkspaceError> {
        self.owners.current_writer_lease_head(project_id)
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

fn owner_authority(repository: &Path) -> CurrentAuthority {
    let owners = owner_harness(repository);
    let authority = owners.authority();
    CurrentAuthority { authority, owners }
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
        .create_worktree(&authority, &authority, &base)
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
        .create_worktree(&authority, &authority, &base)
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
fn authority_is_rechecked_immediately_before_worktree_creation() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let owners = owner_harness(&repository);
    let authority = owners.authority();
    let lookup = ExpiringLookup {
        owners: &owners,
        observation_calls: Cell::new(0),
        allowed_observation_calls: 1,
    };
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let executor = RecordingExecutor::default();
    let requests = Arc::clone(&executor.requests);
    let workspace = GitWorkspace::new(&workspace_root, executor).expect("workspace boundary");

    let error = workspace
        .create_worktree(&authority, &lookup, &base)
        .expect_err("authority that expires during preflight must stop creation");

    assert_eq!(error, WorkspaceError::ProjectObservationMismatch);
    assert!(
        requests
            .lock()
            .expect("recorded requests")
            .iter()
            .all(|request| !request.windows(2).any(|args| args == ["worktree", "add"]))
    );
    assert!(
        fs::read_dir(workspace_root.join(".lattice-guards"))
            .expect("guard root")
            .next()
            .is_none(),
        "the exact pre-effect guard must be reclaimed"
    );
    workspace
        .create_worktree(&authority, &owners, &base)
        .expect("current authority can retry after exact guard reclamation");
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
        .create_worktree(&authority, &authority, &base)
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
fn preexisting_or_tampered_execution_sandbox_fails_before_git_effects() {
    let fixture = TempRoot::new();
    let preexisting_hooks_root = fixture.path().join("preexisting-hooks-worktrees");
    fs::create_dir(&preexisting_hooks_root).expect("preexisting hooks workspace root");
    fs::create_dir(preexisting_hooks_root.join(".lattice-hooks-empty"))
        .expect("attacker-controlled empty hooks root");
    assert_eq!(
        GitWorkspace::new(&preexisting_hooks_root, CommandGitExecutor)
            .expect_err("must reject preexisting hooks root"),
        WorkspaceError::WorkspaceRootUnsafe
    );

    let preexisting_root = fixture.path().join("preexisting-worktrees");
    fs::create_dir(&preexisting_root).expect("preexisting workspace root");
    fs::write(
        preexisting_root.join(".lattice-empty-gitconfig"),
        "[filter \"danger\"]\nsmudge = dangerous-program\n",
    )
    .expect("malicious preexisting global config");
    assert_eq!(
        GitWorkspace::new(&preexisting_root, CommandGitExecutor).expect_err("must reject config"),
        WorkspaceError::WorkspaceRootUnsafe
    );

    let (repository, base) = disposable_repository(fixture.path());
    let authority = owner_authority(&repository);
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");
    fs::write(
        workspace_root.join(".lattice-empty-gitconfig"),
        "[include]\npath = missing-config\n",
    )
    .expect("tamper empty config");

    assert_eq!(
        workspace
            .create_worktree(&authority, &authority, &base)
            .expect_err("tampered config must fail before Git"),
        WorkspaceError::WorkspaceRootUnsafe
    );
}

#[test]
fn tampered_empty_hooks_directory_fails_before_git_effects() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let authority = owner_authority(&repository);
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");
    fs::write(
        workspace_root
            .join(".lattice-hooks-empty")
            .join("post-checkout"),
        "must not execute",
    )
    .expect("tamper hooks directory");

    assert_eq!(
        workspace
            .create_worktree(&authority, &authority, &base)
            .expect_err("tampered hooks must fail before Git"),
        WorkspaceError::WorkspaceRootUnsafe
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
        .create_worktree(&authority, &authority, &base)
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
        .changed_manifest(&authority, &authority, &owned)
        .expect("changed-path manifest");
    let second = workspace
        .changed_manifest(&authority, &authority, &owned)
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

    let staged_create = changes
        .iter()
        .find(|change| {
            change.destination_path() == Some("staged.txt")
                && change.operation() == ChangeOperation::Create
        })
        .expect("staged create evidence");
    assert_eq!(staged_create.states(), &[ChangeState::Staged]);
    let unstaged_modify = changes
        .iter()
        .find(|change| {
            change.destination_path() == Some("staged.txt")
                && change.operation() == ChangeOperation::Modify
        })
        .expect("unstaged modification evidence");
    assert_eq!(unstaged_modify.states(), &[ChangeState::Unstaged]);

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
        .create_worktree(&authority, &authority, &base)
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
        .changed_manifest(&authority, &authority, &owned)
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
fn changed_manifest_detects_an_empty_physical_link_without_following_it() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let authority = owner_authority(&repository);
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");
    let owned = workspace
        .create_worktree(&authority, &authority, &base)
        .expect("feature worktree");
    let outside = fixture.path().join("outside");
    fs::create_dir(&outside).expect("outside target");
    fs::write(outside.join("must-not-be-followed.txt"), "outside\n").expect("outside fixture");
    let link = owned.path().join("empty-link");
    create_directory_symlink(&outside, &link);
    fs::write(owned.path().join("forbidden.txt"), "must not be hidden\n")
        .expect("concurrent regular change");

    let error = workspace
        .changed_manifest(&authority, &authority, &owned)
        .expect_err("physical link must stop complete-manifest production");
    let WorkspaceError::PhysicalPathHazard { evidence } = error else {
        panic!("unexpected physical-link error: {error:?}");
    };
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].path(), "empty-link");
    assert!(matches!(
        evidence[0].kind(),
        PathKind::Symlink | PathKind::ReparsePoint
    ));
    assert!(outside.join("must-not-be-followed.txt").exists());

    fs::remove_dir(link).expect("remove disposable link without following it");
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
        .create_worktree(&authority, &authority, &base)
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
        .changed_manifest(&authority, &authority, &owned)
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
        .create_worktree(&authority, &authority, &base)
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
        .changed_manifest(&authority, &authority, &owned)
        .expect("unmerged evidence");
    let unmerged = conflict_manifest
        .changes()
        .iter()
        .find(|change| change.destination_path() == Some("shared.txt"))
        .expect("unmerged path");
    assert_eq!(unmerged.operation(), ChangeOperation::Unmerged);
}

#[test]
fn layered_changes_remain_visible_when_the_worktree_returns_to_base_endpoint() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let authority = owner_authority(&repository);
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");
    let owned = workspace
        .create_worktree(&authority, &authority, &base)
        .expect("feature worktree");
    fs::write(owned.path().join("forbidden.txt"), "committed\n").expect("committed layer");
    git(owned.path(), &["add", "forbidden.txt"]);
    git(owned.path(), &["commit", "-m", "add forbidden path"]);
    fs::remove_file(owned.path().join("forbidden.txt")).expect("return endpoint to base");

    let manifest = workspace
        .changed_manifest(&authority, &authority, &owned)
        .expect("layer-complete evidence");

    assert!(manifest.changes().iter().any(|change| {
        change.operation() == ChangeOperation::Create
            && change.destination_path() == Some("forbidden.txt")
            && change.states() == [ChangeState::Committed]
    }));
    assert!(manifest.changes().iter().any(|change| {
        change.operation() == ChangeOperation::Delete
            && change.source_path() == Some("forbidden.txt")
            && change.states() == [ChangeState::Unstaged]
    }));
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
        .create_worktree(&authority, &authority, &base)
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
        .analyze_integration(&authority, &authority, &target, &feature)
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
        .remove_owned_worktree(&authority, &authority, &mut owned)
        .expect("remove exact clean feature worktree");
    assert!(!owned.path().exists());
}

#[test]
fn post_merge_tree_authority_failure_requires_reconciliation() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let owners = owner_harness(&repository);
    let authority = owners.authority();
    let lookup = ExpiringLookup {
        owners: &owners,
        observation_calls: Cell::new(0),
        allowed_observation_calls: 2,
    };
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let executor = RecordingExecutor::default();
    let requests = Arc::clone(&executor.requests);
    let workspace = GitWorkspace::new(&workspace_root, executor).expect("workspace boundary");

    let error = workspace
        .analyze_integration(&authority, &lookup, &base, &base)
        .expect_err("post-effect authority failure must require reconciliation");

    assert_eq!(error, WorkspaceError::ReconciliationRequired);
    assert!(
        requests
            .lock()
            .expect("recorded requests")
            .iter()
            .any(|request| request
                .windows(2)
                .any(|args| args == ["merge-tree", "--write-tree"]))
    );
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
        .create_worktree(&authority, &authority, &base)
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
        .analyze_integration(&authority, &authority, &base, &feature)
        .expect("clean readiness");
    let second = workspace
        .analyze_integration(&authority, &authority, &base, &feature)
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
        .remove_owned_worktree(&authority, &authority, &mut owned)
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
        .create_worktree(&authority, &authority, &base)
        .expect("feature worktree");
    fs::write(owned.path().join("tracked.txt"), "dirty\n").expect("dirty fixture");

    let error = workspace
        .remove_owned_worktree(&authority, &authority, &mut owned)
        .expect_err("dirty worktree must remain");

    assert_eq!(error, WorkspaceError::WorktreeDirty);
    assert!(owned.path().exists());
}

#[test]
fn ignored_files_are_dirty_and_are_never_removed() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let authority = owner_authority(&repository);
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");
    let mut owned = workspace
        .create_worktree(&authority, &authority, &base)
        .expect("feature worktree");
    fs::write(owned.path().join("valuable.ignored"), "retain me\n").expect("ignored user output");

    let error = workspace
        .remove_owned_worktree(&authority, &authority, &mut owned)
        .expect_err("ignored output must remain");

    assert_eq!(error, WorkspaceError::WorktreeDirty);
    assert!(owned.path().join("valuable.ignored").exists());
}

#[test]
fn physical_links_are_never_removed_or_followed_during_cleanup() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let authority = owner_authority(&repository);
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");
    let mut owned = workspace
        .create_worktree(&authority, &authority, &base)
        .expect("feature worktree");
    let outside = fixture.path().join("cleanup-outside");
    fs::create_dir(&outside).expect("outside target");
    let sentinel = outside.join("retain.txt");
    fs::write(&sentinel, "retain\n").expect("outside sentinel");
    let link = owned.path().join("cleanup-link");
    create_directory_symlink(&outside, &link);

    let error = workspace
        .remove_owned_worktree(&authority, &authority, &mut owned)
        .expect_err("physical link must retain the complete worktree");

    let WorkspaceError::PhysicalPathHazard { evidence } = error else {
        panic!("unexpected cleanup error: {error:?}");
    };
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].path(), "cleanup-link");
    assert!(owned.path().exists());
    assert!(link.exists());
    assert!(sentinel.exists());

    fs::remove_dir(link).expect("remove disposable link without following it");
}

#[test]
fn authority_is_rechecked_immediately_before_worktree_removal() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let owners = owner_harness(&repository);
    let authority = owners.authority();
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");
    let mut owned = workspace
        .create_worktree(&authority, &owners, &base)
        .expect("feature worktree");
    let lookup = ExpiringLookup {
        owners: &owners,
        observation_calls: Cell::new(0),
        allowed_observation_calls: 1,
    };

    let error = workspace
        .remove_owned_worktree(&authority, &lookup, &mut owned)
        .expect_err("authority that expires during preflight must retain the worktree");

    assert_eq!(error, WorkspaceError::ProjectObservationMismatch);
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
        .create_worktree(&initial_authority, &owners, &base)
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
    let error = workspace
        .changed_manifest(&initial_authority, &owners, &owned)
        .expect_err("old authority itself must not cross an owner-head change");

    assert_eq!(error, WorkspaceError::LeaseHeadMismatch);
}

#[test]
fn current_physical_registry_observation_must_match_every_git_operation() {
    let fixture = TempRoot::new();
    let (repository, base) = disposable_repository(fixture.path());
    let mut owners = owner_harness(&repository);
    let authority = owners.authority();
    let workspace_root = fixture.path().join("worktrees");
    fs::create_dir(&workspace_root).expect("workspace root");
    let workspace =
        GitWorkspace::new(&workspace_root, CommandGitExecutor).expect("workspace boundary");
    let owned = workspace
        .create_worktree(&authority, &owners, &base)
        .expect("feature worktree");
    owners.registry_observation = RepositoryObservation::new(
        repository.to_str().expect("utf8 fixture path"),
        digest('e'),
        digest('f'),
        digest('7'),
        GitRefIdentity::new("refs/heads/main", digest('8')).expect("changed physical ref"),
    )
    .expect("changed inspector observation");

    let error = workspace
        .changed_manifest(&authority, &owners, &owned)
        .expect_err("physical observation drift must fail before Git");

    assert_eq!(error, WorkspaceError::ProjectObservationMismatch);
}
