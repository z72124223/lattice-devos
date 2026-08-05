use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

use crate::error::{GraphifyAdapterError, GraphifyAdapterErrorKind, GraphifyAdapterResult};

static SNAPSHOT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Bounded snapshot-materialization limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
// The repeated prefix makes each snapshot resource bound explicit to callers.
#[allow(clippy::struct_field_names)]
pub struct SnapshotLimits {
    max_tree_bytes: usize,
    max_tracked_entries: usize,
    max_source_bytes: u64,
    max_total_source_bytes: u64,
}

impl SnapshotLimits {
    #[must_use]
    pub const fn new(
        max_tree_bytes: usize,
        max_tracked_entries: usize,
        max_source_bytes: u64,
        max_total_source_bytes: u64,
    ) -> Self {
        Self {
            max_tree_bytes,
            max_tracked_entries,
            max_source_bytes,
            max_total_source_bytes,
        }
    }

    #[must_use]
    pub const fn max_tree_bytes(self) -> usize {
        self.max_tree_bytes
    }

    #[must_use]
    pub const fn max_tracked_entries(self) -> usize {
        self.max_tracked_entries
    }

    #[must_use]
    pub const fn max_source_bytes(self) -> u64 {
        self.max_source_bytes
    }

    #[must_use]
    pub const fn max_total_source_bytes(self) -> u64 {
        self.max_total_source_bytes
    }
}

impl Default for SnapshotLimits {
    fn default() -> Self {
        Self::new(16 * 1024 * 1024, 50_000, 8 * 1024 * 1024, 256 * 1024 * 1024)
    }
}

/// Process-owned exact Git snapshot configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitSnapshotConfig {
    git_executable: PathBuf,
    expected_git_sha256: String,
    repository_root: PathBuf,
    snapshot_root: PathBuf,
    limits: SnapshotLimits,
}

impl GitSnapshotConfig {
    /// Creates an exact Git-object snapshot configuration.
    ///
    /// # Errors
    ///
    /// Returns a configuration error when a required path is not absolute,
    /// the Git identity digest is malformed, or a resource bound is invalid.
    pub fn new(
        git_executable: impl Into<PathBuf>,
        expected_git_sha256: impl Into<String>,
        repository_root: impl Into<PathBuf>,
        snapshot_root: impl Into<PathBuf>,
        limits: SnapshotLimits,
    ) -> GraphifyAdapterResult<Self> {
        let git_executable = git_executable.into();
        let expected_git_sha256 = expected_git_sha256.into();
        let repository_root = repository_root.into();
        let snapshot_root = snapshot_root.into();
        if !git_executable.is_absolute()
            || !repository_root.is_absolute()
            || !snapshot_root.is_absolute()
            || !is_lowercase_sha256(&expected_git_sha256)
            || limits.max_tree_bytes == 0
            || limits.max_tracked_entries == 0
            || limits.max_source_bytes == 0
            || limits.max_total_source_bytes < limits.max_source_bytes
        {
            return Err(error(
                GraphifyAdapterErrorKind::Configuration,
                "GRAPHIFY_SNAPSHOT_CONFIG_REJECTED",
            ));
        }
        Ok(Self {
            git_executable,
            expected_git_sha256,
            repository_root,
            snapshot_root,
            limits,
        })
    }

    #[must_use]
    pub fn git_executable(&self) -> &Path {
        &self.git_executable
    }

    #[must_use]
    pub fn expected_git_sha256(&self) -> &str {
        &self.expected_git_sha256
    }

    #[must_use]
    pub fn repository_root(&self) -> &Path {
        &self.repository_root
    }

    #[must_use]
    pub fn snapshot_root(&self) -> &Path {
        &self.snapshot_root
    }

    #[must_use]
    pub const fn limits(&self) -> SnapshotLimits {
        self.limits
    }
}

/// One source copied from an exact Git blob into the immutable snapshot.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SnapshotSource {
    relative_path: String,
    content_sha256: String,
    byte_length: u64,
}

impl SnapshotSource {
    #[must_use]
    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    #[must_use]
    pub fn content_sha256(&self) -> &str {
        &self.content_sha256
    }

    #[must_use]
    pub const fn byte_length(&self) -> u64 {
        self.byte_length
    }
}

/// Exact, tracked-only filesystem snapshot evidence used as Graphify input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedSnapshot {
    commit_id: String,
    tree_id: String,
    root: PathBuf,
    sources: Vec<SnapshotSource>,
    manifest_sha256: String,
    exclusion_sha256: String,
    snapshot_sha256: String,
    excluded_path_count: usize,
}

impl MaterializedSnapshot {
    #[must_use]
    pub fn commit_id(&self) -> &str {
        &self.commit_id
    }

    #[must_use]
    pub fn tree_id(&self) -> &str {
        &self.tree_id
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn sources(&self) -> &[SnapshotSource] {
        &self.sources
    }

    #[must_use]
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    #[must_use]
    pub fn exclusion_sha256(&self) -> &str {
        &self.exclusion_sha256
    }

    #[must_use]
    pub fn snapshot_sha256(&self) -> &str {
        &self.snapshot_sha256
    }

    #[must_use]
    pub const fn excluded_path_count(&self) -> usize {
        self.excluded_path_count
    }

    pub(crate) fn source_digests(&self) -> BTreeMap<&str, &str> {
        self.sources
            .iter()
            .map(|source| {
                (
                    source.relative_path.as_str(),
                    source.content_sha256.as_str(),
                )
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn for_test(root: PathBuf, sources: Vec<(&str, &[u8])>) -> Self {
        let sources = sources
            .into_iter()
            .map(|(relative_path, bytes)| SnapshotSource {
                relative_path: relative_path.to_owned(),
                content_sha256: sha256_bytes(bytes),
                byte_length: bytes.len() as u64,
            })
            .collect::<Vec<_>>();
        Self {
            commit_id: "1".repeat(40),
            tree_id: "2".repeat(40),
            root,
            sources,
            manifest_sha256: "3".repeat(64),
            exclusion_sha256: "4".repeat(64),
            snapshot_sha256: "5".repeat(64),
            excluded_path_count: 0,
        }
    }
}

/// Read-only materializer backed exclusively by exact Git object reads.
#[derive(Clone, Debug)]
pub struct ExactGitSnapshotMaterializer {
    config: GitSnapshotConfig,
    bridge: SnapshotBridge,
}

impl ExactGitSnapshotMaterializer {
    #[must_use]
    pub fn new(config: GitSnapshotConfig) -> Self {
        Self::with_bridge(config, SnapshotBridge::new())
    }

    #[must_use]
    pub const fn with_bridge(config: GitSnapshotConfig, bridge: SnapshotBridge) -> Self {
        Self { config, bridge }
    }

    #[must_use]
    pub const fn config(&self) -> &GitSnapshotConfig {
        &self.config
    }

    #[must_use]
    pub const fn bridge(&self) -> &SnapshotBridge {
        &self.bridge
    }

    /// Materializes programming-language sources from one full commit object.
    ///
    /// Untracked worktree files are unreachable because every byte is read by
    /// blob id from `git ls-tree`/`git cat-file`, never from the worktree.
    ///
    /// # Errors
    ///
    /// Returns an error if the commit is not an exact object id, Git identity
    /// or object reads fail, a tracked path is unsafe, a resource bound is
    /// exceeded, or the materialized snapshot cannot be verified exactly.
    // The materialization path is an ordered fail-closed protocol. Keeping the
    // checks linear makes the before/after identity and binding gates auditable.
    #[allow(clippy::too_many_lines)]
    pub fn materialize(&self, commit_id: &str) -> GraphifyAdapterResult<MaterializedSnapshot> {
        if !is_git_object_id(commit_id) {
            return Err(error(
                GraphifyAdapterErrorKind::GitObject,
                "GRAPHIFY_SNAPSHOT_COMMIT_REJECTED",
            ));
        }
        let repository_root = fs::canonicalize(&self.config.repository_root).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::Configuration,
                "GRAPHIFY_REPOSITORY_ROOT_UNAVAILABLE",
            )
        })?;
        if !repository_root.is_dir() {
            return Err(error(
                GraphifyAdapterErrorKind::Configuration,
                "GRAPHIFY_REPOSITORY_ROOT_REJECTED",
            ));
        }
        fs::create_dir_all(&self.config.snapshot_root).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::SnapshotIo,
                "GRAPHIFY_SNAPSHOT_ROOT_CREATE_FAILED",
            )
        })?;
        let snapshot_parent = fs::canonicalize(&self.config.snapshot_root).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::SnapshotIo,
                "GRAPHIFY_SNAPSHOT_ROOT_RESOLVE_FAILED",
            )
        })?;
        let git_home = snapshot_parent.join(".git-runtime-home");
        fs::create_dir_all(&git_home).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::SnapshotIo,
                "GRAPHIFY_GIT_HOME_CREATE_FAILED",
            )
        })?;

        verify_file_identity(
            &self.config.git_executable,
            &self.config.expected_git_sha256,
            GraphifyAdapterErrorKind::GitIdentity,
            "GRAPHIFY_GIT_IDENTITY_REJECTED",
        )?;

        let resolved_commit = git_text(
            &self.config.git_executable,
            &repository_root,
            &git_home,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{commit_id}^{{commit}}"),
            ],
        )?;
        if resolved_commit != commit_id {
            return Err(error(
                GraphifyAdapterErrorKind::GitObject,
                "GRAPHIFY_SNAPSHOT_COMMIT_MISMATCH",
            ));
        }
        let tree_id = git_text(
            &self.config.git_executable,
            &repository_root,
            &git_home,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{commit_id}^{{tree}}"),
            ],
        )?;
        if !is_git_object_id(&tree_id) {
            return Err(error(
                GraphifyAdapterErrorKind::GitObject,
                "GRAPHIFY_SNAPSHOT_TREE_REJECTED",
            ));
        }
        let tree_output = git_bytes(
            &self.config.git_executable,
            &repository_root,
            &git_home,
            &["ls-tree", "-r", "-z", "-l", "--full-tree", &tree_id],
        )?;
        if tree_output.len() > self.config.limits.max_tree_bytes {
            return Err(error(
                GraphifyAdapterErrorKind::SnapshotLimit,
                "GRAPHIFY_SNAPSHOT_TREE_OUTPUT_LIMIT",
            ));
        }
        let (mut entries, mut exclusions) = parse_tree(&tree_output, self.config.limits)?;
        if entries.is_empty() {
            return Err(error(
                GraphifyAdapterErrorKind::EmptyAnalysis,
                "GRAPHIFY_SNAPSHOT_HAS_NO_CODE",
            ));
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        exclusions.sort();

        let sequence = SNAPSHOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let directory_name = format!(
            "snapshot-{}-{}-{sequence}",
            &commit_id[..12],
            std::process::id()
        );
        let snapshot_root = snapshot_parent.join(directory_name);
        fs::create_dir(&snapshot_root).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::SnapshotIo,
                "GRAPHIFY_SNAPSHOT_CREATE_FAILED",
            )
        })?;
        // This empty marker establishes the snapshot as the nearest VCS root.
        // Without it, Graphify inherits an ancestor repository's `target/`
        // ignore and can emit a misleading zero-node success.
        fs::create_dir(snapshot_root.join(".git")).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::SnapshotIo,
                "GRAPHIFY_SNAPSHOT_VCS_BOUNDARY_FAILED",
            )
        })?;

        let mut sources = Vec::with_capacity(entries.len());
        let mut total_bytes = 0_u64;
        for entry in entries {
            total_bytes = total_bytes.checked_add(entry.byte_length).ok_or_else(|| {
                error(
                    GraphifyAdapterErrorKind::SnapshotLimit,
                    "GRAPHIFY_SNAPSHOT_TOTAL_SIZE_OVERFLOW",
                )
            })?;
            if total_bytes > self.config.limits.max_total_source_bytes {
                return Err(error(
                    GraphifyAdapterErrorKind::SnapshotLimit,
                    "GRAPHIFY_SNAPSHOT_TOTAL_SIZE_LIMIT",
                ));
            }
            let blob = git_bytes(
                &self.config.git_executable,
                &repository_root,
                &git_home,
                &["cat-file", "blob", &entry.object_id],
            )?;
            if blob.len() as u64 != entry.byte_length {
                return Err(error(
                    GraphifyAdapterErrorKind::GitObject,
                    "GRAPHIFY_SNAPSHOT_BLOB_SIZE_MISMATCH",
                ));
            }
            let destination = safe_join(&snapshot_root, &entry.path)?;
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|_| {
                    error(
                        GraphifyAdapterErrorKind::SnapshotIo,
                        "GRAPHIFY_SNAPSHOT_DIRECTORY_CREATE_FAILED",
                    )
                })?;
            }
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&destination)
                .map_err(|_| {
                    error(
                        GraphifyAdapterErrorKind::SnapshotIo,
                        "GRAPHIFY_SNAPSHOT_FILE_CREATE_FAILED",
                    )
                })?;
            file.write_all(&blob).map_err(|_| {
                error(
                    GraphifyAdapterErrorKind::SnapshotIo,
                    "GRAPHIFY_SNAPSHOT_FILE_WRITE_FAILED",
                )
            })?;
            file.sync_all().map_err(|_| {
                error(
                    GraphifyAdapterErrorKind::SnapshotIo,
                    "GRAPHIFY_SNAPSHOT_FILE_SYNC_FAILED",
                )
            })?;
            let mut permissions = file
                .metadata()
                .map_err(|_| {
                    error(
                        GraphifyAdapterErrorKind::SnapshotIo,
                        "GRAPHIFY_SNAPSHOT_FILE_METADATA_FAILED",
                    )
                })?
                .permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&destination, permissions).map_err(|_| {
                error(
                    GraphifyAdapterErrorKind::SnapshotIo,
                    "GRAPHIFY_SNAPSHOT_FILE_READONLY_FAILED",
                )
            })?;
            sources.push(SnapshotSource {
                relative_path: entry.path,
                content_sha256: sha256_bytes(&blob),
                byte_length: entry.byte_length,
            });
        }

        verify_file_identity(
            &self.config.git_executable,
            &self.config.expected_git_sha256,
            GraphifyAdapterErrorKind::GitIdentity,
            "GRAPHIFY_GIT_IDENTITY_CHANGED",
        )?;
        let manifest_sha256 = manifest_digest(&sources);
        let exclusion_sha256 = exclusion_digest(&exclusions);
        let snapshot_sha256 = framed_digest(&[
            commit_id.as_bytes(),
            tree_id.as_bytes(),
            manifest_sha256.as_bytes(),
            exclusion_sha256.as_bytes(),
        ]);
        let snapshot = MaterializedSnapshot {
            commit_id: commit_id.to_owned(),
            tree_id,
            root: snapshot_root,
            sources,
            manifest_sha256,
            exclusion_sha256,
            snapshot_sha256,
            excluded_path_count: exclusions.len(),
        };
        verify_snapshot_binding(&snapshot)?;
        Ok(snapshot)
    }
}

/// In-process path bridge shared by the distinct snapshot and analysis ports.
///
/// The bridge holds only ephemeral LATTICE-owned filesystem handles. Contract
/// evidence remains the binding authority and every lookup rechecks its exact
/// commit/tree/manifest/exclusion key before the path is used.
#[derive(Clone, Debug, Default)]
pub struct SnapshotBridge {
    inner: Arc<Mutex<BTreeMap<String, MaterializedSnapshot>>>,
}

impl SnapshotBridge {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn insert(
        &self,
        key: String,
        snapshot: MaterializedSnapshot,
    ) -> GraphifyAdapterResult<()> {
        let mut snapshots = self.inner.lock().map_err(|_| {
            error(
                GraphifyAdapterErrorKind::SnapshotChanged,
                "GRAPHIFY_SNAPSHOT_BRIDGE_POISONED",
            )
        })?;
        snapshots.insert(key, snapshot);
        while snapshots.len() > 16 {
            let first = snapshots.keys().next().cloned().ok_or_else(|| {
                error(
                    GraphifyAdapterErrorKind::SnapshotChanged,
                    "GRAPHIFY_SNAPSHOT_BRIDGE_EVICTION_FAILED",
                )
            })?;
            snapshots.remove(&first);
        }
        Ok(())
    }

    pub(crate) fn get(&self, key: &str) -> GraphifyAdapterResult<MaterializedSnapshot> {
        self.inner
            .lock()
            .map_err(|_| {
                error(
                    GraphifyAdapterErrorKind::SnapshotChanged,
                    "GRAPHIFY_SNAPSHOT_BRIDGE_POISONED",
                )
            })?
            .get(key)
            .cloned()
            .ok_or_else(|| {
                error(
                    GraphifyAdapterErrorKind::SnapshotChanged,
                    "GRAPHIFY_SNAPSHOT_BRIDGE_BINDING_MISSING",
                )
            })
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct TreeEntry {
    path: String,
    object_id: String,
    byte_length: u64,
}

fn parse_tree(
    output: &[u8],
    limits: SnapshotLimits,
) -> GraphifyAdapterResult<(Vec<TreeEntry>, Vec<String>)> {
    let mut entries = Vec::new();
    let mut exclusions = Vec::new();
    let mut seen = BTreeSet::new();
    for raw_record in output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        if seen.len() >= limits.max_tracked_entries {
            return Err(error(
                GraphifyAdapterErrorKind::SnapshotLimit,
                "GRAPHIFY_SNAPSHOT_ENTRY_LIMIT",
            ));
        }
        let tab = raw_record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| {
                error(
                    GraphifyAdapterErrorKind::GitObject,
                    "GRAPHIFY_SNAPSHOT_TREE_RECORD_MALFORMED",
                )
            })?;
        let header = std::str::from_utf8(&raw_record[..tab]).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::GitObject,
                "GRAPHIFY_SNAPSHOT_TREE_HEADER_NON_UTF8",
            )
        })?;
        let path = std::str::from_utf8(&raw_record[tab + 1..]).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::UnsafeSnapshot,
                "GRAPHIFY_SNAPSHOT_PATH_NON_UTF8",
            )
        })?;
        validate_relative_path(path)?;
        if !seen.insert(path.to_owned()) {
            return Err(error(
                GraphifyAdapterErrorKind::GitObject,
                "GRAPHIFY_SNAPSHOT_DUPLICATE_PATH",
            ));
        }
        let fields: Vec<&str> = header.split_ascii_whitespace().collect();
        if fields.len() != 4 || fields[1] != "blob" || !is_git_object_id(fields[2]) {
            return Err(error(
                GraphifyAdapterErrorKind::UnsafeSnapshot,
                "GRAPHIFY_SNAPSHOT_UNSUPPORTED_TREE_ENTRY",
            ));
        }
        if fields[0] != "100644" && fields[0] != "100755" {
            return Err(error(
                GraphifyAdapterErrorKind::UnsafeSnapshot,
                "GRAPHIFY_SNAPSHOT_SYMLINK_OR_SPECIAL_ENTRY",
            ));
        }
        let byte_length = fields[3].parse::<u64>().map_err(|_| {
            error(
                GraphifyAdapterErrorKind::GitObject,
                "GRAPHIFY_SNAPSHOT_TREE_SIZE_MALFORMED",
            )
        })?;
        let reason = if is_sensitive_path(path) {
            Some("sensitive")
        } else if !is_graphify_code_path(path) {
            Some("non_code")
        } else {
            None
        };
        if let Some(reason) = reason {
            exclusions.push(format!("{path}\0{}\0{reason}", fields[2]));
            continue;
        }
        if byte_length > limits.max_source_bytes {
            return Err(error(
                GraphifyAdapterErrorKind::SnapshotLimit,
                "GRAPHIFY_SNAPSHOT_SOURCE_SIZE_LIMIT",
            ));
        }
        entries.push(TreeEntry {
            path: path.to_owned(),
            object_id: fields[2].to_owned(),
            byte_length,
        });
    }
    Ok((entries, exclusions))
}

pub(crate) fn verify_snapshot_binding(
    snapshot: &MaterializedSnapshot,
) -> GraphifyAdapterResult<()> {
    if !snapshot.root.is_dir() || !snapshot.root.join(".git").is_dir() {
        return Err(error(
            GraphifyAdapterErrorKind::SnapshotChanged,
            "GRAPHIFY_SNAPSHOT_BOUNDARY_CHANGED",
        ));
    }
    let expected = snapshot.source_digests();
    let mut observed = BTreeMap::new();
    let mut pending = vec![snapshot.root.clone()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::SnapshotChanged,
                "GRAPHIFY_SNAPSHOT_ENUMERATION_FAILED",
            )
        })? {
            let entry = entry.map_err(|_| {
                error(
                    GraphifyAdapterErrorKind::SnapshotChanged,
                    "GRAPHIFY_SNAPSHOT_ENTRY_UNREADABLE",
                )
            })?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|_| {
                error(
                    GraphifyAdapterErrorKind::SnapshotChanged,
                    "GRAPHIFY_SNAPSHOT_ENTRY_TYPE_UNREADABLE",
                )
            })?;
            if file_type.is_symlink() {
                return Err(error(
                    GraphifyAdapterErrorKind::SnapshotChanged,
                    "GRAPHIFY_SNAPSHOT_REPARSE_REJECTED",
                ));
            }
            if file_type.is_dir() {
                if path == snapshot.root.join(".git") {
                    continue;
                }
                pending.push(path);
                continue;
            }
            if !file_type.is_file() {
                return Err(error(
                    GraphifyAdapterErrorKind::SnapshotChanged,
                    "GRAPHIFY_SNAPSHOT_SPECIAL_FILE_REJECTED",
                ));
            }
            let relative = path.strip_prefix(&snapshot.root).map_err(|_| {
                error(
                    GraphifyAdapterErrorKind::SnapshotChanged,
                    "GRAPHIFY_SNAPSHOT_PATH_ESCAPE",
                )
            })?;
            let relative = relative.to_string_lossy().replace('\\', "/");
            validate_relative_path(&relative)?;
            if observed.insert(relative, file_sha256(&path)?).is_some() {
                return Err(error(
                    GraphifyAdapterErrorKind::SnapshotChanged,
                    "GRAPHIFY_SNAPSHOT_DUPLICATE_OBSERVED_PATH",
                ));
            }
        }
    }
    if observed.len() != expected.len()
        || observed
            .iter()
            .any(|(path, digest)| expected.get(path.as_str()) != Some(&digest.as_str()))
    {
        return Err(error(
            GraphifyAdapterErrorKind::SnapshotChanged,
            "GRAPHIFY_SNAPSHOT_BINDING_CHANGED",
        ));
    }
    Ok(())
}

fn git_text(
    executable: &Path,
    repository_root: &Path,
    git_home: &Path,
    arguments: &[&str],
) -> GraphifyAdapterResult<String> {
    let bytes = git_bytes(executable, repository_root, git_home, arguments)?;
    let text = String::from_utf8(bytes).map_err(|_| {
        error(
            GraphifyAdapterErrorKind::GitObject,
            "GRAPHIFY_GIT_OUTPUT_NON_UTF8",
        )
    })?;
    let value = text.trim_end_matches(['\r', '\n']);
    if value.is_empty() || value.contains(['\r', '\n']) {
        return Err(error(
            GraphifyAdapterErrorKind::GitObject,
            "GRAPHIFY_GIT_TEXT_OUTPUT_MALFORMED",
        ));
    }
    Ok(value.to_owned())
}

fn git_bytes(
    executable: &Path,
    repository_root: &Path,
    git_home: &Path,
    arguments: &[&str],
) -> GraphifyAdapterResult<Vec<u8>> {
    let mut command = Command::new(executable);
    command.current_dir(repository_root);
    command.arg("--no-optional-locks");
    command.args(arguments);
    apply_git_environment(&mut command, executable, git_home)?;
    let output = command.output().map_err(|_| {
        error(
            GraphifyAdapterErrorKind::GitObject,
            "GRAPHIFY_GIT_PROCESS_START_FAILED",
        )
    })?;
    if !output.status.success() || !output.stderr.is_empty() {
        return Err(error(
            GraphifyAdapterErrorKind::GitObject,
            "GRAPHIFY_GIT_PROCESS_REJECTED",
        ));
    }
    Ok(output.stdout)
}

fn apply_git_environment(
    command: &mut Command,
    executable: &Path,
    git_home: &Path,
) -> GraphifyAdapterResult<()> {
    command.env_clear();
    copy_system_environment(command);
    let executable_parent = executable.parent().ok_or_else(|| {
        error(
            GraphifyAdapterErrorKind::Configuration,
            "GRAPHIFY_GIT_EXECUTABLE_PARENT_MISSING",
        )
    })?;
    let mut paths = vec![executable_parent.to_path_buf()];
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        paths.push(PathBuf::from(system_root).join("System32"));
    } else {
        paths.push(PathBuf::from("/usr/bin"));
        paths.push(PathBuf::from("/bin"));
    }
    let path = std::env::join_paths(paths).map_err(|_| {
        error(
            GraphifyAdapterErrorKind::Configuration,
            "GRAPHIFY_GIT_MINIMAL_PATH_REJECTED",
        )
    })?;
    command.env("PATH", path);
    command.env("HOME", git_home);
    command.env("USERPROFILE", git_home);
    command.env("GIT_CONFIG_NOSYSTEM", "1");
    command.env(
        "GIT_CONFIG_GLOBAL",
        if cfg!(windows) { "NUL" } else { "/dev/null" },
    );
    command.env("GIT_OPTIONAL_LOCKS", "0");
    command.env("GIT_NO_REPLACE_OBJECTS", "1");
    command.env("GIT_NO_LAZY_FETCH", "1");
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.env("LC_ALL", "C");
    Ok(())
}

pub(crate) fn copy_system_environment(command: &mut Command) {
    for name in ["SystemRoot", "WINDIR", "ComSpec", "PATHEXT", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
}

pub(crate) fn verify_file_identity(
    path: &Path,
    expected_sha256: &str,
    kind: GraphifyAdapterErrorKind,
    code: &'static str,
) -> GraphifyAdapterResult<()> {
    if file_sha256(path)? == expected_sha256 {
        Ok(())
    } else {
        Err(error(kind, code))
    }
}

pub(crate) fn file_sha256(path: &Path) -> GraphifyAdapterResult<String> {
    let mut file = File::open(path).map_err(|_| {
        error(
            GraphifyAdapterErrorKind::SnapshotIo,
            "GRAPHIFY_FILE_HASH_OPEN_FAILED",
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let read = file.read(&mut buffer).map_err(|_| {
            error(
                GraphifyAdapterErrorKind::SnapshotIo,
                "GRAPHIFY_FILE_HASH_READ_FAILED",
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_digest(&hasher.finalize()))
}

pub(crate) fn framed_digest(fields: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for field in fields {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field);
    }
    hex_digest(&hasher.finalize())
}

pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    hex_digest(&Sha256::digest(bytes))
}

fn manifest_digest(sources: &[SnapshotSource]) -> String {
    let mut hasher = Sha256::new();
    for source in sources {
        hash_field(&mut hasher, source.relative_path.as_bytes());
        hash_field(&mut hasher, source.content_sha256.as_bytes());
        hash_field(&mut hasher, &source.byte_length.to_be_bytes());
    }
    hex_digest(&hasher.finalize())
}

fn exclusion_digest(exclusions: &[String]) -> String {
    let mut hasher = Sha256::new();
    for exclusion in exclusions {
        hash_field(&mut hasher, exclusion.as_bytes());
    }
    hex_digest(&hasher.finalize())
}

fn hash_field(hasher: &mut Sha256, field: &[u8]) {
    hasher.update((field.len() as u64).to_be_bytes());
    hasher.update(field);
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn safe_join(root: &Path, relative: &str) -> GraphifyAdapterResult<PathBuf> {
    validate_relative_path(relative)?;
    let mut path = root.to_path_buf();
    for segment in relative.split('/') {
        path.push(segment);
    }
    Ok(path)
}

pub(crate) fn validate_relative_path(path: &str) -> GraphifyAdapterResult<()> {
    if path.is_empty()
        || path.len() > 4_096
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.chars().any(char::is_control)
    {
        return Err(error(
            GraphifyAdapterErrorKind::UnsafeSnapshot,
            "GRAPHIFY_SNAPSHOT_PATH_REJECTED",
        ));
    }
    for segment in path.split('/') {
        let lower = segment.to_ascii_lowercase();
        let stem = lower.split('.').next().unwrap_or_default();
        let reserved = matches!(stem, "con" | "prn" | "aux" | "nul")
            || (stem.len() == 4
                && (stem.starts_with("com") || stem.starts_with("lpt"))
                && stem.as_bytes()[3].is_ascii_digit());
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.ends_with([' ', '.'])
            || reserved
        {
            return Err(error(
                GraphifyAdapterErrorKind::UnsafeSnapshot,
                "GRAPHIFY_SNAPSHOT_PATH_SEGMENT_REJECTED",
            ));
        }
    }
    Ok(())
}

fn is_sensitive_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let mut segments = lower.split('/');
    let file_name = segments.next_back().unwrap_or_default();
    if file_name == ".env"
        || file_name.starts_with(".env.")
        || file_name == ".envrc"
        || file_name.starts_with(".envrc.")
        || matches!(
            file_name,
            ".pgpass"
                | ".netrc"
                | ".npmrc"
                | ".pypirc"
                | ".git-credentials"
                | "id_rsa"
                | "id_dsa"
                | "id_ecdsa"
                | "id_ed25519"
        )
        || [".pem", ".key", ".p12", ".pfx", ".p8", ".der"]
            .iter()
            .any(|suffix| file_name.ends_with(suffix))
    {
        return true;
    }
    let secret_prone = [
        ".json", ".yaml", ".yml", ".toml", ".ini", ".cfg", ".conf", ".txt",
    ]
    .iter()
    .any(|suffix| file_name.ends_with(suffix));
    lower.split('/').any(|segment| {
        matches!(segment, ".ssh" | ".gnupg" | ".aws" | ".gcloud")
            || (secret_prone && matches!(segment, "secrets" | ".secrets" | "credentials"))
    }) || (secret_prone
        && [
            "secret",
            "credential",
            "password",
            "passwd",
            "private_key",
            "service_account",
            "api_key",
        ]
        .iter()
        .any(|keyword| file_name.contains(keyword)))
}

fn is_graphify_code_path(path: &str) -> bool {
    let Some((_, extension)) = path.rsplit_once('.') else {
        return false;
    };
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "py" | "ts"
            | "tsx"
            | "mts"
            | "cts"
            | "js"
            | "jsx"
            | "mjs"
            | "cjs"
            | "ejs"
            | "ets"
            | "go"
            | "rs"
            | "java"
            | "groovy"
            | "gradle"
            | "cpp"
            | "cc"
            | "cxx"
            | "c"
            | "h"
            | "hpp"
            | "cu"
            | "cuh"
            | "metal"
            | "rb"
            | "rake"
            | "swift"
            | "kt"
            | "kts"
            | "cs"
            | "scala"
            | "php"
            | "lua"
            | "luau"
            | "zig"
            | "ps1"
            | "psm1"
            | "psd1"
            | "ex"
            | "exs"
            | "m"
            | "mm"
            | "jl"
            | "vue"
            | "svelte"
            | "astro"
            | "dart"
            | "v"
            | "sv"
            | "svh"
            | "sql"
            | "r"
            | "f"
            | "f90"
            | "f95"
            | "f03"
            | "f08"
            | "pas"
            | "pp"
            | "dpr"
            | "dpk"
            | "lpr"
            | "inc"
            | "dfm"
            | "lfm"
            | "lpk"
            | "sh"
            | "bash"
            | "json"
            | "tf"
            | "tfvars"
            | "hcl"
            | "dm"
            | "dme"
            | "dmi"
            | "dmm"
            | "dmf"
            | "sln"
            | "slnx"
            | "csproj"
            | "fsproj"
            | "vbproj"
            | "xaml"
            | "razor"
            | "cshtml"
            | "cls"
            | "trigger"
    )
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_git_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

const fn error(kind: GraphifyAdapterErrorKind, code: &'static str) -> GraphifyAdapterError {
    GraphifyAdapterError::new(kind, code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_parser_sorts_later_and_excludes_sensitive_or_non_code() {
        let output = concat!(
            "100644 blob aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 4\tsrc/z.rs\0",
            "100644 blob bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb 5\t.env\0",
            "100644 blob cccccccccccccccccccccccccccccccccccccccc 6\tREADME.md\0",
            "100644 blob dddddddddddddddddddddddddddddddddddddddd 7\tsrc/a.rs\0",
        );
        let (mut entries, exclusions) =
            parse_tree(output.as_bytes(), SnapshotLimits::default()).expect("tree");
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec!["src/a.rs", "src/z.rs"]
        );
        assert_eq!(exclusions.len(), 2);
    }

    #[test]
    fn path_validation_rejects_escape_ads_and_windows_reserved_names() {
        for path in [
            "../secret.rs",
            "src\\escape.rs",
            "src/x.rs:secret",
            "src/CON.rs",
        ] {
            assert!(validate_relative_path(path).is_err(), "{path}");
        }
        assert!(validate_relative_path("src/lib.rs").is_ok());
    }

    #[test]
    fn git_environment_disables_replacements_and_lazy_fetches() {
        let executable = std::env::current_exe().expect("current test executable");
        let mut command = Command::new(&executable);
        apply_git_environment(&mut command, &executable, &std::env::temp_dir())
            .expect("minimal Git environment");
        let environment = command
            .get_envs()
            .filter_map(|(name, value)| value.map(|value| (name, value)))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            environment.get(std::ffi::OsStr::new("GIT_NO_REPLACE_OBJECTS")),
            Some(&std::ffi::OsStr::new("1"))
        );
        assert_eq!(
            environment.get(std::ffi::OsStr::new("GIT_NO_LAZY_FETCH")),
            Some(&std::ffi::OsStr::new("1"))
        );
    }
}
