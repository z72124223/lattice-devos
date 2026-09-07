use lattice_runtime::{RuntimeCommand, parse_command};

fn args(reconcile: bool) -> Vec<String> {
    let mut args: Vec<String> = [
        if reconcile {
            "project-registry-reconcile"
        } else {
            "project-registry-inspect"
        },
        "--postgres-host",
        "127.0.0.1",
        "--postgres-port",
        "54321",
        "--postgres-run-id",
        "1234567890abcdef1234567890abcdef",
        "--project-id",
        "recovery-project",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if reconcile {
        args.extend([
            "--expected-revision".into(),
            "2".into(),
            "--expected-receipt-digest".into(),
            "a".repeat(64),
            "--pending-observation-digest".into(),
            "b".repeat(64),
        ]);
    }
    args
}

#[test]
fn registry_cli_parses_explicit_read_and_write_commands() {
    assert!(matches!(
        parse_command(&args(false)).unwrap(),
        RuntimeCommand::ProjectRegistryInspect { .. }
    ));
    assert!(matches!(
        parse_command(&args(true)).unwrap(),
        RuntimeCommand::ProjectRegistryReconcile { .. }
    ));
}

#[test]
fn registry_cli_rejects_extra_duplicate_missing_and_invalid_bounds() {
    for write in [false, true] {
        for name in ["--path", "--active", "--decision", "--sql", "--project-id"] {
            let mut bad = args(write);
            bad.extend([name.into(), "injected".into()]);
            assert!(parse_command(&bad).is_err());
        }
        let mut bad = args(write);
        bad.pop();
        assert!(parse_command(&bad).is_err());
    }
    for (name, values) in [
        (
            "--expected-revision",
            vec!["0", "-1", "18446744073709551616"],
        ),
        ("--expected-receipt-digest", vec!["short", "ZZZZ"]),
        ("--pending-observation-digest", vec!["short", "bad"]),
        ("--postgres-host", vec!["example.com", "0.0.0.0"]),
    ] {
        for value in values {
            let mut bad = args(true);
            let index = bad.iter().position(|v| v == name).unwrap();
            bad[index + 1] = value.into();
            assert!(parse_command(&bad).is_err(), "accepted {name}={value}");
        }
    }
}
