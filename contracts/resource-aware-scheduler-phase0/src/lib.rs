//! Isolated Phase 0 contract fixture for GitHub Issue #7.
//!
//! This crate performs no I/O, scheduling, dispatch, persistence, process
//! control, or workspace mutation. It only characterizes resource evidence and
//! admission decisions while TASK-038 production contracts remain unstable.

#![forbid(unsafe_code)]

/// The authority that produced a resource observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationSource {
    WindowsCim,
    WindowsPerfCounter,
    NvidiaSmi,
    CodexRateLimits,
    TestFixture,
}

/// Stable, non-secret failure classes for a collector boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SampleFailure {
    QueryFailed,
    ParseOrQueryFailed,
    AccessDenied,
    SourceUnavailable,
    Unsupported,
}

/// Freshness and availability at the caller-provided evaluation time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationState {
    Fresh,
    Stale,
    Unknown,
    Failed(SampleFailure),
}

/// Structural faults that make an observation unsafe to consume.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationFault {
    InvalidValidityWindow,
    FutureObservation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ObservationPayload<T> {
    Observed {
        captured_at: u64,
        valid_until: u64,
        value: T,
    },
    Failed {
        captured_at: u64,
        failure: SampleFailure,
    },
    Unknown,
}

/// A typed value with explicit source, freshness, and error semantics.
///
/// Time values are caller-defined monotonic ticks. The contract never reads a
/// clock and never substitutes a missing or failed sample with zero.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Observation<T> {
    source: ObservationSource,
    payload: ObservationPayload<T>,
}

impl<T> Observation<T> {
    /// Creates a successfully collected value with an inclusive validity end.
    pub fn observed(
        source: ObservationSource,
        captured_at: u64,
        valid_until: u64,
        value: T,
    ) -> Result<Self, ObservationFault> {
        if valid_until < captured_at {
            return Err(ObservationFault::InvalidValidityWindow);
        }

        Ok(Self {
            source,
            payload: ObservationPayload::Observed {
                captured_at,
                valid_until,
                value,
            },
        })
    }

    /// Records a classified collection failure without inventing a value.
    #[must_use]
    pub const fn failed(
        source: ObservationSource,
        captured_at: u64,
        failure: SampleFailure,
    ) -> Self {
        Self {
            source,
            payload: ObservationPayload::Failed {
                captured_at,
                failure,
            },
        }
    }

    /// Records that no formal signal is currently available.
    #[must_use]
    pub const fn unknown(source: ObservationSource) -> Self {
        Self {
            source,
            payload: ObservationPayload::Unknown,
        }
    }

    #[must_use]
    pub const fn source(&self) -> ObservationSource {
        self.source
    }

    /// Returns the observed value only when a collector actually supplied one.
    #[must_use]
    pub const fn value(&self) -> Option<&T> {
        match &self.payload {
            ObservationPayload::Observed { value, .. } => Some(value),
            ObservationPayload::Failed { .. } | ObservationPayload::Unknown => None,
        }
    }

    /// Evaluates freshness using a caller-provided clock value.
    pub const fn state_at(&self, now: u64) -> Result<ObservationState, ObservationFault> {
        match &self.payload {
            ObservationPayload::Observed {
                captured_at,
                valid_until,
                ..
            } => {
                if now < *captured_at {
                    Err(ObservationFault::FutureObservation)
                } else if now <= *valid_until {
                    Ok(ObservationState::Fresh)
                } else {
                    Ok(ObservationState::Stale)
                }
            }
            ObservationPayload::Failed {
                captured_at,
                failure,
            } => {
                if now < *captured_at {
                    Err(ObservationFault::FutureObservation)
                } else {
                    Ok(ObservationState::Failed(*failure))
                }
            }
            ObservationPayload::Unknown => Ok(ObservationState::Unknown),
        }
    }
}

/// A percentage represented as thousandths, where 1000 is 100%.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Permille(u16);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeasurementFault {
    PermilleOutOfRange,
}

impl Permille {
    pub const fn new(value: u16) -> Result<Self, MeasurementFault> {
        if value <= 1000 {
            Ok(Self(value))
        } else {
            Err(MeasurementFault::PermilleOutOfRange)
        }
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Coarse resource shape used for admission ordering and policy selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceClass {
    CloudLight,
    CpuHeavy,
    MemoryHeavy,
    GpuHeavy,
    IoHeavy,
    NetworkHeavy,
    Exclusive,
    Mixed,
}

/// Shared authority surfaces that must not be edited concurrently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedResource {
    RootCargoManifest,
    CargoLock,
    RuntimeComposition,
    McpDispatcher,
    PostgresAdapter,
    PostgresMigration,
    Orchestrator,
    TaskDomain,
    WriterLease,
    ProjectRegistry,
    WorkSessionContract,
    PlanningDocuments,
}

/// Symbolic identity only. It cannot access or mutate a filesystem path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeId(String);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityFault {
    Empty,
    InvalidCharacter,
}

impl WorktreeId {
    pub fn new(value: &str) -> Result<Self, IdentityFault> {
        if value.is_empty() {
            return Err(IdentityFault::Empty);
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(IdentityFault::InvalidCharacter);
        }
        Ok(Self(value.to_owned()))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceRequirements {
    pub requires_gpu: bool,
    pub requires_cloud_quota: bool,
}

/// Dependency facts supplied by their owning modules; this crate never infers
/// them from process presence or repository state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Prerequisites {
    pub approved: bool,
    pub dependencies_complete: bool,
    pub task_control_stable: bool,
    pub worker_observation_stable: bool,
    pub project_registered: bool,
    pub outcomes_reconciled: bool,
    pub writer_lease_ready: bool,
}

impl Prerequisites {
    #[must_use]
    pub const fn ready() -> Self {
        Self {
            approved: true,
            dependencies_complete: true,
            task_control_stable: true,
            worker_observation_stable: true,
            project_registered: true,
            outcomes_reconciled: true,
            writer_lease_ready: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskCandidate {
    pub task_key: String,
    pub worktree: WorktreeId,
    pub class: ResourceClass,
    pub requirements: ResourceRequirements,
    pub prerequisites: Prerequisites,
    pub shared_resources: Vec<SharedResource>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveTask {
    pub task_key: String,
    pub worktree: WorktreeId,
    pub shared_resources: Vec<SharedResource>,
}

/// Caller-supplied thresholds. The contract intentionally contains no machine
/// profile defaults and cannot turn a Phase 0 sample into production policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionThresholds {
    pub max_cpu_used: Permille,
    pub min_memory_available_bytes: u64,
    pub max_disk_busy: Permille,
    pub max_disk_queue_milli: u32,
    pub max_disk_latency_micros: u64,
    pub min_gpu_free_bytes: u64,
    pub max_gpu_temperature_deci_celsius: u16,
    pub max_cloud_rate_used: Permille,
    pub min_observation_window_seconds: u32,
    pub min_observation_samples: u16,
}

/// Coverage of the rolling evidence summarized by a host snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvidenceWindow {
    pub duration_seconds: u32,
    pub sample_count: u16,
}

/// Host evidence supplied by collectors outside this pure crate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostSnapshot {
    pub window: EvidenceWindow,
    pub cpu_used: Observation<Permille>,
    pub memory_available_bytes: Observation<u64>,
    /// Maximum across physical disks of `100% - PercentIdleTime`.
    /// Aggregate `_Total PercentDiskTime` is not a valid input.
    pub max_normalized_disk_busy: Observation<Permille>,
    pub max_disk_queue_milli: Observation<u32>,
    pub max_disk_latency_micros: Observation<u64>,
    pub gpu_free_bytes: Observation<u64>,
    pub gpu_temperature_deci_celsius: Observation<u16>,
    pub cloud_rate_used: Observation<Permille>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DependencyCheckpoint {
    Approved,
    DependenciesComplete,
    TaskControlStable,
    WorkerObservationStable,
    ProjectRegistered,
    OutcomesReconciled,
    WriterLeaseReady,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalKind {
    CpuUsed,
    MemoryAvailable,
    MaxNormalizedDiskBusy,
    MaxDiskQueue,
    MaxDiskLatency,
    GpuFreeMemory,
    GpuTemperature,
    CloudRateUsed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThresholdKind {
    CpuUsed,
    MemoryAvailable,
    MaxNormalizedDiskBusy,
    MaxDiskQueue,
    MaxDiskLatency,
    GpuFreeMemory,
    GpuTemperature,
    CloudRateUsed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequiredSignal {
    Gpu,
    CloudQuota,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionReason {
    DependencyCheckpointMissing(DependencyCheckpoint),
    RequiredSignalMissing(RequiredSignal),
    ObservationWindowTooShort,
    ObservationWindowTooSparse,
    TaskAlreadyActive,
    WorktreeConflict,
    SharedResourceConflict(SharedResource),
    SignalSourceMismatch {
        signal: SignalKind,
        actual: ObservationSource,
    },
    SignalNotUsable {
        signal: SignalKind,
        state: ObservationState,
    },
    ObservationFault {
        signal: SignalKind,
        fault: ObservationFault,
    },
    ThresholdExceeded(ThresholdKind),
}

/// The Phase 0 evaluator never grants a batch. A caller may admit one task and
/// must collect a new snapshot before considering another.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionDecision {
    AdmitOne,
    Hold(AdmissionReason),
}

fn missing_prerequisite(value: Prerequisites) -> Option<DependencyCheckpoint> {
    if !value.approved {
        Some(DependencyCheckpoint::Approved)
    } else if !value.dependencies_complete {
        Some(DependencyCheckpoint::DependenciesComplete)
    } else if !value.task_control_stable {
        Some(DependencyCheckpoint::TaskControlStable)
    } else if !value.worker_observation_stable {
        Some(DependencyCheckpoint::WorkerObservationStable)
    } else if !value.project_registered {
        Some(DependencyCheckpoint::ProjectRegistered)
    } else if !value.outcomes_reconciled {
        Some(DependencyCheckpoint::OutcomesReconciled)
    } else if !value.writer_lease_ready {
        Some(DependencyCheckpoint::WriterLeaseReady)
    } else {
        None
    }
}

fn fresh_value<T: Copy>(
    observation: &Observation<T>,
    signal: SignalKind,
    now: u64,
) -> Result<T, AdmissionReason> {
    if !source_is_allowed(signal, observation.source()) {
        return Err(AdmissionReason::SignalSourceMismatch {
            signal,
            actual: observation.source(),
        });
    }

    match observation.state_at(now) {
        Ok(ObservationState::Fresh) => {
            observation
                .value()
                .copied()
                .ok_or(AdmissionReason::SignalNotUsable {
                    signal,
                    state: ObservationState::Unknown,
                })
        }
        Ok(state) => Err(AdmissionReason::SignalNotUsable { signal, state }),
        Err(fault) => Err(AdmissionReason::ObservationFault { signal, fault }),
    }
}

const fn source_is_allowed(signal: SignalKind, source: ObservationSource) -> bool {
    if matches!(source, ObservationSource::TestFixture) {
        return true;
    }

    match signal {
        SignalKind::CpuUsed | SignalKind::MemoryAvailable => matches!(
            source,
            ObservationSource::WindowsCim | ObservationSource::WindowsPerfCounter
        ),
        SignalKind::MaxNormalizedDiskBusy
        | SignalKind::MaxDiskQueue
        | SignalKind::MaxDiskLatency => {
            matches!(source, ObservationSource::WindowsPerfCounter)
        }
        SignalKind::GpuFreeMemory | SignalKind::GpuTemperature => {
            matches!(source, ObservationSource::NvidiaSmi)
        }
        SignalKind::CloudRateUsed => matches!(source, ObservationSource::CodexRateLimits),
    }
}

/// Evaluates a single candidate using only explicit dependency, conflict, and
/// resource evidence. It performs no scheduling or side effects.
#[must_use]
pub fn evaluate_admission(
    candidate: &TaskCandidate,
    active_tasks: &[ActiveTask],
    host: &HostSnapshot,
    thresholds: &AdmissionThresholds,
    now: u64,
) -> AdmissionDecision {
    if let Some(checkpoint) = missing_prerequisite(candidate.prerequisites) {
        return AdmissionDecision::Hold(AdmissionReason::DependencyCheckpointMissing(checkpoint));
    }

    if matches!(candidate.class, ResourceClass::CloudLight)
        && !candidate.requirements.requires_cloud_quota
    {
        return AdmissionDecision::Hold(AdmissionReason::RequiredSignalMissing(
            RequiredSignal::CloudQuota,
        ));
    }
    if matches!(candidate.class, ResourceClass::GpuHeavy) && !candidate.requirements.requires_gpu {
        return AdmissionDecision::Hold(AdmissionReason::RequiredSignalMissing(
            RequiredSignal::Gpu,
        ));
    }

    for active in active_tasks {
        if active.task_key == candidate.task_key {
            return AdmissionDecision::Hold(AdmissionReason::TaskAlreadyActive);
        }
        if active.worktree == candidate.worktree {
            return AdmissionDecision::Hold(AdmissionReason::WorktreeConflict);
        }
        for resource in &candidate.shared_resources {
            if active.shared_resources.contains(resource) {
                return AdmissionDecision::Hold(AdmissionReason::SharedResourceConflict(*resource));
            }
        }
    }

    if host.window.duration_seconds < thresholds.min_observation_window_seconds {
        return AdmissionDecision::Hold(AdmissionReason::ObservationWindowTooShort);
    }
    if host.window.sample_count < thresholds.min_observation_samples {
        return AdmissionDecision::Hold(AdmissionReason::ObservationWindowTooSparse);
    }

    let cpu_used = match fresh_value(&host.cpu_used, SignalKind::CpuUsed, now) {
        Ok(value) => value,
        Err(reason) => return AdmissionDecision::Hold(reason),
    };
    if cpu_used >= thresholds.max_cpu_used {
        return AdmissionDecision::Hold(AdmissionReason::ThresholdExceeded(ThresholdKind::CpuUsed));
    }

    let memory_available = match fresh_value(
        &host.memory_available_bytes,
        SignalKind::MemoryAvailable,
        now,
    ) {
        Ok(value) => value,
        Err(reason) => return AdmissionDecision::Hold(reason),
    };
    if memory_available < thresholds.min_memory_available_bytes {
        return AdmissionDecision::Hold(AdmissionReason::ThresholdExceeded(
            ThresholdKind::MemoryAvailable,
        ));
    }

    let disk_busy = match fresh_value(
        &host.max_normalized_disk_busy,
        SignalKind::MaxNormalizedDiskBusy,
        now,
    ) {
        Ok(value) => value,
        Err(reason) => return AdmissionDecision::Hold(reason),
    };
    if disk_busy >= thresholds.max_disk_busy {
        return AdmissionDecision::Hold(AdmissionReason::ThresholdExceeded(
            ThresholdKind::MaxNormalizedDiskBusy,
        ));
    }

    let disk_queue = match fresh_value(&host.max_disk_queue_milli, SignalKind::MaxDiskQueue, now) {
        Ok(value) => value,
        Err(reason) => return AdmissionDecision::Hold(reason),
    };
    if disk_queue >= thresholds.max_disk_queue_milli {
        return AdmissionDecision::Hold(AdmissionReason::ThresholdExceeded(
            ThresholdKind::MaxDiskQueue,
        ));
    }

    let disk_latency = match fresh_value(
        &host.max_disk_latency_micros,
        SignalKind::MaxDiskLatency,
        now,
    ) {
        Ok(value) => value,
        Err(reason) => return AdmissionDecision::Hold(reason),
    };
    if disk_latency >= thresholds.max_disk_latency_micros {
        return AdmissionDecision::Hold(AdmissionReason::ThresholdExceeded(
            ThresholdKind::MaxDiskLatency,
        ));
    }

    if candidate.requirements.requires_gpu {
        let gpu_free = match fresh_value(&host.gpu_free_bytes, SignalKind::GpuFreeMemory, now) {
            Ok(value) => value,
            Err(reason) => return AdmissionDecision::Hold(reason),
        };
        if gpu_free < thresholds.min_gpu_free_bytes {
            return AdmissionDecision::Hold(AdmissionReason::ThresholdExceeded(
                ThresholdKind::GpuFreeMemory,
            ));
        }

        let gpu_temperature = match fresh_value(
            &host.gpu_temperature_deci_celsius,
            SignalKind::GpuTemperature,
            now,
        ) {
            Ok(value) => value,
            Err(reason) => return AdmissionDecision::Hold(reason),
        };
        if gpu_temperature >= thresholds.max_gpu_temperature_deci_celsius {
            return AdmissionDecision::Hold(AdmissionReason::ThresholdExceeded(
                ThresholdKind::GpuTemperature,
            ));
        }
    }

    if candidate.requirements.requires_cloud_quota {
        let cloud_rate = match fresh_value(&host.cloud_rate_used, SignalKind::CloudRateUsed, now) {
            Ok(value) => value,
            Err(reason) => return AdmissionDecision::Hold(reason),
        };
        if cloud_rate >= thresholds.max_cloud_rate_used {
            return AdmissionDecision::Hold(AdmissionReason::ThresholdExceeded(
                ThresholdKind::CloudRateUsed,
            ));
        }
    }

    AdmissionDecision::AdmitOne
}
