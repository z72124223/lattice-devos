use lattice_runtime::{RuntimeCommand, parse_command};

fn args(command: &str) -> Vec<String> {
    [
        command,
        "--postgres-host",
        "127.0.0.1",
        "--postgres-port",
        "58743",
        "--postgres-run-id",
        "1234567890abcdef1234567890abcdef",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[test]
fn lifecycle_requires_explicit_install_and_fixed_loopback_binding() {
    for (name, expected_install) in [
        ("bot-lifecycle", false),
        ("bot-lifecycle-install", true),
        ("bot-lifecycle-migrate", false),
        ("bot-lifecycle-reconcile-archive", false),
    ] {
        assert!(
            matches!(parse_command(&args(name)).unwrap(), RuntimeCommand::BotLifecycle { install, migrate, reconcile_archive, port: 58743, .. } if install == expected_install && migrate == (name == "bot-lifecycle-migrate") && reconcile_archive == (name == "bot-lifecycle-reconcile-archive"))
        );
        for (index, value) in [(2, "0.0.0.0"), (4, "0"), (6, "../../other")] {
            let mut invalid = args(name);
            invalid[index] = value.into();
            assert!(parse_command(&invalid).is_err());
        }
        let mut extra = args(name);
        extra.extend(["--sql".into(), "SELECT 1".into()]);
        assert!(parse_command(&extra).is_err());
        let mut duplicate = args(name);
        duplicate.extend(["--postgres-port".into(), "58743".into()]);
        assert!(parse_command(&duplicate).is_err());
        assert!(parse_command(&args(name)[..5]).is_err());
    }
}
