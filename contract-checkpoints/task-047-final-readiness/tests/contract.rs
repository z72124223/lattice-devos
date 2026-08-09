use std::collections::BTreeSet;

const EVIDENCE: &str = include_str!("fixtures/evidence_matrix.tsv");

#[test]
fn phase_a_packet_is_closed_unique_and_blocked() {
    let required: BTreeSet<_> = (1..=15).map(|n| format!("E{n:02}")).collect();
    let allowed_status = ["PASS_PHASE_A_ONLY", "BLOCKED", "MISSING", "OUT_OF_SCOPE"];
    let mut ids = BTreeSet::new();
    let mut blocking = Vec::new();
    for (line_number, line) in EVIDENCE
        .lines()
        .enumerate()
        .skip(1)
        .filter(|(_, line)| !line.trim().is_empty())
    {
        let columns: Vec<_> = line.split('\t').collect();
        assert_eq!(columns.len(), 5, "evidence line {}", line_number + 1);
        assert!(
            ids.insert(columns[0].to_owned()),
            "duplicate {}",
            columns[0]
        );
        assert!(
            allowed_status.contains(&columns[3]),
            "unknown status {}",
            columns[3]
        );
        if matches!(columns[3], "BLOCKED" | "MISSING") {
            blocking.push(columns[0]);
        }
        if columns[3] == "OUT_OF_SCOPE" {
            assert!(matches!(columns[0], "E14" | "E15"));
        }
    }
    assert_eq!(ids, required);
    assert!(!blocking.is_empty(), "Phase A cannot be ready");
    assert!(!ids.contains("MERGED"));
    assert!(!ids.contains("DEPLOYED"));
    assert!(!ids.contains("RELEASED"));
}

#[cfg(feature = "acceptance")]
#[test]
fn task_047_remains_red_until_required_evidence_is_current() {
    let missing: Vec<_> = EVIDENCE
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let columns: Vec<_> = line.split('\t').collect();
            let required_for_readiness = !matches!(columns[3], "OUT_OF_SCOPE");
            (required_for_readiness && columns[3] != "PASS")
                .then_some(format!("{}={}", columns[0], columns[3]))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "TASK047_RED_EVIDENCE_INCOMPLETE: {}",
        missing.join(",")
    );
}
