use std::collections::BTreeSet;

use lattice_contracts::{
    ApprovalAuthority, ApprovalIdentity, ApprovalLane, ApprovalOrigin, ApprovalSubject,
    ContentDigest, ContractError, DaemonEpoch, GuardianRuntimeSubject, ProtectedReleaseSubject,
    ReleaseSubject, RuntimeAdmissionMode, SubjectBinding, TaskId, UpgradeDelta,
};
use lattice_contracts::{ProjectId, ProjectSnapshotId};

const REQUIREMENTS: &str = include_str!("fixtures/requirements.tsv");
const STATES: &str = include_str!("fixtures/state_machine.tsv");

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid fixture digest")
}

fn fixture_release() -> ProtectedReleaseSubject {
    let release = ReleaseSubject::new(
        "activation-task-045",
        "saga-task-045",
        "release-b",
        "1",
        digest('1'),
        "a".repeat(40),
        digest('2'),
        digest('3'),
        vec![digest('4')],
        vec![],
        digest('5'),
        "release-a",
        digest('6'),
        "slot-a",
        "slot-b",
        DaemonEpoch::new(8).expect("positive target epoch"),
        true,
        UpgradeDelta::new(false, false, false, false, false, false, false, false),
    )
    .expect("valid immutable release subject");
    let guardian = GuardianRuntimeSubject::new(
        "guardian-task-045",
        digest('7'),
        "guardian-daemon-task-045",
        DaemonEpoch::new(8).expect("positive guardian epoch"),
    )
    .expect("valid guardian subject");
    ProtectedReleaseSubject::new(release, guardian)
}

#[test]
fn phase_a_fixtures_are_closed_unique_and_fail_closed() {
    let allowed_evidence = [
        "READY",
        "BASELINE_GREEN_PURE",
        "RED_MISSING_OWNER",
        "BLOCKED_TASK038",
        "BLOCKED_GOVERNANCE",
    ];
    let mut requirement_ids = BTreeSet::new();
    let mut ready_requirement_ids = BTreeSet::new();
    let mut not_ready = 0usize;
    let mut rows = 0usize;
    for (line_number, line) in REQUIREMENTS
        .lines()
        .enumerate()
        .skip(1)
        .filter(|(_, line)| !line.trim().is_empty())
    {
        let columns: Vec<_> = line.split('\t').collect();
        assert_eq!(columns.len(), 5, "requirements line {}", line_number + 1);
        assert!(
            requirement_ids.insert(columns[0]),
            "duplicate {}",
            columns[0]
        );
        assert!(
            allowed_evidence.contains(&columns[3]),
            "unknown evidence {}",
            columns[3]
        );
        if columns[3] == "READY" {
            ready_requirement_ids.insert(columns[0]);
        } else {
            not_ready += 1;
        }
        rows += 1;
    }
    assert_eq!(rows, 12, "closed TASK-045 requirement count");
    assert_eq!(
        ready_requirement_ids,
        BTreeSet::from(["G12_ACTIVE_CONSTITUTION"]),
        "only the independently validated constitution is ready"
    );
    assert!(not_ready > 0, "Phase A must remain fail-closed overall");

    let allowed_states = [
        "STAGED",
        "PROMOTION_APPROVED",
        "ACTIVATION_CLAIMED",
        "OLD_DAEMON_DRAINED",
        "SLOT_POINTER_WRITTEN",
        "CANDIDATE_STARTED",
        "EPOCH_ACTIVATED",
        "HEALTH_WINDOW_PASSED",
        "ACTIVATION_FINALIZED",
        "ROLLBACK_REQUESTED",
        "ROLLBACK_SLOT_WRITTEN",
        "ROLLBACK_DAEMON_STARTED",
        "ROLLBACK_EPOCH_ACTIVATED",
        "ROLLED_BACK",
        "RECONCILIATION_REQUIRED",
    ];
    let allowed_admission = RuntimeAdmissionMode::ALL.map(RuntimeAdmissionMode::as_str);
    let mut transition_ids = BTreeSet::new();
    let mut transitions = 0usize;
    for (line_number, line) in STATES
        .lines()
        .enumerate()
        .skip(1)
        .filter(|(_, line)| !line.trim().is_empty())
    {
        let columns: Vec<_> = line.split('\t').collect();
        assert_eq!(columns.len(), 6, "state line {}", line_number + 1);
        assert!(
            transition_ids.insert(columns[0]),
            "duplicate {}",
            columns[0]
        );
        assert!(allowed_states.contains(&columns[1]));
        assert!(allowed_states.contains(&columns[2]));
        assert!(allowed_admission.contains(&columns[3]));
        assert!(matches!(columns[5], "ALLOW" | "DENY"));
        transitions += 1;
    }
    assert_eq!(transitions, 15, "closed TASK-045 transition count");
}

#[test]
fn normal_gateway_lane_cannot_construct_a_protected_release_identity() {
    let binding = SubjectBinding::new(
        ProjectId::new("lattice-system").expect("project"),
        ProjectSnapshotId::new("snapshot-task-045").expect("snapshot"),
        TaskId::new("TASK-045").expect("task"),
        "1",
        digest('8'),
    )
    .expect("binding");
    let identity = ApprovalIdentity::new(
        "approval-task-045",
        "challenge-task-045",
        binding,
        ApprovalSubject::ProtectedRelease(Box::new(fixture_release())),
        "requester-task-045",
        "normal-gateway-user",
        ApprovalAuthority::ResponsibleUser,
        ApprovalOrigin::NormalGateway,
        ApprovalLane::Normal,
        "openclaw-channel",
        "openclaw-session",
    );
    assert_eq!(
        identity,
        Err(ContractError::InvalidApprovalIdentity {
            field: "authority_origin_lane"
        })
    );
}

#[cfg(feature = "acceptance")]
#[test]
fn task_045_remains_red_until_every_owner_requirement_is_ready() {
    let not_ready: Vec<_> = REQUIREMENTS
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let columns: Vec<_> = line.split('\t').collect();
            (columns[3] != "READY").then_some(format!("{}={}", columns[0], columns[3]))
        })
        .collect();
    assert!(
        not_ready.is_empty(),
        "TASK045_RED_NOT_READY: {}",
        not_ready.join(",")
    );
}
