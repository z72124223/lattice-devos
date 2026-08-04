use lattice_runtime::{RuntimeCommand, RuntimeError, parse_command};

#[test]
fn parses_only_the_exact_codex_preflight_surface() {
    let arguments = vec![
        "codex-preflight".to_owned(),
        "--launcher".to_owned(),
        r"C:\tools\codex.exe".to_owned(),
        "--version".to_owned(),
        "codex-cli 0.144.6".to_owned(),
        "--sha256".to_owned(),
        "a".repeat(64),
        "--schema-dir".to_owned(),
        r"C:\temp\schema".to_owned(),
    ];

    assert_eq!(
        parse_command(&arguments),
        Ok(RuntimeCommand::CodexPreflight {
            launcher: r"C:\tools\codex.exe".into(),
            version: "codex-cli 0.144.6".to_owned(),
            sha256: "a".repeat(64),
            schema_dir: r"C:\temp\schema".into(),
        })
    );
}

#[test]
fn rejects_missing_duplicate_unknown_and_malformed_preflight_arguments() {
    assert_eq!(parse_command(&[]), Err(RuntimeError::Usage));
    assert_eq!(
        parse_command(&["unknown".to_owned()]),
        Err(RuntimeError::Usage)
    );

    let missing = vec![
        "codex-preflight".to_owned(),
        "--launcher".to_owned(),
        r"C:\tools\codex.exe".to_owned(),
    ];
    assert_eq!(parse_command(&missing), Err(RuntimeError::Usage));

    let duplicate = vec![
        "codex-preflight".to_owned(),
        "--launcher".to_owned(),
        r"C:\tools\codex.exe".to_owned(),
        "--launcher".to_owned(),
        r"C:\other\codex.exe".to_owned(),
        "--version".to_owned(),
        "codex-cli 0.144.6".to_owned(),
        "--sha256".to_owned(),
        "a".repeat(64),
        "--schema-dir".to_owned(),
        r"C:\temp\schema".to_owned(),
    ];
    assert_eq!(parse_command(&duplicate), Err(RuntimeError::Usage));

    let mut malformed_digest = duplicate;
    malformed_digest[3] = "--version".to_owned();
    malformed_digest[4] = "codex-cli 0.144.6".to_owned();
    malformed_digest.drain(5..7);
    let digest_index = malformed_digest
        .iter()
        .position(|value| value == "--sha256")
        .expect("digest option remains");
    malformed_digest[digest_index + 1] = "not-a-digest".to_owned();
    assert_eq!(
        parse_command(&malformed_digest),
        Err(RuntimeError::InvalidDigest)
    );
}
