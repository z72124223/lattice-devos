//! Narrow, isolated Git delivery fixture for the first executable LATTICE path.
//!
//! This module deliberately does not expose a general Git adapter. It owns one
//! freshly provisioned repository, accepts exactly one new `answer.txt`, runs
//! one fixed local comparison, and creates one local commit only after every
//! pre-commit check passes.

use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// Exact product output accepted by the TASK-032 delivery fixture.
pub const EXPECTED_ANSWER_BYTES: &[u8] = b"LATTICE_DELIVERY_OK\n";

const ANSWER_FILE_NAME: &str = "answer.txt";
const INITIAL_COMMIT_MESSAGE: &str = "chore: initialize LATTICE delivery fixture";
const DELIVERY_COMMIT_MESSAGE: &str = "feat: complete LATTICE delivery fixture";

/// Stable fail-closed categories for the isolated Git delivery boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitDeliveryErrorKind {
    InvalidPath,
    UnsafePath,
    GitFailed,
    ScopeDrift,
    MetadataDrift,
    TestFailed,
    CommitOutcomeUnknown,
}

/// A bounded Git delivery failure with no captured command output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitDeliveryError {
    kind: GitDeliveryErrorKind,
    code: &'static str,
}

impl GitDeliveryError {
    #[must_use]
    pub const fn kind(&self) -> GitDeliveryErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for GitDeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.kind, self.code)
    }
}

impl Error for GitDeliveryError {}

/// Machine-readable evidence emitted only after the local commit is verified.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitDeliveryEvidence {
    pub repository_path: PathBuf,
    pub baseline_commit: String,
    pub commit_sha: String,
    pub changed_paths: Vec<String>,
    pub test_command_id: &'static str,
}

/// One freshly provisioned and metadata-isolated Git delivery repository.
pub struct IsolatedGitDelivery {
    root: PathBuf,
    repository_path: PathBuf,
    control_path: PathBuf,
    git_directory: PathBuf,
    hooks_directory: PathBuf,
    home_directory: PathBuf,
    temp_directory: PathBuf,
    global_config_path: PathBuf,
    attributes_path: PathBuf,
    expected_answer_path: PathBuf,
    git_pointer_bytes: Vec<u8>,
    local_config_bytes: Vec<u8>,
    baseline_refs: Vec<u8>,
    baseline_commit: String,
    runner: GitRunner,
}

impl IsolatedGitDelivery {
    /// Creates a new repository and an external Git control directory.
    ///
    /// `root` must be an absent absolute path whose parent is already a
    /// canonical real directory. `git_exe` must identify one absolute regular
    /// executable file. No existing repository is adopted or modified.
    ///
    /// # Errors
    ///
    /// Rejects unsafe paths, an invalid Git executable, metadata isolation
    /// failure, or inability to create and verify the baseline commit.
    pub fn provision(
        root: impl AsRef<Path>,
        git_exe: impl AsRef<Path>,
    ) -> Result<Self, GitDeliveryError> {
        let deadline = Instant::now()
            .checked_add(Duration::from_mins(1))
            .ok_or_else(|| failure(GitDeliveryErrorKind::GitFailed, "DEADLINE_INVALID"))?;
        Self::provision_until(root, git_exe, deadline)
    }

    /// Creates one isolated repository under a caller-owned absolute deadline.
    ///
    /// # Errors
    ///
    /// Rejects the same unsafe inputs as [`Self::provision`] and fails closed
    /// if any owned Git subprocess cannot finish before `deadline`.
    pub fn provision_until(
        root: impl AsRef<Path>,
        git_exe: impl AsRef<Path>,
        deadline: Instant,
    ) -> Result<Self, GitDeliveryError> {
        let root = root.as_ref().to_path_buf();
        validate_fresh_root(&root)?;
        let git_exe = validate_git_executable(git_exe.as_ref())?;

        fs::create_dir(&root)
            .map_err(|_| failure(GitDeliveryErrorKind::UnsafePath, "ROOT_CREATE_FAILED"))?;
        ensure_canonical_directory(&root)?;

        let repository_path = root.join("repo");
        let control_path = root.join("control");
        let hooks_directory = control_path.join("empty-hooks");
        let home_directory = control_path.join("git-home");
        let temp_directory = control_path.join("temp");
        let global_config_path = control_path.join("empty-global.gitconfig");
        let attributes_path = control_path.join("empty-attributes");
        let expected_answer_path = control_path.join("expected-answer.txt");
        let git_directory = control_path.join("git-dir");

        for directory in [
            &repository_path,
            &control_path,
            &hooks_directory,
            &home_directory,
            &temp_directory,
        ] {
            fs::create_dir(directory).map_err(|_| {
                failure(
                    GitDeliveryErrorKind::UnsafePath,
                    "CONTROL_DIRECTORY_CREATE_FAILED",
                )
            })?;
            ensure_canonical_directory(directory)?;
        }
        create_new_file(&global_config_path, b"")?;
        create_new_file(&attributes_path, b"")?;
        create_new_file(&expected_answer_path, EXPECTED_ANSWER_BYTES)?;

        let runner = GitRunner {
            git_exe,
            current_directory: root.clone(),
            hooks_directory: hooks_directory.clone(),
            home_directory: home_directory.clone(),
            temp_directory: temp_directory.clone(),
            global_config_path: global_config_path.clone(),
            attributes_path: attributes_path.clone(),
            deadline,
            next_output: AtomicU64::new(0),
        };

        let mut init_args = string_args(&["init"]);
        init_args.push(OsString::from("--initial-branch=main"));
        init_args.push(prefixed_path_argument(
            "--separate-git-dir=",
            &git_directory,
        ));
        init_args.push(repository_path.as_os_str().to_owned());
        require_success(runner.output(&init_args), "GIT_INIT_FAILED")?;
        ensure_canonical_directory(&git_directory)?;

        let initial_commit_args = repository_args(
            &repository_path,
            &[
                "commit",
                "--allow-empty",
                "--no-verify",
                "--no-gpg-sign",
                "-m",
                INITIAL_COMMIT_MESSAGE,
            ],
        );
        require_success(runner.output(&initial_commit_args), "INITIAL_COMMIT_FAILED")?;

        let baseline_commit = read_head(&runner, &repository_path)?;
        let baseline_refs = require_success(
            runner.output(&repository_args(
                &repository_path,
                &["show-ref", "--head", "--dereference"],
            )),
            "INITIAL_REFS_FAILED",
        )?
        .stdout;
        verify_only_main_ref(&baseline_refs, &baseline_commit)?;

        let git_pointer_path = repository_path.join(".git");
        let local_config_path = git_directory.join("config");
        let git_pointer_bytes = read_regular_file(&git_pointer_path, "GIT_POINTER_UNSAFE")?;
        let local_config_bytes = read_regular_file(&local_config_path, "GIT_CONFIG_UNSAFE")?;

        let delivery = Self {
            root,
            repository_path,
            control_path,
            git_directory,
            hooks_directory,
            home_directory,
            temp_directory,
            global_config_path,
            attributes_path,
            expected_answer_path,
            git_pointer_bytes,
            local_config_bytes,
            baseline_refs,
            baseline_commit,
            runner,
        };
        delivery.verify_static_metadata()?;
        delivery.assert_no_unsafe_local_config()?;
        delivery.assert_clean_baseline()?;
        Ok(delivery)
    }

    /// Returns the only directory that may be supplied to the Codex turn.
    #[must_use]
    pub fn repo_path(&self) -> &Path {
        &self.repository_path
    }

    /// Verifies the exact output and creates one local commit.
    ///
    /// No staging or commit occurs until static metadata, HEAD/index state,
    /// the exact changed-file scope, and the fixed local comparison all pass.
    ///
    /// # Errors
    ///
    /// Rejects scope, metadata, or test drift and treats an uncertain commit
    /// result as reconciliation-required evidence.
    pub fn verify_and_commit(&self) -> Result<GitDeliveryEvidence, GitDeliveryError> {
        self.verify_precommit_state()?;
        self.verify_exact_scope()?;
        self.run_fixed_test()?;

        // Recheck the complete pre-commit boundary immediately before the
        // first Git mutation owned by this method.
        self.verify_precommit_state()?;
        self.verify_exact_scope()?;

        require_success(
            self.runner.output(&repository_args(
                &self.repository_path,
                &["add", "--", ANSWER_FILE_NAME],
            )),
            "GIT_ADD_FAILED",
        )?;
        self.assert_exact_staged_answer()?;
        self.run_fixed_test()?;

        let commit_args = repository_args(
            &self.repository_path,
            &[
                "commit",
                "--no-verify",
                "--no-gpg-sign",
                "-m",
                DELIVERY_COMMIT_MESSAGE,
            ],
        );
        let commit = self.runner.output(&commit_args).map_err(|_| {
            failure(
                GitDeliveryErrorKind::CommitOutcomeUnknown,
                "COMMIT_WAIT_UNKNOWN",
            )
        })?;
        if !commit.status.success() {
            return Err(failure(
                GitDeliveryErrorKind::CommitOutcomeUnknown,
                "COMMIT_EXIT_UNKNOWN",
            ));
        }

        self.verify_static_metadata().map_err(|_| {
            failure(
                GitDeliveryErrorKind::CommitOutcomeUnknown,
                "POST_COMMIT_METADATA_UNKNOWN",
            )
        })?;
        let commit_sha = read_head(&self.runner, &self.repository_path).map_err(|_| {
            failure(
                GitDeliveryErrorKind::CommitOutcomeUnknown,
                "POST_COMMIT_HEAD_UNKNOWN",
            )
        })?;
        if commit_sha == self.baseline_commit {
            return Err(failure(
                GitDeliveryErrorKind::CommitOutcomeUnknown,
                "COMMIT_DID_NOT_ADVANCE_HEAD",
            ));
        }
        self.verify_committed_result(&commit_sha)?;

        Ok(GitDeliveryEvidence {
            repository_path: self.repository_path.clone(),
            baseline_commit: self.baseline_commit.clone(),
            commit_sha,
            changed_paths: vec![ANSWER_FILE_NAME.to_owned()],
            test_command_id: "git-diff-no-index-exact-answer-v1",
        })
    }

    fn verify_static_metadata(&self) -> Result<(), GitDeliveryError> {
        for directory in [
            &self.root,
            &self.repository_path,
            &self.control_path,
            &self.git_directory,
            &self.hooks_directory,
            &self.home_directory,
            &self.temp_directory,
        ] {
            ensure_canonical_directory(directory)?;
        }
        if fs::read_dir(&self.hooks_directory)
            .map_err(|_| failure(GitDeliveryErrorKind::MetadataDrift, "HOOKS_READ_FAILED"))?
            .next()
            .is_some()
        {
            return Err(failure(
                GitDeliveryErrorKind::MetadataDrift,
                "HOOKS_DIRECTORY_NOT_EMPTY",
            ));
        }
        assert_regular_file_bytes(&self.global_config_path, b"", "GLOBAL_CONFIG_DRIFT")?;
        assert_regular_file_bytes(&self.attributes_path, b"", "ATTRIBUTES_DRIFT")?;
        assert_regular_file_bytes(
            &self.expected_answer_path,
            EXPECTED_ANSWER_BYTES,
            "EXPECTED_ANSWER_DRIFT",
        )?;
        assert_regular_file_bytes(
            &self.repository_path.join(".git"),
            &self.git_pointer_bytes,
            "GIT_POINTER_DRIFT",
        )?;
        assert_regular_file_bytes(
            &self.git_directory.join("config"),
            &self.local_config_bytes,
            "GIT_CONFIG_DRIFT",
        )?;
        Ok(())
    }

    fn assert_no_unsafe_local_config(&self) -> Result<(), GitDeliveryError> {
        let output = self.runner.output(&repository_args(
            &self.repository_path,
            &[
                "config",
                "--local",
                "--null",
                "--get-regexp",
                "^(include\\..*|includeif\\..*|filter\\..*\\.(clean|smudge|process|required)|merge\\..*\\.driver|diff\\..*\\.(command|textconv)|core\\.(hookspath|fsmonitor|attributesfile|sshcommand))$",
            ],
        ));
        let output = output.map_err(|_| {
            failure(
                GitDeliveryErrorKind::GitFailed,
                "GIT_CONFIG_INSPECTION_FAILED",
            )
        })?;
        match output.status.code() {
            Some(0 | 1) if output.stdout.is_empty() => Ok(()),
            Some(0) => Err(failure(
                GitDeliveryErrorKind::MetadataDrift,
                "UNSAFE_LOCAL_GIT_CONFIG",
            )),
            _ => Err(failure(
                GitDeliveryErrorKind::GitFailed,
                "GIT_CONFIG_INSPECTION_FAILED",
            )),
        }
    }

    fn assert_clean_baseline(&self) -> Result<(), GitDeliveryError> {
        let status = require_success(
            self.runner.output(&repository_args(
                &self.repository_path,
                &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            )),
            "INITIAL_STATUS_FAILED",
        )?;
        if !status.stdout.is_empty() {
            return Err(failure(
                GitDeliveryErrorKind::MetadataDrift,
                "INITIAL_REPOSITORY_NOT_CLEAN",
            ));
        }
        Ok(())
    }

    fn verify_precommit_state(&self) -> Result<(), GitDeliveryError> {
        self.verify_static_metadata()?;
        self.assert_no_unsafe_local_config()?;
        let head = read_head(&self.runner, &self.repository_path)?;
        if head != self.baseline_commit {
            return Err(failure(GitDeliveryErrorKind::MetadataDrift, "HEAD_DRIFT"));
        }
        let refs = require_success(
            self.runner.output(&repository_args(
                &self.repository_path,
                &["show-ref", "--head", "--dereference"],
            )),
            "REF_INSPECTION_FAILED",
        )?;
        if refs.stdout != self.baseline_refs {
            return Err(failure(GitDeliveryErrorKind::MetadataDrift, "REF_DRIFT"));
        }
        let staged = self
            .runner
            .output(&repository_args(
                &self.repository_path,
                &[
                    "diff",
                    "--cached",
                    "--quiet",
                    "--no-ext-diff",
                    "--no-textconv",
                    "HEAD",
                    "--",
                ],
            ))
            .map_err(|_| failure(GitDeliveryErrorKind::GitFailed, "INDEX_INSPECTION_FAILED"))?;
        if !staged.status.success() {
            return Err(failure(GitDeliveryErrorKind::MetadataDrift, "INDEX_DRIFT"));
        }
        Ok(())
    }

    fn verify_exact_scope(&self) -> Result<(), GitDeliveryError> {
        let mut saw_git_pointer = false;
        let mut saw_answer = false;
        for entry in fs::read_dir(&self.repository_path)
            .map_err(|_| failure(GitDeliveryErrorKind::ScopeDrift, "REPOSITORY_SCAN_FAILED"))?
        {
            let entry = entry
                .map_err(|_| failure(GitDeliveryErrorKind::ScopeDrift, "REPOSITORY_SCAN_FAILED"))?;
            let name = entry.file_name();
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|_| failure(GitDeliveryErrorKind::ScopeDrift, "REPOSITORY_SCAN_FAILED"))?;
            if unsafe_file_type(&metadata) || !metadata.is_file() {
                return Err(failure(
                    GitDeliveryErrorKind::ScopeDrift,
                    "NON_REGULAR_REPOSITORY_ENTRY",
                ));
            }
            if name == OsStr::new(".git") {
                saw_git_pointer = true;
                if fs::read(entry.path()).map_err(|_| {
                    failure(
                        GitDeliveryErrorKind::MetadataDrift,
                        "GIT_POINTER_READ_FAILED",
                    )
                })? != self.git_pointer_bytes
                {
                    return Err(failure(
                        GitDeliveryErrorKind::MetadataDrift,
                        "GIT_POINTER_DRIFT",
                    ));
                }
            } else if name == OsStr::new(ANSWER_FILE_NAME) {
                saw_answer = true;
            } else {
                return Err(failure(GitDeliveryErrorKind::ScopeDrift, "FOREIGN_PATH"));
            }
        }
        if !saw_git_pointer {
            return Err(failure(
                GitDeliveryErrorKind::MetadataDrift,
                "GIT_POINTER_MISSING",
            ));
        }
        if !saw_answer {
            return Err(failure(GitDeliveryErrorKind::TestFailed, "ANSWER_MISSING"));
        }

        #[cfg(windows)]
        self.verify_windows_answer_file_boundary()?;

        let status = require_success(
            self.runner.output(&repository_args(
                &self.repository_path,
                &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            )),
            "SCOPE_STATUS_FAILED",
        )?;
        if status.stdout != b"?? answer.txt\0" {
            return Err(failure(
                GitDeliveryErrorKind::ScopeDrift,
                "UNEXPECTED_GIT_STATUS",
            ));
        }
        Ok(())
    }

    fn run_fixed_test(&self) -> Result<(), GitDeliveryError> {
        #[cfg(windows)]
        self.verify_windows_answer_file_boundary()?;

        let mut test_args = repository_args(
            &self.repository_path,
            &[
                "diff",
                "--no-index",
                "--no-ext-diff",
                "--no-textconv",
                "--exit-code",
                "--",
            ],
        );
        test_args.push(self.expected_answer_path.as_os_str().to_owned());
        test_args.push(
            self.repository_path
                .join(ANSWER_FILE_NAME)
                .as_os_str()
                .to_owned(),
        );
        let output = self
            .runner
            .output(&test_args)
            .map_err(|_| failure(GitDeliveryErrorKind::GitFailed, "FIXED_TEST_START_FAILED"))?;
        if !output.status.success() {
            return Err(failure(
                GitDeliveryErrorKind::TestFailed,
                "FIXED_TEST_FAILED",
            ));
        }
        assert_regular_file_bytes(
            &self.repository_path.join(ANSWER_FILE_NAME),
            EXPECTED_ANSWER_BYTES,
            "ANSWER_BYTES_MISMATCH",
        )
        .map_err(|_| failure(GitDeliveryErrorKind::TestFailed, "ANSWER_BYTES_MISMATCH"))
    }

    #[cfg(windows)]
    fn verify_windows_answer_file_boundary(&self) -> Result<(), GitDeliveryError> {
        let answer_path = self.repository_path.join(ANSWER_FILE_NAME);
        let fsutil =
            windows_system_executable(&["System32", "fsutil.exe"], "HARDLINK_PROBE_UNAVAILABLE")?;
        let hardlinks = self
            .runner
            .windows_probe_output(
                &fsutil,
                &[
                    OsString::from("hardlink"),
                    OsString::from("list"),
                    answer_path.as_os_str().to_owned(),
                ],
                None,
                "hardlink",
            )
            .map_err(|_| {
                failure(
                    GitDeliveryErrorKind::ScopeDrift,
                    "HARDLINK_INSPECTION_FAILED",
                )
            })?;
        if !hardlinks.status.success() {
            return Err(failure(
                GitDeliveryErrorKind::ScopeDrift,
                "HARDLINK_INSPECTION_FAILED",
            ));
        }
        let link_count = hardlinks
            .stdout
            .split(|byte| *byte == b'\n')
            .filter(|line| {
                line.iter()
                    .any(|byte| !matches!(*byte, b'\r' | b' ' | b'\t' | 0))
            })
            .count();
        if link_count != 1 {
            return Err(failure(
                GitDeliveryErrorKind::ScopeDrift,
                "ANSWER_HARDLINK_COUNT_DRIFT",
            ));
        }

        let powershell = windows_system_executable(
            &["System32", "WindowsPowerShell", "v1.0", "powershell.exe"],
            "STREAM_PROBE_UNAVAILABLE",
        )?;
        let stream_script = concat!(
            "$ErrorActionPreference='Stop';try{",
            "$p=[Environment]::GetEnvironmentVariable('LATTICE_TASK032_ANSWER_PATH','Process');",
            "if([string]::IsNullOrWhiteSpace($p)){exit 42};",
            "$s=@(Get-Item -LiteralPath $p -Stream * -ErrorAction Stop);",
            // PowerShell reports the unnamed NTFS `::$DATA` stream as `:$DATA`.
            "if($s.Count -ne 1 -or $s[0].Stream -ne ':$DATA'){exit 41};",
            "exit 0}catch{exit 42}"
        );
        let streams = self
            .runner
            .windows_probe_output(
                &powershell,
                &[
                    OsString::from("-NoLogo"),
                    OsString::from("-NoProfile"),
                    OsString::from("-NonInteractive"),
                    OsString::from("-Command"),
                    OsString::from(stream_script),
                ],
                Some(("LATTICE_TASK032_ANSWER_PATH", answer_path.as_os_str())),
                "streams",
            )
            .map_err(|_| failure(GitDeliveryErrorKind::ScopeDrift, "STREAM_INSPECTION_FAILED"))?;
        match streams.status.code() {
            Some(0) => Ok(()),
            Some(41) => Err(failure(
                GitDeliveryErrorKind::ScopeDrift,
                "ANSWER_ALTERNATE_DATA_STREAM_DRIFT",
            )),
            _ => Err(failure(
                GitDeliveryErrorKind::ScopeDrift,
                "STREAM_INSPECTION_FAILED",
            )),
        }
    }

    fn assert_exact_staged_answer(&self) -> Result<(), GitDeliveryError> {
        let staged = require_success(
            self.runner.output(&repository_args(
                &self.repository_path,
                &[
                    "diff",
                    "--cached",
                    "--name-status",
                    "-z",
                    "--no-renames",
                    "HEAD",
                    "--",
                ],
            )),
            "STAGED_SCOPE_FAILED",
        )?;
        if staged.stdout != b"A\0answer.txt\0" {
            return Err(failure(
                GitDeliveryErrorKind::MetadataDrift,
                "STAGED_SCOPE_DRIFT",
            ));
        }
        let status = require_success(
            self.runner.output(&repository_args(
                &self.repository_path,
                &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            )),
            "STAGED_STATUS_FAILED",
        )?;
        if status.stdout != b"A  answer.txt\0" {
            return Err(failure(
                GitDeliveryErrorKind::MetadataDrift,
                "STAGED_STATUS_DRIFT",
            ));
        }
        Ok(())
    }

    fn verify_committed_result(&self, commit_sha: &str) -> Result<(), GitDeliveryError> {
        let unknown = || {
            failure(
                GitDeliveryErrorKind::CommitOutcomeUnknown,
                "POST_COMMIT_VERIFICATION_FAILED",
            )
        };
        let parent = require_success(
            self.runner.output(&repository_args(
                &self.repository_path,
                &["rev-parse", "--verify", "HEAD^"],
            )),
            "POST_COMMIT_PARENT_FAILED",
        )
        .map_err(|_| unknown())?;
        if parse_object_id(&parent.stdout).map_err(|_| unknown())? != self.baseline_commit {
            return Err(unknown());
        }

        let refs = require_success(
            self.runner.output(&repository_args(
                &self.repository_path,
                &["show-ref", "--head", "--dereference"],
            )),
            "POST_COMMIT_REFS_FAILED",
        )
        .map_err(|_| unknown())?;
        verify_only_main_ref(&refs.stdout, commit_sha).map_err(|_| unknown())?;

        let changed = require_success(
            self.runner.output(&repository_args(
                &self.repository_path,
                &[
                    "diff-tree",
                    "--no-commit-id",
                    "--name-status",
                    "-r",
                    "-z",
                    "--no-renames",
                    "HEAD",
                ],
            )),
            "POST_COMMIT_DIFF_FAILED",
        )
        .map_err(|_| unknown())?;
        if changed.stdout != b"A\0answer.txt\0" {
            return Err(unknown());
        }

        let blob = require_success(
            self.runner.output(&repository_args(
                &self.repository_path,
                &["show", "HEAD:answer.txt"],
            )),
            "POST_COMMIT_BLOB_FAILED",
        )
        .map_err(|_| unknown())?;
        if blob.stdout != EXPECTED_ANSWER_BYTES {
            return Err(unknown());
        }
        let status = require_success(
            self.runner.output(&repository_args(
                &self.repository_path,
                &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            )),
            "POST_COMMIT_STATUS_FAILED",
        )
        .map_err(|_| unknown())?;
        if !status.stdout.is_empty() {
            return Err(unknown());
        }
        Ok(())
    }
}

struct GitRunner {
    git_exe: PathBuf,
    current_directory: PathBuf,
    hooks_directory: PathBuf,
    home_directory: PathBuf,
    temp_directory: PathBuf,
    global_config_path: PathBuf,
    attributes_path: PathBuf,
    deadline: Instant,
    next_output: AtomicU64,
}

impl GitRunner {
    fn output(&self, args: &[OsString]) -> io::Result<Output> {
        let mut command = Command::new(&self.git_exe);
        command.current_dir(&self.current_directory).env_clear();
        for key in ["SystemRoot", "WINDIR", "ComSpec", "PATHEXT"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        if let Some(path) = isolated_path(&self.git_exe) {
            command.env("PATH", path);
        }
        command
            .env("HOME", &self.home_directory)
            .env("USERPROFILE", &self.home_directory)
            .env("XDG_CONFIG_HOME", self.home_directory.join("xdg"))
            .env("TEMP", &self.temp_directory)
            .env("TMP", &self.temp_directory)
            .env("TMPDIR", &self.temp_directory)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &self.global_config_path)
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "Never")
            .env("GIT_LFS_SKIP_SMUDGE", "1")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_PAGER", "cat")
            .env("PAGER", "cat")
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .env("GIT_AUTHOR_NAME", "LATTICE DevOS")
            .env("GIT_AUTHOR_EMAIL", "lattice@invalid.example")
            .env("GIT_COMMITTER_NAME", "LATTICE DevOS")
            .env("GIT_COMMITTER_EMAIL", "lattice@invalid.example")
            .arg("-c")
            .arg(prefixed_path_argument(
                "core.hooksPath=",
                &self.hooks_directory,
            ))
            .arg("-c")
            .arg("core.fsmonitor=false")
            .arg("-c")
            .arg(prefixed_path_argument(
                "core.attributesFile=",
                &self.attributes_path,
            ))
            .arg("-c")
            .arg("core.autocrlf=false")
            .arg("-c")
            .arg("core.safecrlf=true")
            .arg("-c")
            .arg("commit.gpgSign=false")
            .arg("-c")
            .arg("tag.gpgSign=false")
            .args(args)
            .stdin(Stdio::null());
        self.capture_output(&mut command, "git")
    }

    #[cfg(windows)]
    fn windows_probe_output(
        &self,
        executable: &Path,
        args: &[OsString],
        extra_environment: Option<(&str, &OsStr)>,
        label: &str,
    ) -> io::Result<Output> {
        let mut command = Command::new(executable);
        command.current_dir(&self.current_directory).env_clear();
        for key in ["SystemRoot", "WINDIR", "ComSpec", "PATHEXT"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            command.env("PATH", PathBuf::from(system_root).join("System32"));
        }
        if let Some((name, value)) = extra_environment {
            command.env(name, value);
        }
        command.args(args).stdin(Stdio::null());
        self.capture_output(&mut command, label)
    }

    fn capture_output(&self, command: &mut Command, label: &str) -> io::Result<Output> {
        if Instant::now() >= self.deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Git delivery deadline expired",
            ));
        }
        let output_id = self.next_output.fetch_add(1, Ordering::Relaxed);
        let stdout_path = self
            .temp_directory
            .join(format!("{label}-{output_id}.stdout"));
        let stderr_path = self
            .temp_directory
            .join(format!("{label}-{output_id}.stderr"));
        let stdout_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&stdout_path)?;
        let stderr_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&stderr_path)?;
        command
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file));
        let result = self.wait_for_output(command, &stdout_path, &stderr_path);
        let _ = fs::remove_file(&stdout_path);
        let _ = fs::remove_file(&stderr_path);
        result
    }

    fn wait_for_output(
        &self,
        command: &mut Command,
        stdout_path: &Path,
        stderr_path: &Path,
    ) -> io::Result<Output> {
        let mut child = command.spawn()?;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            let now = Instant::now();
            if now >= self.deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Git delivery deadline expired",
                ));
            }
            thread::sleep(Duration::from_millis(10).min(self.deadline.duration_since(now)));
        };
        Ok(Output {
            status,
            stdout: fs::read(stdout_path)?,
            stderr: fs::read(stderr_path)?,
        })
    }
}

fn failure(kind: GitDeliveryErrorKind, code: &'static str) -> GitDeliveryError {
    GitDeliveryError { kind, code }
}

fn validate_fresh_root(root: &Path) -> Result<(), GitDeliveryError> {
    if !root.is_absolute()
        || root.file_name().is_none()
        || root
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(failure(
            GitDeliveryErrorKind::InvalidPath,
            "ROOT_MUST_BE_ABSOLUTE_AND_NORMALIZED",
        ));
    }
    match fs::symlink_metadata(root) {
        Ok(_) => {
            return Err(failure(
                GitDeliveryErrorKind::InvalidPath,
                "ROOT_MUST_BE_ABSENT",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err(failure(
                GitDeliveryErrorKind::UnsafePath,
                "ROOT_INSPECTION_FAILED",
            ));
        }
    }
    let parent = root
        .parent()
        .ok_or_else(|| failure(GitDeliveryErrorKind::InvalidPath, "ROOT_PARENT_MISSING"))?;
    ensure_canonical_directory(parent)
}

fn validate_git_executable(path: &Path) -> Result<PathBuf, GitDeliveryError> {
    if !path.is_absolute() {
        return Err(failure(
            GitDeliveryErrorKind::InvalidPath,
            "GIT_EXE_MUST_BE_ABSOLUTE",
        ));
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        failure(
            GitDeliveryErrorKind::UnsafePath,
            "GIT_EXE_INSPECTION_FAILED",
        )
    })?;
    if unsafe_file_type(&metadata) || !metadata.is_file() {
        return Err(failure(
            GitDeliveryErrorKind::UnsafePath,
            "GIT_EXE_NOT_REGULAR",
        ));
    }
    fs::canonicalize(path).map_err(|_| {
        failure(
            GitDeliveryErrorKind::UnsafePath,
            "GIT_EXE_CANONICALIZE_FAILED",
        )
    })
}

fn ensure_canonical_directory(path: &Path) -> Result<(), GitDeliveryError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        failure(
            GitDeliveryErrorKind::UnsafePath,
            "DIRECTORY_INSPECTION_FAILED",
        )
    })?;
    if unsafe_file_type(&metadata) || !metadata.is_dir() {
        return Err(failure(
            GitDeliveryErrorKind::UnsafePath,
            "DIRECTORY_NOT_CANONICAL",
        ));
    }
    let canonical = fs::canonicalize(path).map_err(|_| {
        failure(
            GitDeliveryErrorKind::UnsafePath,
            "DIRECTORY_CANONICALIZE_FAILED",
        )
    })?;
    if !same_path(&canonical, path) {
        return Err(failure(
            GitDeliveryErrorKind::UnsafePath,
            "DIRECTORY_PATH_ESCAPE",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn windows_system_executable(
    components: &[&str],
    code: &'static str,
) -> Result<PathBuf, GitDeliveryError> {
    let system_root = std::env::var_os("SystemRoot")
        .ok_or_else(|| failure(GitDeliveryErrorKind::ScopeDrift, code))?;
    let system_root = PathBuf::from(system_root);
    ensure_canonical_directory(&system_root)
        .map_err(|_| failure(GitDeliveryErrorKind::ScopeDrift, code))?;
    let path = components
        .iter()
        .fold(system_root, |path, component| path.join(component));
    let metadata =
        fs::symlink_metadata(&path).map_err(|_| failure(GitDeliveryErrorKind::ScopeDrift, code))?;
    if unsafe_file_type(&metadata) || !metadata.is_file() {
        return Err(failure(GitDeliveryErrorKind::ScopeDrift, code));
    }
    let canonical =
        fs::canonicalize(&path).map_err(|_| failure(GitDeliveryErrorKind::ScopeDrift, code))?;
    if !same_path(&canonical, &path) {
        return Err(failure(GitDeliveryErrorKind::ScopeDrift, code));
    }
    Ok(canonical)
}

#[cfg(windows)]
fn unsafe_file_type(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn unsafe_file_type(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn comparable_path(path: &Path) -> String {
    let value = path.to_string_lossy().replace('/', "\\");
    let value = if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned()
    };
    value.trim_end_matches('\\').to_lowercase()
}

#[cfg(not(windows))]
fn comparable_path(path: &Path) -> String {
    path.to_string_lossy().trim_end_matches('/').to_owned()
}

fn same_path(left: &Path, right: &Path) -> bool {
    comparable_path(left) == comparable_path(right)
}

fn create_new_file(path: &Path, bytes: &[u8]) -> Result<(), GitDeliveryError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|_| {
            failure(
                GitDeliveryErrorKind::UnsafePath,
                "CONTROL_FILE_CREATE_FAILED",
            )
        })?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| {
            failure(
                GitDeliveryErrorKind::UnsafePath,
                "CONTROL_FILE_WRITE_FAILED",
            )
        })
}

fn read_regular_file(path: &Path, code: &'static str) -> Result<Vec<u8>, GitDeliveryError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| failure(GitDeliveryErrorKind::MetadataDrift, code))?;
    if unsafe_file_type(&metadata) || !metadata.is_file() {
        return Err(failure(GitDeliveryErrorKind::MetadataDrift, code));
    }
    fs::read(path).map_err(|_| failure(GitDeliveryErrorKind::MetadataDrift, code))
}

fn assert_regular_file_bytes(
    path: &Path,
    expected: &[u8],
    code: &'static str,
) -> Result<(), GitDeliveryError> {
    if read_regular_file(path, code)? != expected {
        return Err(failure(GitDeliveryErrorKind::MetadataDrift, code));
    }
    Ok(())
}

fn isolated_path(git_exe: &Path) -> Option<OsString> {
    let mut paths = Vec::new();
    if let Some(parent) = git_exe.parent() {
        paths.push(parent.to_path_buf());
    }
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        paths.push(PathBuf::from(system_root).join("System32"));
    }
    #[cfg(not(windows))]
    {
        paths.push(PathBuf::from("/usr/bin"));
        paths.push(PathBuf::from("/bin"));
    }
    std::env::join_paths(paths).ok()
}

fn prefixed_path_argument(prefix: &str, path: &Path) -> OsString {
    let mut argument = OsString::from(prefix);
    argument.push(path.as_os_str());
    argument
}

fn string_args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn repository_args(repository: &Path, values: &[&str]) -> Vec<OsString> {
    let mut args = Vec::with_capacity(values.len() + 2);
    args.push(OsString::from("-C"));
    args.push(repository.as_os_str().to_owned());
    args.extend(values.iter().map(OsString::from));
    args
}

fn require_success(
    output: io::Result<Output>,
    code: &'static str,
) -> Result<Output, GitDeliveryError> {
    let output = output.map_err(|_| failure(GitDeliveryErrorKind::GitFailed, code))?;
    if !output.status.success() {
        return Err(failure(GitDeliveryErrorKind::GitFailed, code));
    }
    Ok(output)
}

fn parse_object_id(bytes: &[u8]) -> Result<String, GitDeliveryError> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_| failure(GitDeliveryErrorKind::GitFailed, "GIT_OBJECT_ID_MALFORMED"))?
        .trim();
    if !matches!(value.len(), 40 | 64)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(failure(
            GitDeliveryErrorKind::GitFailed,
            "GIT_OBJECT_ID_MALFORMED",
        ));
    }
    Ok(value.to_owned())
}

fn read_head(runner: &GitRunner, repository: &Path) -> Result<String, GitDeliveryError> {
    let output = require_success(
        runner.output(&repository_args(
            repository,
            &["rev-parse", "--verify", "HEAD"],
        )),
        "HEAD_INSPECTION_FAILED",
    )?;
    parse_object_id(&output.stdout)
}

fn verify_only_main_ref(bytes: &[u8], expected: &str) -> Result<(), GitDeliveryError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| failure(GitDeliveryErrorKind::MetadataDrift, "REF_OUTPUT_MALFORMED"))?;
    let mut lines = text.lines().collect::<Vec<_>>();
    lines.sort_unstable();
    let mut expected_lines = vec![
        format!("{expected} HEAD"),
        format!("{expected} refs/heads/main"),
    ];
    expected_lines.sort_unstable();
    if lines != expected_lines {
        return Err(failure(
            GitDeliveryErrorKind::MetadataDrift,
            "UNEXPECTED_GIT_REFS",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new(label: &str) -> Self {
            let nonce = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos();
            Self(std::env::temp_dir().join(format!(
                "lattice-task032-git-{label}-{}-{nanos}-{nonce}",
                std::process::id()
            )))
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let temp = std::env::temp_dir();
            if self.0.starts_with(&temp)
                && self
                    .0
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("lattice-task032-git-"))
            {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }

    fn git_executable() -> Option<PathBuf> {
        let executable = if cfg!(windows) { "git.exe" } else { "git" };
        std::env::split_paths(&std::env::var_os("PATH")?).find_map(|directory| {
            let candidate = directory.join(executable);
            candidate.is_file().then_some(candidate)
        })
    }

    fn fixture(label: &str) -> Option<(TestRoot, IsolatedGitDelivery)> {
        let git = git_executable()?;
        let root = TestRoot::new(label);
        let delivery = IsolatedGitDelivery::provision(&root.0, git)
            .expect("isolated Git fixture should provision");
        Some((root, delivery))
    }

    #[test]
    fn exact_answer_is_tested_and_committed_once() {
        let Some((_root, delivery)) = fixture("happy") else {
            return;
        };
        fs::write(
            delivery.repo_path().join(ANSWER_FILE_NAME),
            EXPECTED_ANSWER_BYTES,
        )
        .expect("write answer");

        let evidence = delivery.verify_and_commit().expect("delivery commit");

        assert_ne!(evidence.commit_sha, evidence.baseline_commit);
        assert_eq!(evidence.changed_paths, [ANSWER_FILE_NAME]);
        assert_eq!(
            evidence.test_command_id,
            "git-diff-no-index-exact-answer-v1"
        );
    }

    #[test]
    fn foreign_path_is_rejected_before_commit() {
        let Some((_root, delivery)) = fixture("foreign") else {
            return;
        };
        fs::write(
            delivery.repo_path().join(ANSWER_FILE_NAME),
            EXPECTED_ANSWER_BYTES,
        )
        .expect("write answer");
        fs::write(delivery.repo_path().join("foreign.txt"), b"no\n").expect("write foreign path");

        let error = delivery.verify_and_commit().expect_err("scope must fail");

        assert_eq!(error.kind(), GitDeliveryErrorKind::ScopeDrift);
        assert_eq!(
            read_head(&delivery.runner, delivery.repo_path()).expect("read head"),
            delivery.baseline_commit
        );
    }

    #[test]
    fn git_pointer_drift_is_rejected_before_commit() {
        let Some((_root, delivery)) = fixture("metadata") else {
            return;
        };
        fs::write(
            delivery.repo_path().join(ANSWER_FILE_NAME),
            EXPECTED_ANSWER_BYTES,
        )
        .expect("write answer");
        fs::write(delivery.repo_path().join(".git"), b"gitdir: changed\n").expect("tamper pointer");

        let error = delivery
            .verify_and_commit()
            .expect_err("metadata drift must fail");

        assert_eq!(error.kind(), GitDeliveryErrorKind::MetadataDrift);
    }

    #[test]
    fn wrong_answer_fails_the_fixed_test_without_commit() {
        let Some((_root, delivery)) = fixture("test-fail") else {
            return;
        };
        fs::write(delivery.repo_path().join(ANSWER_FILE_NAME), b"wrong\n")
            .expect("write wrong answer");

        let error = delivery.verify_and_commit().expect_err("test must fail");

        assert_eq!(error.kind(), GitDeliveryErrorKind::TestFailed);
        assert_eq!(
            read_head(&delivery.runner, delivery.repo_path()).expect("read head"),
            delivery.baseline_commit
        );
    }

    #[cfg(windows)]
    #[test]
    fn hardlinked_answer_is_rejected_before_commit() {
        let Some((_root, delivery)) = fixture("hardlink") else {
            return;
        };
        let answer_path = delivery.repo_path().join(ANSWER_FILE_NAME);
        fs::write(&answer_path, EXPECTED_ANSWER_BYTES).expect("write answer");
        fs::hard_link(&answer_path, delivery.root.join("answer-hardlink.txt"))
            .expect("create hardlink");

        let error = delivery
            .verify_and_commit()
            .expect_err("hardlinked answer must fail closed");

        assert_eq!(error.kind(), GitDeliveryErrorKind::ScopeDrift);
        assert_eq!(error.code(), "ANSWER_HARDLINK_COUNT_DRIFT");
        assert_eq!(
            read_head(&delivery.runner, delivery.repo_path()).expect("read head"),
            delivery.baseline_commit
        );
    }

    #[cfg(windows)]
    #[test]
    fn answer_with_alternate_data_stream_is_rejected_before_commit() {
        let Some((_root, delivery)) = fixture("alternate-stream") else {
            return;
        };
        let answer_path = delivery.repo_path().join(ANSWER_FILE_NAME);
        fs::write(&answer_path, EXPECTED_ANSWER_BYTES).expect("write answer");
        let mut stream_path = answer_path.as_os_str().to_owned();
        stream_path.push(":lattice-task032-test");
        if fs::write(PathBuf::from(stream_path), b"hidden\n").is_err() {
            return;
        }

        let error = delivery
            .verify_and_commit()
            .expect_err("alternate data stream must fail closed");

        assert_eq!(error.kind(), GitDeliveryErrorKind::ScopeDrift);
        assert_eq!(error.code(), "ANSWER_ALTERNATE_DATA_STREAM_DRIFT");
        assert_eq!(
            read_head(&delivery.runner, delivery.repo_path()).expect("read head"),
            delivery.baseline_commit
        );
    }
}
