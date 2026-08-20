//! Provider-neutral, I/O-free worker and work-session observation contracts.
//!
//! These types describe evidence. They do not observe a process, transition a
//! task, acquire a lease, persist state, or control a provider.

use std::error::Error;
use std::fmt;

use crate::{
    AttemptId, ContentDigest, GatewayTaskProjection, ProjectId, SubjectBinding, TaskId,
    WriterLeaseAuthorityHead,
};

/// Initial provider-neutral worker observation contract version.
pub const WORKER_OBSERVATION_CONTRACT_VERSION: u16 = 1;
/// Maximum byte length of provider, worker, session, and activity identifiers.
pub const WORKER_OBSERVATION_IDENTIFIER_MAX_BYTES: usize = 128;
/// Maximum byte length of an opaque read-only pagination cursor.
pub const WORKER_OBSERVATION_CURSOR_MAX_BYTES: usize = 512;
/// Maximum page size for read-only worker/session list queries.
pub const WORKER_OBSERVATION_PAGE_MAX_ITEMS: u16 = 100;

const OBSERVATION_TIME_MAX_BYTES: usize = 64;
const MAX_POSITIVE_SIGNED_BIGINT: u64 = i64::MAX as u64;

/// Failure to construct a valid provider-neutral observation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerObservationContractError {
    /// One bounded scalar is empty, malformed, oversized, or a zero digest.
    InvalidValue {
        /// Stable field identifier for diagnostics and tests.
        field: &'static str,
    },
    /// Separately owned observation dimensions contradict their declared scope.
    InconsistentObservation {
        /// Stable field identifier for diagnostics and tests.
        field: &'static str,
    },
    /// Two immutable identities or evidence bindings disagree.
    CrossBinding {
        /// Stable binding identifier for diagnostics and tests.
        field: &'static str,
    },
    /// The worker observation contract version is unsupported.
    UnsupportedVersion {
        /// Version supplied by the caller.
        found: u16,
    },
}

impl fmt::Display for WorkerObservationContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidValue { field } => write!(formatter, "invalid worker observation {field}"),
            Self::InconsistentObservation { field } => {
                write!(formatter, "inconsistent worker observation {field}")
            }
            Self::CrossBinding { field } => {
                write!(formatter, "worker observation cross-binding: {field}")
            }
            Self::UnsupportedVersion { found } => write!(
                formatter,
                "unsupported worker observation contract version {found}"
            ),
        }
    }
}

impl Error for WorkerObservationContractError {}

macro_rules! observation_identifier {
    ($name:ident, $field:literal, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            /// Validates and owns the bounded identifier.
            ///
            /// # Errors
            ///
            /// Rejects empty, oversized, path-like, whitespace-bearing, or
            /// non-ASCII values.
            pub fn new(value: impl Into<String>) -> Result<Self, WorkerObservationContractError> {
                let value = value.into();
                require_identifier(&value, $field, WORKER_OBSERVATION_IDENTIFIER_MAX_BYTES)?;
                Ok(Self(value))
            }

            /// Returns the exact bounded identifier.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

observation_identifier!(
    WorkerProviderId,
    "worker_provider_id",
    "Stable provider identity such as Codex, PowerShell, or a verification runner."
);
observation_identifier!(
    WorkerInstanceId,
    "worker_instance_id",
    "Stable identity of one worker/tool instance, independent of a process ID."
);
observation_identifier!(
    WorkSessionId,
    "work_session_id",
    "Stable identity of one bounded work session."
);
observation_identifier!(
    ActivityEventId,
    "activity_event_id",
    "Stable identity of one immutable activity event."
);

/// Opaque read-only pagination cursor; never a path or command fragment.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ObservationCursor(String);

impl ObservationCursor {
    /// Validates a bounded URL-safe opaque cursor.
    ///
    /// # Errors
    ///
    /// Rejects empty, oversized, path-like, whitespace-bearing, or non-ASCII
    /// values.
    pub fn new(value: impl Into<String>) -> Result<Self, WorkerObservationContractError> {
        let value = value.into();
        require_identifier(
            &value,
            "observation_cursor",
            WORKER_OBSERVATION_CURSOR_MAX_BYTES,
        )?;
        Ok(Self(value))
    }

    /// Returns the opaque cursor.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Bounded list page size.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ObservationPageSize(u16);

impl ObservationPageSize {
    /// Validates a non-zero page size no larger than 100.
    ///
    /// # Errors
    ///
    /// Rejects zero or values above
    /// [`WORKER_OBSERVATION_PAGE_MAX_ITEMS`].
    pub const fn new(value: u16) -> Result<Self, WorkerObservationContractError> {
        if value == 0 || value > WORKER_OBSERVATION_PAGE_MAX_ITEMS {
            Err(WorkerObservationContractError::InvalidValue {
                field: "observation_page_size",
            })
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the validated page size.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Provider capability family without binding the platform to a product name.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum WorkerProviderKind {
    /// AI/development agent provider.
    AiAgent,
    /// Shell or terminal provider.
    Terminal,
    /// General tool provider.
    Tool,
    /// Long-running service provider.
    Service,
    /// Test, build, validation, or review provider.
    Verification,
}

impl WorkerProviderKind {
    /// Returns the stable contract value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AiAgent => "AI_AGENT",
            Self::Terminal => "TERMINAL",
            Self::Tool => "TOOL",
            Self::Service => "SERVICE",
            Self::Verification => "VERIFICATION",
        }
    }
}

/// Immutable provider descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerProvider {
    version: u16,
    id: WorkerProviderId,
    kind: WorkerProviderKind,
}

impl WorkerProvider {
    /// Constructs one provider-neutral descriptor.
    ///
    /// # Errors
    ///
    /// Rejects an unsupported contract version.
    pub fn new(
        version: u16,
        id: WorkerProviderId,
        kind: WorkerProviderKind,
    ) -> Result<Self, WorkerObservationContractError> {
        require_version(version)?;
        Ok(Self { version, id, kind })
    }

    /// Returns the worker observation contract version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Returns the stable provider identity.
    #[must_use]
    pub const fn id(&self) -> &WorkerProviderId {
        &self.id
    }

    /// Returns the provider capability family.
    #[must_use]
    pub const fn kind(&self) -> WorkerProviderKind {
        self.kind
    }
}

/// Ownership of a worker instance or its originating work session.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum WorkerOwnership {
    /// Created and governed by LATTICE.
    LatticeManaged,
    /// Managed by a provider with a formal status interface.
    ProviderManaged,
    /// Created by the user and only observed within explicit interfaces.
    UserManaged,
    /// Only process presence was discovered.
    DiscoveredOnly,
    /// Origin or owner is not known.
    Unknown,
}

impl WorkerOwnership {
    /// Returns the stable contract value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LatticeManaged => "LATTICE_MANAGED",
            Self::ProviderManaged => "PROVIDER_MANAGED",
            Self::UserManaged => "USER_MANAGED",
            Self::DiscoveredOnly => "DISCOVERED_ONLY",
            Self::Unknown => "UNKNOWN",
        }
    }
}

/// Immutable worker instance descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerInstance {
    version: u16,
    id: WorkerInstanceId,
    provider_id: WorkerProviderId,
    ownership: WorkerOwnership,
}

impl WorkerInstance {
    /// Constructs one worker instance independent of any process locator.
    ///
    /// # Errors
    ///
    /// Rejects an unsupported contract version.
    pub fn new(
        version: u16,
        id: WorkerInstanceId,
        provider_id: WorkerProviderId,
        ownership: WorkerOwnership,
    ) -> Result<Self, WorkerObservationContractError> {
        require_version(version)?;
        Ok(Self {
            version,
            id,
            provider_id,
            ownership,
        })
    }

    /// Returns the worker observation contract version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Returns the stable worker instance identity.
    #[must_use]
    pub const fn id(&self) -> &WorkerInstanceId {
        &self.id
    }

    /// Returns the provider identity that owns this instance shape.
    #[must_use]
    pub const fn provider_id(&self) -> &WorkerProviderId {
        &self.provider_id
    }

    /// Returns the worker ownership classification.
    #[must_use]
    pub const fn ownership(&self) -> WorkerOwnership {
        self.ownership
    }
}

/// Degree to which the session can be safely observed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ObservationLevel {
    /// LATTICE events and controlled process supervision are available.
    LatticeManaged,
    /// A provider's formal API or event stream is available.
    FormalProviderInterface,
    /// Only process existence and lifecycle can be observed.
    ProcessPresenceOnly,
    /// Policy or platform constraints make the session unobservable.
    Unobservable,
}

impl ObservationLevel {
    /// Returns the stable contract value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LatticeManaged => "LATTICE_MANAGED",
            Self::FormalProviderInterface => "FORMAL_PROVIDER_INTERFACE",
            Self::ProcessPresenceOnly => "PROCESS_PRESENCE_ONLY",
            Self::Unobservable => "UNOBSERVABLE",
        }
    }
}

/// Source of one safe structured observation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ObservationSource {
    /// A LATTICE-owned immutable activity event.
    LatticeActivityEvent,
    /// LATTICE's bounded managed-process supervisor.
    ManagedProcessSupervisor,
    /// A provider's formal query API.
    ProviderApi,
    /// A provider's formal structured event stream.
    ProviderEventStream,
    /// Low-confidence process-presence discovery.
    ProcessDiscovery,
    /// Explicit policy/platform declaration that no observation is available.
    DeclaredUnobservable,
}

impl ObservationSource {
    /// Returns the stable contract value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LatticeActivityEvent => "LATTICE_ACTIVITY_EVENT",
            Self::ManagedProcessSupervisor => "MANAGED_PROCESS_SUPERVISOR",
            Self::ProviderApi => "PROVIDER_API",
            Self::ProviderEventStream => "PROVIDER_EVENT_STREAM",
            Self::ProcessDiscovery => "PROCESS_DISCOVERY",
            Self::DeclaredUnobservable => "DECLARED_UNOBSERVABLE",
        }
    }
}

/// Freshness of the observation relative to an owner-supplied policy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Freshness {
    /// Observation remains within its freshness window.
    Current,
    /// Observation exceeded its freshness window.
    Stale,
    /// Freshness cannot be established.
    Unknown,
}

impl Freshness {
    /// Returns the stable contract value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Current => "CURRENT",
            Self::Stale => "STALE",
            Self::Unknown => "UNKNOWN",
        }
    }
}

/// Confidence class for one observation, independent of source and freshness.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ObservationConfidence {
    /// LATTICE validated structured evidence from an owned event or supervisor.
    VerifiedStructured,
    /// Validated structured evidence reported by a formal provider interface.
    ProviderReported,
    /// Only process-presence or process-lifecycle evidence is available.
    PresenceOnly,
    /// No confidence can be established.
    Unknown,
}

impl ObservationConfidence {
    /// Returns the stable contract value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VerifiedStructured => "VERIFIED_STRUCTURED",
            Self::ProviderReported => "PROVIDER_REPORTED",
            Self::PresenceOnly => "PRESENCE_ONLY",
            Self::Unknown => "UNKNOWN",
        }
    }
}

/// Current process locator state. It is never task or session state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ProcessState {
    /// Process is being started but has not reached running evidence.
    Starting,
    /// Process existence is currently proven.
    Running,
    /// A terminal process exit is proven.
    Exited,
    /// The previously observed process can no longer be reconciled.
    Lost,
    /// No process is expected for this provider/session.
    NotApplicable,
    /// Process lifecycle cannot be determined.
    Unknown,
}

impl ProcessState {
    /// Returns the stable contract value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "STARTING",
            Self::Running => "RUNNING",
            Self::Exited => "EXITED",
            Self::Lost => "LOST",
            Self::NotApplicable => "NOT_APPLICABLE",
            Self::Unknown => "UNKNOWN",
        }
    }
}

/// Worker-session lifecycle state, independent of process and task state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum WorkSessionState {
    /// Session initialization is in progress.
    Starting,
    /// Session is actively working.
    Running,
    /// Session exists but has no proven current action.
    Idle,
    /// Provider formally reports waiting for user input.
    WaitingForInput,
    /// Provider formally reports waiting for an external condition.
    WaitingForExternal,
    /// Session is running a fixed test or verification action.
    Verifying,
    /// Session is blocked and needs coordination.
    Blocked,
    /// Provider emitted an unambiguous terminal success.
    Completed,
    /// Provider emitted an unambiguous terminal failure.
    Failed,
    /// Session was interrupted without being promoted to task cancellation.
    Interrupted,
    /// Provider session exited without implying task completion.
    Exited,
    /// Last known provider/session state is stale.
    Stale,
    /// No session state can be proven.
    Unknown,
    /// The session cannot be observed under the current interface/policy.
    Unobservable,
}

impl WorkSessionState {
    /// Returns the stable contract value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "STARTING",
            Self::Running => "RUNNING",
            Self::Idle => "IDLE",
            Self::WaitingForInput => "WAITING_FOR_INPUT",
            Self::WaitingForExternal => "WAITING_FOR_EXTERNAL",
            Self::Verifying => "VERIFYING",
            Self::Blocked => "BLOCKED",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
            Self::Interrupted => "INTERRUPTED",
            Self::Exited => "EXITED",
            Self::Stale => "STALE",
            Self::Unknown => "UNKNOWN",
            Self::Unobservable => "UNOBSERVABLE",
        }
    }
}

/// Process runtime/environment family without a command line or raw path.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ProcessEnvironment {
    /// Generic Windows process.
    Windows,
    /// PowerShell host.
    PowerShell,
    /// Windows Command Prompt host.
    WindowsCommandPrompt,
    /// WSL-hosted process.
    Wsl,
    /// Generic Linux process.
    Linux,
    /// Bash-hosted process.
    Bash,
    /// Provider process that does not fit the terminal families.
    ProviderRuntime,
}

impl ProcessEnvironment {
    /// Returns the stable contract value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "WINDOWS",
            Self::PowerShell => "POWERSHELL",
            Self::WindowsCommandPrompt => "WINDOWS_COMMAND_PROMPT",
            Self::Wsl => "WSL",
            Self::Linux => "LINUX",
            Self::Bash => "BASH",
            Self::ProviderRuntime => "PROVIDER_RUNTIME",
        }
    }
}

/// Current process ID locator. PID is not a permanent worker/session identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProcessId(u64);

impl ProcessId {
    /// Constructs a positive `PostgreSQL` signed-BIGINT-safe PID locator.
    ///
    /// # Errors
    ///
    /// Rejects zero and values above signed `BIGINT` maximum.
    pub const fn new(value: u64) -> Result<Self, WorkerObservationContractError> {
        if value == 0 || value > MAX_POSITIVE_SIGNED_BIGINT {
            Err(WorkerObservationContractError::InvalidValue {
                field: "process_id",
            })
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the process locator value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Safe process lifecycle binding without command, environment, or raw output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessBinding {
    environment: ProcessEnvironment,
    process_id: ProcessId,
    process_start_identity: Option<ContentDigest>,
    parent_process_id: Option<ProcessId>,
    state: ProcessState,
    freshness: Freshness,
    source: ObservationSource,
    confidence: ObservationConfidence,
    observed_at: String,
    evidence_digest: ContentDigest,
}

impl ProcessBinding {
    /// Constructs one current process locator and lifecycle observation.
    ///
    /// # Errors
    ///
    /// Rejects a zero process-start/evidence digest, self-parenting process
    /// locator, malformed observation time, or unobservable source.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        environment: ProcessEnvironment,
        process_id: ProcessId,
        process_start_identity: Option<ContentDigest>,
        parent_process_id: Option<ProcessId>,
        state: ProcessState,
        freshness: Freshness,
        source: ObservationSource,
        confidence: ObservationConfidence,
        observed_at: impl Into<String>,
        evidence_digest: ContentDigest,
    ) -> Result<Self, WorkerObservationContractError> {
        if process_start_identity.as_ref().is_some_and(is_zero_digest) {
            return Err(WorkerObservationContractError::InvalidValue {
                field: "process_start_identity",
            });
        }
        if parent_process_id == Some(process_id) {
            return Err(WorkerObservationContractError::InvalidValue {
                field: "parent_process_id",
            });
        }
        if source == ObservationSource::DeclaredUnobservable {
            return Err(WorkerObservationContractError::InconsistentObservation {
                field: "process_observation_source",
            });
        }
        validate_source_confidence(source, confidence)?;
        let observed_at = observed_at.into();
        require_observation_time(&observed_at)?;
        require_digest(&evidence_digest, "process_evidence_digest")?;
        Ok(Self {
            environment,
            process_id,
            process_start_identity,
            parent_process_id,
            state,
            freshness,
            source,
            confidence,
            observed_at,
            evidence_digest,
        })
    }

    /// Returns the process environment family.
    #[must_use]
    pub const fn environment(&self) -> ProcessEnvironment {
        self.environment
    }

    /// Returns the non-durable process locator.
    #[must_use]
    pub const fn process_id(&self) -> ProcessId {
        self.process_id
    }

    /// Returns optional process-start evidence used to detect PID reuse.
    #[must_use]
    pub const fn process_start_identity(&self) -> Option<&ContentDigest> {
        self.process_start_identity.as_ref()
    }

    /// Returns an optional parent process locator.
    #[must_use]
    pub const fn parent_process_id(&self) -> Option<ProcessId> {
        self.parent_process_id
    }

    /// Returns process lifecycle state only.
    #[must_use]
    pub const fn state(&self) -> ProcessState {
        self.state
    }

    /// Returns process-observation freshness independently of session state.
    #[must_use]
    pub const fn freshness(&self) -> Freshness {
        self.freshness
    }

    /// Returns the process lifecycle evidence source.
    #[must_use]
    pub const fn source(&self) -> ObservationSource {
        self.source
    }

    /// Returns process-evidence confidence independently of source/freshness.
    #[must_use]
    pub const fn confidence(&self) -> ObservationConfidence {
        self.confidence
    }

    /// Returns the owner-supplied canonical process observation time text.
    #[must_use]
    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }

    /// Returns the process lifecycle evidence digest.
    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }
}

/// Exact LATTICE task binding, distinct from work-session state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskBinding {
    binding: SubjectBinding,
    attempt_id: Option<AttemptId>,
}

impl TaskBinding {
    /// Constructs one exact Task Domain/Task Ledger binding.
    ///
    /// # Errors
    ///
    /// Rejects oversized or path-like reused identifiers.
    pub fn new(
        binding: SubjectBinding,
        attempt_id: Option<AttemptId>,
    ) -> Result<Self, WorkerObservationContractError> {
        require_identifier(
            binding.project_snapshot_id().as_str(),
            "project_snapshot_id",
            WORKER_OBSERVATION_IDENTIFIER_MAX_BYTES,
        )?;
        require_identifier(
            binding.task_id().as_str(),
            "task_id",
            WORKER_OBSERVATION_IDENTIFIER_MAX_BYTES,
        )?;
        if let Some(attempt_id) = attempt_id.as_ref() {
            require_identifier(
                attempt_id.as_str(),
                "attempt_id",
                WORKER_OBSERVATION_IDENTIFIER_MAX_BYTES,
            )?;
        }
        Ok(Self {
            binding,
            attempt_id,
        })
    }

    /// Returns the exact project/task/spec binding.
    #[must_use]
    pub const fn binding(&self) -> &SubjectBinding {
        &self.binding
    }

    /// Returns the exact task attempt when one is formally bound.
    #[must_use]
    pub const fn attempt_id(&self) -> Option<&AttemptId> {
        self.attempt_id.as_ref()
    }
}

/// Read-only authority dimension, never a lease claim or currentness proof.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthorityObservation {
    /// This provider/session does not use product writer authority.
    NotApplicable,
    /// Authority was not observed through the current interface.
    NotObserved,
    /// Structural Writer Lease owner head; currentness still requires an
    /// independent owner query and this value grants no mutation authority.
    WriterLease(Box<WriterLeaseAuthorityHead>),
}

/// Safe immutable activity classification.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ActivityKind {
    /// Session start evidence.
    Started,
    /// Bounded liveness/heartbeat evidence.
    Heartbeat,
    /// Provider reports work progress without raw content.
    Progress,
    /// Provider formally waits for input.
    WaitingForInput,
    /// Provider formally waits for an external condition.
    WaitingForExternal,
    /// A fixed test or verification action began.
    VerificationStarted,
    /// Provider/session reports a blocker.
    Blocked,
    /// Provider emitted terminal success.
    Completed,
    /// Provider emitted terminal failure.
    Failed,
    /// Session was interrupted.
    Interrupted,
    /// A process locator was newly discovered without inferring session state.
    ProcessDiscovered,
    /// A previously discovered process locator is still present.
    ProcessStillPresent,
    /// A process exit was observed without inferring task completion.
    ProcessExited,
    /// A previously observed process can no longer be reconciled.
    ProcessLost,
    /// Formal provider interface disconnected or became stale.
    Disconnected,
}

impl ActivityKind {
    /// Returns the stable contract value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Started => "STARTED",
            Self::Heartbeat => "HEARTBEAT",
            Self::Progress => "PROGRESS",
            Self::WaitingForInput => "WAITING_FOR_INPUT",
            Self::WaitingForExternal => "WAITING_FOR_EXTERNAL",
            Self::VerificationStarted => "VERIFICATION_STARTED",
            Self::Blocked => "BLOCKED",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
            Self::Interrupted => "INTERRUPTED",
            Self::ProcessDiscovered => "PROCESS_DISCOVERED",
            Self::ProcessStillPresent => "PROCESS_STILL_PRESENT",
            Self::ProcessExited => "PROCESS_EXITED",
            Self::ProcessLost => "PROCESS_LOST",
            Self::Disconnected => "DISCONNECTED",
        }
    }
}

/// Immutable activity event with independently optional process/session states.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivityEvent {
    version: u16,
    id: ActivityEventId,
    work_session_id: WorkSessionId,
    sequence: u64,
    kind: ActivityKind,
    source: ObservationSource,
    confidence: ObservationConfidence,
    observed_at: String,
    session_state_after: Option<WorkSessionState>,
    process_state_after: Option<ProcessState>,
    evidence_digest: ContentDigest,
}

impl ActivityEvent {
    /// Constructs one bounded content-free activity event.
    ///
    /// # Errors
    ///
    /// Rejects unsupported versions, zero/overflowing sequences, malformed
    /// observation time, zero evidence, or an unobservable source.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        version: u16,
        id: ActivityEventId,
        work_session_id: WorkSessionId,
        sequence: u64,
        kind: ActivityKind,
        source: ObservationSource,
        confidence: ObservationConfidence,
        observed_at: impl Into<String>,
        session_state_after: Option<WorkSessionState>,
        process_state_after: Option<ProcessState>,
        evidence_digest: ContentDigest,
    ) -> Result<Self, WorkerObservationContractError> {
        require_version(version)?;
        if sequence == 0 || sequence > MAX_POSITIVE_SIGNED_BIGINT {
            return Err(WorkerObservationContractError::InvalidValue {
                field: "activity_sequence",
            });
        }
        if source == ObservationSource::DeclaredUnobservable {
            return Err(WorkerObservationContractError::InconsistentObservation {
                field: "activity_source",
            });
        }
        validate_source_confidence(source, confidence)?;
        if source == ObservationSource::ProcessDiscovery {
            let lifecycle_matches = matches!(
                (kind, process_state_after),
                (
                    ActivityKind::ProcessDiscovered | ActivityKind::ProcessStillPresent,
                    Some(ProcessState::Running)
                ) | (ActivityKind::ProcessExited, Some(ProcessState::Exited))
                    | (ActivityKind::ProcessLost, Some(ProcessState::Lost))
            );
            if !lifecycle_matches || session_state_after.is_some() {
                return Err(WorkerObservationContractError::InconsistentObservation {
                    field: "process_discovery_activity",
                });
            }
        }
        let observed_at = observed_at.into();
        require_observation_time(&observed_at)?;
        require_digest(&evidence_digest, "activity_evidence_digest")?;
        Ok(Self {
            version,
            id,
            work_session_id,
            sequence,
            kind,
            source,
            confidence,
            observed_at,
            session_state_after,
            process_state_after,
            evidence_digest,
        })
    }

    /// Returns the worker observation contract version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Returns the event identity.
    #[must_use]
    pub const fn id(&self) -> &ActivityEventId {
        &self.id
    }

    /// Returns the exact work-session identity.
    #[must_use]
    pub const fn work_session_id(&self) -> &WorkSessionId {
        &self.work_session_id
    }

    /// Returns the positive session-local event sequence.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the safe activity classification.
    #[must_use]
    pub const fn kind(&self) -> ActivityKind {
        self.kind
    }

    /// Returns the structured evidence source.
    #[must_use]
    pub const fn source(&self) -> ObservationSource {
        self.source
    }

    /// Returns activity-evidence confidence independently of source/freshness.
    #[must_use]
    pub const fn confidence(&self) -> ObservationConfidence {
        self.confidence
    }

    /// Returns bounded owner-supplied observation time text.
    #[must_use]
    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }

    /// Returns independently observed worker-session state after the event.
    #[must_use]
    pub const fn session_state_after(&self) -> Option<WorkSessionState> {
        self.session_state_after
    }

    /// Returns independently observed process state after the event.
    #[must_use]
    pub const fn process_state_after(&self) -> Option<ProcessState> {
        self.process_state_after
    }

    /// Returns the structured evidence digest.
    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }
}

/// One provider-neutral work-session observation with separated state axes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkSessionObservation {
    version: u16,
    id: WorkSessionId,
    worker_instance_id: WorkerInstanceId,
    level: ObservationLevel,
    state: WorkSessionState,
    freshness: Freshness,
    source: ObservationSource,
    confidence: ObservationConfidence,
    observed_at: String,
    process: Option<ProcessBinding>,
    task_binding: Option<TaskBinding>,
    task_projection: Option<GatewayTaskProjection>,
    authority: AuthorityObservation,
    last_activity_id: Option<ActivityEventId>,
    evidence_digest: ContentDigest,
}

impl WorkSessionObservation {
    /// Constructs one read-only observation without deriving one state axis
    /// from another.
    ///
    /// # Errors
    ///
    /// Rejects dishonest visibility/source combinations, task/lease
    /// substitution, process-only progress claims, malformed time, or zero
    /// evidence.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        version: u16,
        id: WorkSessionId,
        worker_instance_id: WorkerInstanceId,
        level: ObservationLevel,
        state: WorkSessionState,
        freshness: Freshness,
        source: ObservationSource,
        confidence: ObservationConfidence,
        observed_at: impl Into<String>,
        process: Option<ProcessBinding>,
        task_binding: Option<TaskBinding>,
        task_projection: Option<GatewayTaskProjection>,
        authority: AuthorityObservation,
        last_activity_id: Option<ActivityEventId>,
        evidence_digest: ContentDigest,
    ) -> Result<Self, WorkerObservationContractError> {
        require_version(version)?;
        let observed_at = observed_at.into();
        require_observation_time(&observed_at)?;
        require_digest(&evidence_digest, "session_evidence_digest")?;
        validate_level_source(level, source)?;
        validate_source_confidence(source, confidence)?;
        if source == ObservationSource::ManagedProcessSupervisor
            && (state != WorkSessionState::Unknown || last_activity_id.is_some())
        {
            return Err(WorkerObservationContractError::InconsistentObservation {
                field: "process_source_session_state",
            });
        }
        validate_visibility_content(
            level,
            state,
            freshness,
            process.as_ref(),
            task_binding.as_ref(),
            task_projection.as_ref(),
            &authority,
            last_activity_id.as_ref(),
        )?;
        if let Some(projection) = task_projection.as_ref() {
            let Some(task) = task_binding.as_ref() else {
                return Err(WorkerObservationContractError::InconsistentObservation {
                    field: "task_projection_without_binding",
                });
            };
            if projection.binding() != task.binding() {
                return Err(WorkerObservationContractError::CrossBinding {
                    field: "task_projection",
                });
            }
        }
        if let AuthorityObservation::WriterLease(head) = &authority {
            if process.is_none() {
                return Err(WorkerObservationContractError::InconsistentObservation {
                    field: "writer_lease_without_process_binding",
                });
            }
            let Some(task) = task_binding.as_ref() else {
                return Err(WorkerObservationContractError::InconsistentObservation {
                    field: "writer_lease_without_task_binding",
                });
            };
            validate_writer_binding(task, process.as_ref(), head)?;
        }
        Ok(Self {
            version,
            id,
            worker_instance_id,
            level,
            state,
            freshness,
            source,
            confidence,
            observed_at,
            process,
            task_binding,
            task_projection,
            authority,
            last_activity_id,
            evidence_digest,
        })
    }

    /// Returns the worker observation contract version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Returns the work-session identity.
    #[must_use]
    pub const fn id(&self) -> &WorkSessionId {
        &self.id
    }

    /// Returns the owning worker instance identity.
    #[must_use]
    pub const fn worker_instance_id(&self) -> &WorkerInstanceId {
        &self.worker_instance_id
    }

    /// Returns the observation level.
    #[must_use]
    pub const fn level(&self) -> ObservationLevel {
        self.level
    }

    /// Returns worker-session state only.
    #[must_use]
    pub const fn state(&self) -> WorkSessionState {
        self.state
    }

    /// Returns owner-declared freshness independently of the last known state.
    #[must_use]
    pub const fn freshness(&self) -> Freshness {
        self.freshness
    }

    /// Returns the structured observation source.
    #[must_use]
    pub const fn source(&self) -> ObservationSource {
        self.source
    }

    /// Returns session-evidence confidence independently of source/freshness.
    #[must_use]
    pub const fn confidence(&self) -> ObservationConfidence {
        self.confidence
    }

    /// Returns bounded owner-supplied observation time text.
    #[must_use]
    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }

    /// Returns process lifecycle separately from session/task state.
    #[must_use]
    pub const fn process(&self) -> Option<&ProcessBinding> {
        self.process.as_ref()
    }

    /// Returns the exact LATTICE task binding when formally observed.
    #[must_use]
    pub const fn task_binding(&self) -> Option<&TaskBinding> {
        self.task_binding.as_ref()
    }

    /// Returns the separately owned Task Ledger projection when observed.
    #[must_use]
    pub const fn task_projection(&self) -> Option<&GatewayTaskProjection> {
        self.task_projection.as_ref()
    }

    /// Returns read-only authority observation separately from task/session.
    #[must_use]
    pub const fn authority(&self) -> &AuthorityObservation {
        &self.authority
    }

    /// Returns the last immutable activity event identity when known.
    #[must_use]
    pub const fn last_activity_id(&self) -> Option<&ActivityEventId> {
        self.last_activity_id.as_ref()
    }

    /// Returns the complete session observation evidence digest.
    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }
}

/// Exact provider/instance/session aggregate for read-only status projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerObservation {
    provider: WorkerProvider,
    instance: WorkerInstance,
    session: WorkSessionObservation,
}

impl WorkerObservation {
    /// Constructs one exactly cross-bound provider/instance/session view.
    ///
    /// # Errors
    ///
    /// Rejects provider/instance/session substitution or a visibility level
    /// incompatible with instance ownership.
    pub fn new(
        provider: WorkerProvider,
        instance: WorkerInstance,
        session: WorkSessionObservation,
    ) -> Result<Self, WorkerObservationContractError> {
        if provider.id() != instance.provider_id() {
            return Err(WorkerObservationContractError::CrossBinding {
                field: "worker_provider",
            });
        }
        if instance.id() != session.worker_instance_id() {
            return Err(WorkerObservationContractError::CrossBinding {
                field: "worker_instance",
            });
        }
        validate_ownership_level(instance.ownership(), session.level())?;
        if matches!(session.authority(), AuthorityObservation::WriterLease(_))
            && instance.ownership() != WorkerOwnership::LatticeManaged
        {
            return Err(WorkerObservationContractError::InconsistentObservation {
                field: "writer_lease_ownership",
            });
        }
        Ok(Self {
            provider,
            instance,
            session,
        })
    }

    /// Returns the provider descriptor.
    #[must_use]
    pub const fn provider(&self) -> &WorkerProvider {
        &self.provider
    }

    /// Returns the worker instance descriptor.
    #[must_use]
    pub const fn instance(&self) -> &WorkerInstance {
        &self.instance
    }

    /// Returns the current work-session observation.
    #[must_use]
    pub const fn session(&self) -> &WorkSessionObservation {
        &self.session
    }
}

/// Bounded filters shared by read-only worker/session list queries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationListFilter {
    provider_id: Option<WorkerProviderId>,
    project_id: Option<ProjectId>,
    task_id: Option<TaskId>,
    session_state: Option<WorkSessionState>,
    page_size: ObservationPageSize,
    cursor: Option<ObservationCursor>,
}

impl ObservationListFilter {
    /// Constructs one bounded read-only list filter.
    #[must_use]
    pub const fn new(
        provider_id: Option<WorkerProviderId>,
        project_id: Option<ProjectId>,
        task_id: Option<TaskId>,
        session_state: Option<WorkSessionState>,
        page_size: ObservationPageSize,
        cursor: Option<ObservationCursor>,
    ) -> Self {
        Self {
            provider_id,
            project_id,
            task_id,
            session_state,
            page_size,
            cursor,
        }
    }

    /// Returns an optional provider filter.
    #[must_use]
    pub const fn provider_id(&self) -> Option<&WorkerProviderId> {
        self.provider_id.as_ref()
    }

    /// Returns an optional registered-project filter.
    #[must_use]
    pub const fn project_id(&self) -> Option<&ProjectId> {
        self.project_id.as_ref()
    }

    /// Returns an optional task filter.
    #[must_use]
    pub const fn task_id(&self) -> Option<&TaskId> {
        self.task_id.as_ref()
    }

    /// Returns an optional worker-session state filter.
    #[must_use]
    pub const fn session_state(&self) -> Option<WorkSessionState> {
        self.session_state
    }

    /// Returns the bounded page size.
    #[must_use]
    pub const fn page_size(&self) -> ObservationPageSize {
        self.page_size
    }

    /// Returns an optional opaque cursor.
    #[must_use]
    pub const fn cursor(&self) -> Option<&ObservationCursor> {
        self.cursor.as_ref()
    }
}

/// Closed read-only query surface for future worker/session tools.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservationQuery {
    /// List bounded worker projections.
    WorkerList(ObservationListFilter),
    /// Read one worker instance status.
    WorkerStatus(WorkerInstanceId),
    /// List bounded work-session projections.
    SessionList(ObservationListFilter),
    /// Read one work-session status.
    SessionStatus(WorkSessionId),
}

impl ObservationQuery {
    /// Returns the stable query kind. No control action has a variant.
    #[must_use]
    pub fn kind(self) -> &'static str {
        match self {
            Self::WorkerList(_) => "WORKER_LIST",
            Self::WorkerStatus(_) => "WORKER_STATUS",
            Self::SessionList(_) => "SESSION_LIST",
            Self::SessionStatus(_) => "SESSION_STATUS",
        }
    }
}

fn require_version(version: u16) -> Result<(), WorkerObservationContractError> {
    if version == WORKER_OBSERVATION_CONTRACT_VERSION {
        Ok(())
    } else {
        Err(WorkerObservationContractError::UnsupportedVersion { found: version })
    }
}

fn require_identifier(
    value: &str,
    field: &'static str,
    max_bytes: usize,
) -> Result<(), WorkerObservationContractError> {
    let valid = !value.is_empty()
        && value.len() <= max_bytes
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'));
    if valid {
        Ok(())
    } else {
        Err(WorkerObservationContractError::InvalidValue { field })
    }
}

fn require_observation_time(value: &str) -> Result<(), WorkerObservationContractError> {
    let valid = !value.is_empty()
        && value.len() <= OBSERVATION_TIME_MAX_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b':' | b'.' | b'+' | b'T' | b'Z')
        });
    if valid {
        Ok(())
    } else {
        Err(WorkerObservationContractError::InvalidValue {
            field: "observed_at",
        })
    }
}

fn require_digest(
    digest: &ContentDigest,
    field: &'static str,
) -> Result<(), WorkerObservationContractError> {
    if is_zero_digest(digest) {
        Err(WorkerObservationContractError::InvalidValue { field })
    } else {
        Ok(())
    }
}

fn is_zero_digest(digest: &ContentDigest) -> bool {
    digest.as_str().bytes().all(|byte| byte == b'0')
}

fn validate_level_source(
    level: ObservationLevel,
    source: ObservationSource,
) -> Result<(), WorkerObservationContractError> {
    let valid = match level {
        ObservationLevel::LatticeManaged => matches!(
            source,
            ObservationSource::LatticeActivityEvent | ObservationSource::ManagedProcessSupervisor
        ),
        ObservationLevel::FormalProviderInterface => matches!(
            source,
            ObservationSource::ProviderApi | ObservationSource::ProviderEventStream
        ),
        ObservationLevel::ProcessPresenceOnly => matches!(
            source,
            ObservationSource::ProcessDiscovery | ObservationSource::ManagedProcessSupervisor
        ),
        ObservationLevel::Unobservable => source == ObservationSource::DeclaredUnobservable,
    };
    if valid {
        Ok(())
    } else {
        Err(WorkerObservationContractError::InconsistentObservation {
            field: "observation_level_source",
        })
    }
}

fn validate_source_confidence(
    source: ObservationSource,
    confidence: ObservationConfidence,
) -> Result<(), WorkerObservationContractError> {
    let valid = match source {
        ObservationSource::LatticeActivityEvent | ObservationSource::ManagedProcessSupervisor => {
            confidence == ObservationConfidence::VerifiedStructured
        }
        ObservationSource::ProviderApi | ObservationSource::ProviderEventStream => {
            confidence == ObservationConfidence::ProviderReported
        }
        ObservationSource::ProcessDiscovery => confidence == ObservationConfidence::PresenceOnly,
        ObservationSource::DeclaredUnobservable => confidence == ObservationConfidence::Unknown,
    };
    if valid {
        Ok(())
    } else {
        Err(WorkerObservationContractError::InconsistentObservation {
            field: "observation_source_confidence",
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_visibility_content(
    level: ObservationLevel,
    state: WorkSessionState,
    freshness: Freshness,
    process: Option<&ProcessBinding>,
    task_binding: Option<&TaskBinding>,
    task_projection: Option<&GatewayTaskProjection>,
    authority: &AuthorityObservation,
    last_activity_id: Option<&ActivityEventId>,
) -> Result<(), WorkerObservationContractError> {
    match level {
        ObservationLevel::ProcessPresenceOnly => {
            if process.is_none() {
                return Err(WorkerObservationContractError::InconsistentObservation {
                    field: "process_only_process_binding",
                });
            }
            if state != WorkSessionState::Unknown {
                return Err(WorkerObservationContractError::InconsistentObservation {
                    field: "process_only_session_state",
                });
            }
            if task_binding.is_some() {
                return Err(WorkerObservationContractError::InconsistentObservation {
                    field: "process_only_task_binding",
                });
            }
            if task_projection.is_some() {
                return Err(WorkerObservationContractError::InconsistentObservation {
                    field: "process_only_task_projection",
                });
            }
            if !matches!(authority, AuthorityObservation::NotObserved) {
                return Err(WorkerObservationContractError::InconsistentObservation {
                    field: "process_only_authority",
                });
            }
            if last_activity_id.is_some() {
                return Err(WorkerObservationContractError::InconsistentObservation {
                    field: "process_only_activity",
                });
            }
        }
        ObservationLevel::Unobservable => {
            if state != WorkSessionState::Unobservable
                || freshness != Freshness::Unknown
                || process.is_some()
                || task_binding.is_some()
                || task_projection.is_some()
                || !matches!(authority, AuthorityObservation::NotObserved)
                || last_activity_id.is_some()
            {
                return Err(WorkerObservationContractError::InconsistentObservation {
                    field: "unobservable_content",
                });
            }
        }
        ObservationLevel::LatticeManaged | ObservationLevel::FormalProviderInterface => {
            if state == WorkSessionState::Unobservable {
                return Err(WorkerObservationContractError::InconsistentObservation {
                    field: "observable_session_state",
                });
            }
            if matches!(authority, AuthorityObservation::WriterLease(_))
                && level != ObservationLevel::LatticeManaged
            {
                return Err(WorkerObservationContractError::InconsistentObservation {
                    field: "writer_lease_observation_level",
                });
            }
        }
    }
    Ok(())
}

fn validate_writer_binding(
    task: &TaskBinding,
    process: Option<&ProcessBinding>,
    head: &WriterLeaseAuthorityHead,
) -> Result<(), WorkerObservationContractError> {
    let identity = head.identity();
    let binding = task.binding();
    let task_matches = identity.project_id() == binding.project_id()
        && identity.project_snapshot_id() == binding.project_snapshot_id()
        && identity.task_id() == binding.task_id()
        && identity.task_revision() == binding.task_revision()
        && identity.task_spec_digest() == binding.task_spec_digest()
        && task.attempt_id() == Some(identity.attempt_id());
    if !task_matches {
        return Err(WorkerObservationContractError::CrossBinding {
            field: "writer_lease_task",
        });
    }
    if let Some(process) = process {
        let process_matches = process.process_id().get() == identity.holder_process_id().get()
            && process.process_start_identity() == Some(identity.holder_process_start_identity());
        if !process_matches {
            return Err(WorkerObservationContractError::CrossBinding {
                field: "writer_lease_process",
            });
        }
    }
    Ok(())
}

fn validate_ownership_level(
    ownership: WorkerOwnership,
    level: ObservationLevel,
) -> Result<(), WorkerObservationContractError> {
    let valid = match ownership {
        WorkerOwnership::LatticeManaged => true,
        WorkerOwnership::ProviderManaged | WorkerOwnership::UserManaged => {
            !matches!(level, ObservationLevel::LatticeManaged)
        }
        WorkerOwnership::DiscoveredOnly => level == ObservationLevel::ProcessPresenceOnly,
        WorkerOwnership::Unknown => matches!(
            level,
            ObservationLevel::ProcessPresenceOnly | ObservationLevel::Unobservable
        ),
    };
    if valid {
        Ok(())
    } else {
        Err(WorkerObservationContractError::InconsistentObservation {
            field: "ownership_observation_level",
        })
    }
}
