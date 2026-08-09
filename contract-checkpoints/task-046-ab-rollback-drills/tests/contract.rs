use std::collections::BTreeSet;

const FAULTS: &str = include_str!("fixtures/fault_matrix.tsv");

#[test]
fn fault_fixture_is_closed_unique_and_not_activation_evidence() {
    let required = [
        "F01_MANIFEST_CORRUPT",
        "F02_SLOT_ALIAS",
        "F03_SCHEMA_MIGRATION",
        "F04_NORMAL_GATEWAY_CLAIM",
        "F05_APPROVAL_EXPIRED",
        "F06_APPROVAL_REPLAY",
        "F07_CONCURRENT_CLAIM",
        "F08_CLAIM_ACK_LOSS",
        "F09_DRAIN_ACTIVE_LEASE",
        "F10_DRAIN_UNKNOWN_EFFECT",
        "F11_DRAIN_WRITABLE_CHILD",
        "F12_POWER_BEFORE_POINTER",
        "F13_POWER_AFTER_POINTER",
        "F14_POINTER_CHECKSUM",
        "F15_CANDIDATE_CRASH_LOOP",
        "F16_STALE_CONNECTION",
        "F17_CANARY_USER_WRITE",
        "F18_CANARY_REPLAY_MISMATCH",
        "F19_HEALTH_DISAGREEMENT",
        "F20_ROLLBACK_EPOCH",
        "F21_ROLLBACK_SLOT_BAD",
        "F22_POWER_EACH_EDGE",
        "F23_GUARDIAN_UNAVAILABLE",
    ];
    let allowed_status = ["BLOCKED_TASK045", "RED_NOT_RUN"];
    let mut ids = BTreeSet::new();
    for (line_number, line) in FAULTS
        .lines()
        .enumerate()
        .skip(1)
        .filter(|(_, line)| !line.trim().is_empty())
    {
        let columns: Vec<_> = line.split('\t').collect();
        assert_eq!(columns.len(), 6, "fault line {}", line_number + 1);
        assert!(ids.insert(columns[0]), "duplicate {}", columns[0]);
        assert!(
            allowed_status.contains(&columns[4]),
            "unknown status {}",
            columns[4]
        );
        assert_ne!(columns[4], "PASS", "fixture is not drill evidence");
        assert!(!columns[2].is_empty(), "required observation");
        assert!(!columns[3].is_empty(), "required disposition");
        assert!(!columns[5].is_empty(), "forbidden claim");
    }
    assert_eq!(ids.len(), required.len());
    for id in required {
        assert!(ids.contains(id), "missing required fault {id}");
    }
}

#[cfg(feature = "acceptance")]
#[test]
fn task_046_remains_red_until_every_disposable_drill_passes() {
    let not_passed: Vec<_> = FAULTS
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let columns: Vec<_> = line.split('\t').collect();
            (columns[4] != "PASS").then_some(format!("{}={}", columns[0], columns[4]))
        })
        .collect();
    assert!(
        not_passed.is_empty(),
        "TASK046_RED_DRILLS_NOT_RUN: {}",
        not_passed.join(",")
    );
}
