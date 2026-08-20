use resource_aware_scheduler_phase0::{
    evaluate_admission, schedule_next_session, ActiveTask, AdmissionDecision, AdmissionReason,
    AdmissionThresholds, DependencyCheckpoint, EvidenceWindow, HostSnapshot, IdentityFault,
    Observation, ObservationSource, ObservationState, Permille, Prerequisites, RequiredSignal,
    ResourceClass, ResourceRequirements, SampleFailure, SchedulerDecision, SchedulerLimits,
    SharedResource, SignalKind, TaskCandidate, ThresholdKind, WorktreeId,
};

const GIB: u64 = 1024 * 1024 * 1024;

fn fresh<T>(value: T) -> Observation<T> {
    Observation::observed(ObservationSource::TestFixture, 90, 110, value)
        .expect("valid fixture observation")
}

fn thresholds(max_cpu_used: u16) -> AdmissionThresholds {
    AdmissionThresholds {
        max_cpu_used: Permille::new(max_cpu_used).expect("valid threshold"),
        min_memory_available_bytes: 12 * GIB,
        max_disk_busy: Permille::new(300).expect("valid threshold"),
        max_disk_queue_milli: 1_000,
        max_disk_latency_micros: 20_000,
        min_gpu_free_bytes: 4 * GIB,
        max_gpu_temperature_deci_celsius: 750,
        max_cloud_rate_used: Permille::new(850).expect("valid threshold"),
        min_observation_window_seconds: 60,
        min_observation_samples: 6,
    }
}

fn healthy_host() -> HostSnapshot {
    HostSnapshot {
        window: EvidenceWindow {
            duration_seconds: 60,
            sample_count: 6,
        },
        cpu_used: fresh(Permille::new(207).expect("valid sample")),
        memory_available_bytes: fresh(19 * GIB),
        max_normalized_disk_busy: fresh(Permille::new(0).expect("valid sample")),
        max_disk_queue_milli: fresh(0),
        max_disk_latency_micros: fresh(500),
        gpu_free_bytes: fresh(9 * GIB),
        gpu_temperature_deci_celsius: fresh(600),
        cloud_rate_used: fresh(Permille::new(200).expect("valid sample")),
    }
}

fn ready_candidate() -> TaskCandidate {
    TaskCandidate {
        task_key: "issue-007-fixture".to_owned(),
        worktree: WorktreeId::new("issue-007-resource-aware-scheduler")
            .expect("valid symbolic worktree id"),
        class: ResourceClass::CloudLight,
        requirements: ResourceRequirements {
            requires_gpu: false,
            requires_cloud_quota: true,
        },
        prerequisites: Prerequisites::ready(),
        shared_resources: vec![],
    }
}

fn candidate_with_key(key: &str) -> TaskCandidate {
    let mut candidate = ready_candidate();
    candidate.task_key = key.to_owned();
    candidate.worktree = WorktreeId::new(key).expect("valid symbolic worktree id");
    candidate
}

#[test]
fn unknown_formal_cloud_quota_holds_instead_of_guessing() {
    let mut host = healthy_host();
    host.cloud_rate_used = Observation::unknown(ObservationSource::CodexRateLimits);

    assert_eq!(
        evaluate_admission(&ready_candidate(), &[], &host, &thresholds(500), 100),
        AdmissionDecision::Hold(AdmissionReason::SignalNotUsable {
            signal: SignalKind::CloudRateUsed,
            state: ObservationState::Unknown,
        })
    );
}

#[test]
fn cloud_quota_rejects_a_value_from_the_wrong_authority() {
    let mut host = healthy_host();
    host.cloud_rate_used = Observation::observed(
        ObservationSource::WindowsCim,
        90,
        110,
        Permille::new(1).expect("valid sample"),
    )
    .expect("valid observation");

    assert_eq!(
        evaluate_admission(&ready_candidate(), &[], &host, &thresholds(500), 100),
        AdmissionDecision::Hold(AdmissionReason::SignalSourceMismatch {
            signal: SignalKind::CloudRateUsed,
            actual: ObservationSource::WindowsCim,
        })
    );
}

#[test]
fn unstable_task_control_checkpoint_blocks_current_integration() {
    let mut candidate = ready_candidate();
    candidate.prerequisites.task_control_stable = false;

    assert_eq!(
        evaluate_admission(&candidate, &[], &healthy_host(), &thresholds(500), 100),
        AdmissionDecision::Hold(AdmissionReason::DependencyCheckpointMissing(
            DependencyCheckpoint::TaskControlStable,
        ))
    );
}

#[test]
fn resource_class_cannot_bypass_its_required_formal_signal() {
    let mut candidate = ready_candidate();
    candidate.requirements.requires_cloud_quota = false;
    assert_eq!(
        evaluate_admission(&candidate, &[], &healthy_host(), &thresholds(500), 100),
        AdmissionDecision::Hold(AdmissionReason::RequiredSignalMissing(
            RequiredSignal::CloudQuota,
        ))
    );

    candidate.class = ResourceClass::GpuHeavy;
    assert_eq!(
        evaluate_admission(&candidate, &[], &healthy_host(), &thresholds(500), 100),
        AdmissionDecision::Hold(AdmissionReason::RequiredSignalMissing(RequiredSignal::Gpu,))
    );
}

#[test]
fn a_short_or_sparse_window_cannot_stand_in_for_rolling_evidence() {
    let mut host = healthy_host();
    host.window.duration_seconds = 25;
    assert_eq!(
        evaluate_admission(&ready_candidate(), &[], &host, &thresholds(500), 100),
        AdmissionDecision::Hold(AdmissionReason::ObservationWindowTooShort)
    );

    host.window.duration_seconds = 60;
    host.window.sample_count = 1;
    assert_eq!(
        evaluate_admission(&ready_candidate(), &[], &host, &thresholds(500), 100),
        AdmissionDecision::Hold(AdmissionReason::ObservationWindowTooSparse)
    );
}

#[test]
fn same_worktree_and_shared_resource_claims_are_exclusive() {
    let candidate = ready_candidate();
    let same_worktree = ActiveTask {
        task_key: "task-038".to_owned(),
        worktree: candidate.worktree.clone(),
        shared_resources: vec![],
    };
    assert_eq!(
        evaluate_admission(
            &candidate,
            &[same_worktree],
            &healthy_host(),
            &thresholds(500),
            100,
        ),
        AdmissionDecision::Hold(AdmissionReason::WorktreeConflict)
    );

    let mut candidate = ready_candidate();
    candidate.shared_resources = vec![SharedResource::McpDispatcher];
    let active = ActiveTask {
        task_key: "task-038".to_owned(),
        worktree: WorktreeId::new("chatgpt-mcp").expect("valid symbolic worktree id"),
        shared_resources: vec![SharedResource::McpDispatcher],
    };
    assert_eq!(
        evaluate_admission(
            &candidate,
            &[active],
            &healthy_host(),
            &thresholds(500),
            100
        ),
        AdmissionDecision::Hold(AdmissionReason::SharedResourceConflict(
            SharedResource::McpDispatcher,
        ))
    );
}

#[test]
fn an_active_task_identity_cannot_be_admitted_twice() {
    let candidate = ready_candidate();
    let active = ActiveTask {
        task_key: candidate.task_key.clone(),
        worktree: WorktreeId::new("another-worktree").expect("valid symbolic worktree id"),
        shared_resources: vec![],
    };

    assert_eq!(
        evaluate_admission(
            &candidate,
            &[active],
            &healthy_host(),
            &thresholds(500),
            100
        ),
        AdmissionDecision::Hold(AdmissionReason::TaskAlreadyActive)
    );
}

#[test]
fn worktree_identity_is_symbolic_and_rejects_path_syntax() {
    assert_eq!(
        WorktreeId::new("../chatgpt-mcp"),
        Err(IdentityFault::InvalidCharacter)
    );
    assert_eq!(
        WorktreeId::new("C:\\repo\\worktree"),
        Err(IdentityFault::InvalidCharacter)
    );
}

#[test]
fn normalized_per_disk_signal_is_required_not_aggregate_percent_disk_time() {
    let mut host = healthy_host();
    host.max_normalized_disk_busy = Observation::unknown(ObservationSource::WindowsPerfCounter);

    assert_eq!(
        evaluate_admission(&ready_candidate(), &[], &host, &thresholds(500), 100),
        AdmissionDecision::Hold(AdmissionReason::SignalNotUsable {
            signal: SignalKind::MaxNormalizedDiskBusy,
            state: ObservationState::Unknown,
        })
    );
}

#[test]
fn cpu_heavy_short_window_above_threshold_holds() {
    let mut candidate = ready_candidate();
    candidate.class = ResourceClass::CpuHeavy;
    candidate.requirements.requires_cloud_quota = false;
    let mut host = healthy_host();
    host.window.duration_seconds = 90;
    host.cpu_used = fresh(Permille::new(440).expect("valid sample"));
    let mut cpu_thresholds = thresholds(350);
    cpu_thresholds.min_observation_window_seconds = 90;
    assert_eq!(
        evaluate_admission(&candidate, &[], &host, &cpu_thresholds, 100),
        AdmissionDecision::Hold(AdmissionReason::ThresholdExceeded(ThresholdKind::CpuUsed,))
    );
}

#[test]
fn failed_memory_sample_is_not_zero_and_fails_closed() {
    let mut host = healthy_host();
    host.memory_available_bytes = Observation::failed(
        ObservationSource::WindowsCim,
        90,
        SampleFailure::QueryFailed,
    );

    assert_eq!(
        evaluate_admission(&ready_candidate(), &[], &host, &thresholds(500), 100),
        AdmissionDecision::Hold(AdmissionReason::SignalNotUsable {
            signal: SignalKind::MemoryAvailable,
            state: ObservationState::Failed(SampleFailure::QueryFailed),
        })
    );
}

#[test]
fn healthy_independent_fixture_admits_exactly_one_candidate() {
    assert_eq!(
        evaluate_admission(
            &ready_candidate(),
            &[],
            &healthy_host(),
            &thresholds(500),
            100
        ),
        AdmissionDecision::AdmitOne
    );
}

#[test]
fn scheduler_selects_the_first_admissible_candidate_after_reporting_prior_holds() {
    let mut blocked = candidate_with_key("blocked-on-dependency");
    blocked.prerequisites.dependencies_complete = false;
    let ready = candidate_with_key("ready-cloud-light");

    assert_eq!(
        schedule_next_session(
            &[blocked, ready],
            &[],
            &healthy_host(),
            &thresholds(500),
            SchedulerLimits { max_sessions: 4 },
            100,
        ),
        SchedulerDecision::StartOne {
            queue_index: 1,
            task_key: "ready-cloud-light".to_owned(),
            prior_holds: vec![(
                0,
                "blocked-on-dependency".to_owned(),
                AdmissionReason::DependencyCheckpointMissing(
                    DependencyCheckpoint::DependenciesComplete,
                )
            )],
        }
    );
}

#[test]
fn scheduler_never_starts_more_than_one_session_per_evaluation() {
    assert_eq!(
        schedule_next_session(
            &[candidate_with_key("first"), candidate_with_key("second")],
            &[],
            &healthy_host(),
            &thresholds(500),
            SchedulerLimits { max_sessions: 4 },
            100,
        ),
        SchedulerDecision::StartOne {
            queue_index: 0,
            task_key: "first".to_owned(),
            prior_holds: vec![],
        }
    );
}

#[test]
fn scheduler_holds_when_session_cap_is_reached_before_resource_evaluation() {
    let active = ActiveTask {
        task_key: "active".to_owned(),
        worktree: WorktreeId::new("active-worktree").expect("valid symbolic worktree id"),
        shared_resources: vec![],
    };

    assert_eq!(
        schedule_next_session(
            &[candidate_with_key("ready")],
            &[active],
            &healthy_host(),
            &thresholds(500),
            SchedulerLimits { max_sessions: 1 },
            100,
        ),
        SchedulerDecision::HoldAll {
            reason: AdmissionReason::SessionLimitReached,
            held: vec![],
        }
    );
}

#[test]
fn scheduler_reports_all_candidate_hold_reasons_when_none_can_start() {
    let mut dependency_blocked = candidate_with_key("dependency-blocked");
    dependency_blocked.prerequisites.worker_observation_stable = false;
    let mut resource_blocked = candidate_with_key("resource-blocked");
    resource_blocked.class = ResourceClass::CpuHeavy;
    resource_blocked.requirements.requires_cloud_quota = false;
    let mut host = healthy_host();
    host.cpu_used = fresh(Permille::new(900).expect("valid sample"));

    assert_eq!(
        schedule_next_session(
            &[dependency_blocked, resource_blocked],
            &[],
            &host,
            &thresholds(500),
            SchedulerLimits { max_sessions: 4 },
            100,
        ),
        SchedulerDecision::HoldAll {
            reason: AdmissionReason::NoCandidateAdmissible,
            held: vec![
                (
                    0,
                    "dependency-blocked".to_owned(),
                    AdmissionReason::DependencyCheckpointMissing(
                        DependencyCheckpoint::WorkerObservationStable,
                    )
                ),
                (
                    1,
                    "resource-blocked".to_owned(),
                    AdmissionReason::ThresholdExceeded(ThresholdKind::CpuUsed,)
                )
            ],
        }
    );
}
