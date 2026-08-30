use lattice_artifact_store::{
    ManagedEvidenceError, ManagedEvidenceInput, ManagedEvidenceKind, VerifiedManagedEvidence,
    verify_untrusted_managed_evidence,
};
use lattice_contracts::{ContentDigest, ProjectId};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("digest")
}

fn evidence(bytes: &[u8]) -> VerifiedManagedEvidence {
    VerifiedManagedEvidence::new(
        ManagedEvidenceInput::new(
            ProjectId::new("project-managed-evidence").expect("project"),
            digest('1'),
            1,
            ManagedEvidenceKind::VerificationResult,
            "application/vnd.lattice.verification+json",
            "lattice.managed-verification/1.0",
            "lattice-foreman",
            "1.0",
            digest('2'),
            "2026-08-26T13:00:00Z",
            bytes.to_vec(),
        )
        .expect("input"),
    )
    .expect("verified evidence")
}

#[test]
fn exact_bounded_bytes_have_separate_content_and_descriptor_digests() {
    let record = evidence(br#"{"command_id":"verify.fixed.v1","exit_code":0}"#);
    assert_eq!(record.attempt(), 1);
    assert_eq!(record.kind(), ManagedEvidenceKind::VerificationResult);
    assert_eq!(
        record.bytes(),
        br#"{"command_id":"verify.fixed.v1","exit_code":0}"#
    );
    assert_ne!(record.content_digest(), record.descriptor_digest());
    assert_eq!(
        verify_untrusted_managed_evidence(&record.to_untrusted()),
        Ok(record.clone())
    );
    assert!(!format!("{record:?}").contains("verify.fixed.v1"));

    let untrusted = record
        .to_untrusted()
        .with_bytes(b"tampered-objective-sentinel".to_vec());
    let untrusted_debug = format!("{untrusted:?}");
    assert!(!untrusted_debug.contains("tampered-objective-sentinel"));
    assert!(untrusted_debug.contains("byte_length: 27"));
}

#[test]
fn tamper_oversize_and_secret_shaped_bytes_fail_closed() {
    let record = evidence(b"focused checks passed");
    assert_eq!(
        verify_untrusted_managed_evidence(
            &record
                .to_untrusted()
                .with_bytes(b"focused checks failed".to_vec())
        ),
        Err(ManagedEvidenceError::DigestMismatch)
    );
    assert_eq!(
        verify_untrusted_managed_evidence(
            &record
                .to_untrusted()
                .with_bytes(b"replayed ghp_do-not-persist".to_vec())
        ),
        Err(ManagedEvidenceError::ForbiddenContent)
    );
    assert_eq!(
        ManagedEvidenceInput::new(
            ProjectId::new("project-managed-evidence").unwrap(),
            digest('1'),
            1,
            ManagedEvidenceKind::WorkerLifecycle,
            "application/json",
            "lattice.worker-lifecycle/1.0",
            "lattice-foreman",
            "1.0",
            digest('2'),
            "2026-08-26T13:00:00Z",
            vec![b'x'; 1_048_577],
        ),
        Err(ManagedEvidenceError::BytesLimitExceeded)
    );
    assert_eq!(
        ManagedEvidenceInput::new(
            ProjectId::new("project-managed-evidence").unwrap(),
            digest('1'),
            1,
            ManagedEvidenceKind::ReviewResult,
            "application/json",
            "lattice.managed-review/1.0",
            "ghp_do-not-persist",
            "1.0",
            digest('2'),
            "2026-08-26T13:00:00Z",
            b"bounded review".to_vec(),
        ),
        Err(ManagedEvidenceError::ForbiddenContent)
    );
    let mut artifact_rows = Vec::new();
    for (kind, secret) in [
        (
            ManagedEvidenceKind::WorkerLifecycle,
            b"Authorization: Bearer live-secret".as_slice(),
        ),
        (
            ManagedEvidenceKind::WorkerLifecycle,
            b"password=hunter2".as_slice(),
        ),
        (
            ManagedEvidenceKind::WorkerLifecycle,
            b"https://user:secret@example.invalid/repo".as_slice(),
        ),
        (
            ManagedEvidenceKind::ReviewResult,
            br#"{"summary":"bare ghp_do-not-persist"}"#.as_slice(),
        ),
        (
            ManagedEvidenceKind::ReviewResult,
            br#"{"summary":"bare github_pat_do_not_persist"}"#.as_slice(),
        ),
        (
            ManagedEvidenceKind::VerificationResult,
            b"\xffbinary\x00sk-do-not-persist".as_slice(),
        ),
        (
            ManagedEvidenceKind::VerificationResult,
            b"\xffhttps://user:secret@example.invalid/repo".as_slice(),
        ),
        (
            ManagedEvidenceKind::VerificationResult,
            b"use AKIAIOSFODNN7EXAMPLE here".as_slice(),
        ),
    ] {
        let candidate = ManagedEvidenceInput::new(
            ProjectId::new("project-managed-evidence").unwrap(),
            digest('1'),
            1,
            kind,
            "application/json",
            "lattice.worker-lifecycle/1.0",
            "lattice-foreman",
            "1.0",
            digest('2'),
            "2026-08-26T13:00:00Z",
            secret.to_vec(),
        );
        assert_eq!(candidate, Err(ManagedEvidenceError::ForbiddenContent));
        if let Ok(input) = candidate {
            artifact_rows.push(VerifiedManagedEvidence::new(input).expect("verified row"));
        }
    }
    assert!(
        artifact_rows.is_empty(),
        "rejected evidence cannot append a row"
    );
}
