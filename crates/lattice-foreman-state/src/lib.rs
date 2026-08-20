//! Secret-free foreman snapshot validation, replay projection, and watchdog logic.

use std::collections::BTreeMap;

const SNAPSHOT_SCHEMA: &str = "lattice.foreman-snapshot/1.0";
const MAX_REFERENCE_BYTES: usize = 256;

/// Closed worker coordination state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForemanState {
    Active,
    Blocked,
    Completed,
}

/// Stable rejection and replay failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotError {
    MalformedReference,
    ForbiddenContent,
    MissingBlocker,
    UnexpectedBlocker,
    GenerationRollback,
    DuplicateWorkerIdentity,
}

/// One versioned, bounded coordination record. It deliberately has no free-form
/// transcript, command, path, environment, or credential field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForemanSnapshot {
    worker: String,
    thread: String,
    task: String,
    branch: String,
    worktree: String,
    head: String,
    state: ForemanState,
    blocker: Option<String>,
    heartbeat: String,
    evidence: String,
    generation: u64,
}

impl ForemanSnapshot {
    /// # Errors
    ///
    /// Returns a typed rejection for malformed, secret-bearing, or
    /// state-incompatible fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        worker: impl Into<String>,
        thread: impl Into<String>,
        task: impl Into<String>,
        branch: impl Into<String>,
        worktree: impl Into<String>,
        head: impl Into<String>,
        state: ForemanState,
        blocker: Option<String>,
        heartbeat: impl Into<String>,
        evidence: impl Into<String>,
        generation: u64,
    ) -> Result<Self, SnapshotError> {
        let worker = bounded_reference(worker.into())?;
        let thread = bounded_reference(thread.into())?;
        let task = bounded_reference(task.into())?;
        let branch = bounded_reference(branch.into())?;
        let worktree = bounded_reference(worktree.into())?;
        let head = head.into();
        if !is_hex(&head, 40) {
            return Err(SnapshotError::MalformedReference);
        }
        let heartbeat = digest_pointer(heartbeat.into(), "heartbeat")?;
        let evidence = digest_pointer(evidence.into(), "evidence")?;
        if generation == 0 {
            return Err(SnapshotError::GenerationRollback);
        }
        let blocker = blocker.map(bounded_reference).transpose()?;
        match (state, blocker.is_some()) {
            (ForemanState::Blocked, false) => return Err(SnapshotError::MissingBlocker),
            (ForemanState::Active | ForemanState::Completed, true) => {
                return Err(SnapshotError::UnexpectedBlocker);
            }
            _ => {}
        }
        Ok(Self {
            worker,
            thread,
            task,
            branch,
            worktree,
            head,
            state,
            blocker,
            heartbeat,
            evidence,
            generation,
        })
    }

    #[must_use]
    pub const fn schema(&self) -> &'static str {
        SNAPSHOT_SCHEMA
    }

    #[must_use]
    pub fn worker(&self) -> &str {
        &self.worker
    }

    #[must_use]
    pub fn thread(&self) -> &str {
        &self.thread
    }

    #[must_use]
    pub fn branch(&self) -> &str {
        &self.branch
    }

    #[must_use]
    pub fn head(&self) -> &str {
        &self.head
    }

    #[must_use]
    pub const fn state(&self) -> ForemanState {
        self.state
    }

    #[must_use]
    pub fn blocker(&self) -> Option<&str> {
        self.blocker.as_deref()
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

/// One reconstructed blocked record. Blocked coordination never permits archive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockedWorker {
    snapshot: ForemanSnapshot,
}

impl BlockedWorker {
    #[must_use]
    pub const fn archive_ready(&self) -> bool {
        false
    }
}

/// Fresh-reader projection over verified ordered snapshot events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForemanProjection {
    active: Vec<ForemanSnapshot>,
    blocked: Vec<BlockedWorker>,
    next_action: String,
}

impl ForemanProjection {
    #[must_use]
    pub fn active(&self) -> &[ForemanSnapshot] {
        &self.active
    }

    #[must_use]
    pub fn blocked(&self) -> &[BlockedWorker] {
        &self.blocked
    }

    #[must_use]
    pub fn next_action(&self) -> &str {
        &self.next_action
    }
}

/// Reconstructs the current worker projection from append order without I/O.
///
/// # Errors
///
/// Rejects duplicate worker ownership and non-monotonic generations.
pub fn reconstruct(
    snapshots: impl IntoIterator<Item = ForemanSnapshot>,
) -> Result<ForemanProjection, SnapshotError> {
    let mut by_worker = BTreeMap::<String, ForemanSnapshot>::new();
    for snapshot in snapshots {
        if let Some(previous) = by_worker.get(snapshot.worker()) {
            if previous.thread() != snapshot.thread() {
                return Err(SnapshotError::DuplicateWorkerIdentity);
            }
            if snapshot.generation() <= previous.generation() {
                return Err(SnapshotError::GenerationRollback);
            }
        }
        by_worker.insert(snapshot.worker().to_owned(), snapshot);
    }
    let mut active = Vec::new();
    let mut blocked = Vec::new();
    for snapshot in by_worker.into_values() {
        match snapshot.state() {
            ForemanState::Active => active.push(snapshot),
            ForemanState::Blocked => blocked.push(BlockedWorker { snapshot }),
            ForemanState::Completed => {}
        }
    }
    let next_action = if let Some(blocked_worker) = blocked.first() {
        format!(
            "unblock {}: {}",
            blocked_worker.snapshot.worker(),
            blocked_worker.snapshot.blocker().unwrap_or_default(),
        )
    } else if let Some(active_worker) = active.first() {
        format!("await {}", active_worker.worker())
    } else {
        "no active worker".to_owned()
    };
    Ok(ForemanProjection {
        active,
        blocked,
        next_action,
    })
}

/// Read-only dashboard metadata; it is never a durable authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DashboardIndex {
    generated_at: String,
    branch: String,
    head: String,
    outcome: String,
}

impl DashboardIndex {
    /// # Errors
    ///
    /// Rejects malformed bounded dashboard index values.
    pub fn new(
        generated_at: impl Into<String>,
        branch: impl Into<String>,
        head: impl Into<String>,
        outcome: impl Into<String>,
    ) -> Result<Self, SnapshotError> {
        let generated_at = bounded_reference(generated_at.into())?;
        let branch = bounded_reference(branch.into())?;
        let head = head.into();
        if !is_hex(&head, 40) {
            return Err(SnapshotError::MalformedReference);
        }
        let outcome = bounded_reference(outcome.into())?;
        Ok(Self {
            generated_at,
            branch,
            head,
            outcome,
        })
    }
}

/// Independently collected current worktree facts, injected by a later adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveWorktree {
    worker: String,
    branch: String,
    head: String,
    heartbeat_fresh: bool,
}

impl LiveWorktree {
    /// # Errors
    ///
    /// Rejects malformed bounded live worktree values.
    pub fn new(
        worker: impl Into<String>,
        branch: impl Into<String>,
        head: impl Into<String>,
        heartbeat_fresh: bool,
    ) -> Result<Self, SnapshotError> {
        let worker = bounded_reference(worker.into())?;
        let branch = bounded_reference(branch.into())?;
        let head = head.into();
        if !is_hex(&head, 40) {
            return Err(SnapshotError::MalformedReference);
        }
        Ok(Self {
            worker,
            branch,
            head,
            heartbeat_fresh,
        })
    }
}

/// Fail-closed watchdog results.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WatchdogFinding {
    AllWorkersMissedHeartbeat,
    OldHead { worker: String },
    DashboardDrift,
}

/// Compares untrusted dashboard metadata with injected live observations.
///
/// # Errors
///
/// Rejects a snapshot with no exact independently supplied live worker.
pub fn watchdog(
    snapshots: &[ForemanSnapshot],
    dashboard: &DashboardIndex,
    live: &[LiveWorktree],
) -> Result<Vec<WatchdogFinding>, SnapshotError> {
    let mut findings = Vec::new();
    if !live.is_empty() && live.iter().all(|item| !item.heartbeat_fresh) {
        findings.push(WatchdogFinding::AllWorkersMissedHeartbeat);
    }
    for snapshot in snapshots {
        let item = live
            .iter()
            .find(|candidate| candidate.worker == snapshot.worker());
        let Some(item) = item else {
            return Err(SnapshotError::DuplicateWorkerIdentity);
        };
        if item.branch != snapshot.branch() || item.head != snapshot.head() {
            findings.push(WatchdogFinding::OldHead {
                worker: snapshot.worker().to_owned(),
            });
        }
        if (dashboard.branch != item.branch
            || dashboard.head != item.head
            || dashboard.outcome != snapshot.state().as_str())
            && !findings.contains(&WatchdogFinding::DashboardDrift)
        {
            findings.push(WatchdogFinding::DashboardDrift);
        }
    }
    Ok(findings)
}

impl ForemanState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Blocked => "BLOCKED",
            Self::Completed => "COMPLETED",
        }
    }
}

fn bounded_reference(value: String) -> Result<String, SnapshotError> {
    if value.is_empty()
        || value.len() > MAX_REFERENCE_BYTES
        || !value.is_ascii()
        || value.contains(char::is_whitespace)
        || looks_secret_like(&value)
    {
        return Err(if looks_secret_like(&value) {
            SnapshotError::ForbiddenContent
        } else {
            SnapshotError::MalformedReference
        });
    }
    Ok(value)
}

fn digest_pointer(value: String, prefix: &str) -> Result<String, SnapshotError> {
    let expected_prefix = format!("{prefix}:sha256:");
    if !value.starts_with(&expected_prefix) || !is_hex(&value[expected_prefix.len()..], 64) {
        return Err(if looks_secret_like(&value) {
            SnapshotError::ForbiddenContent
        } else {
            SnapshotError::MalformedReference
        });
    }
    Ok(value)
}

fn is_hex(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn looks_secret_like(value: &str) -> bool {
    let lowercase = value.to_ascii_lowercase();
    lowercase.starts_with("sk-")
        || lowercase.starts_with("bearer ")
        || lowercase.contains("password")
        || lowercase.contains("full chat")
        || lowercase.contains("begin private")
}
