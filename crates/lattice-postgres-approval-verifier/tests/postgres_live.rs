use std::env;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use lattice_approval_verifier::{
    ApprovalCommandOutcome, ApprovalDenial, ApprovalEffectClaimIntent, ApprovalIssueRequest,
    ApprovalNormalClaimExecution, ApprovalNormalClaimRequest, ApprovalRepository,
    ApprovalRepositoryCommand, ApprovalRepositoryErrorKind, ApprovalVerifyRequest,
    FakeNormalSigner, FakeProtectedSigner, SecretMaterial, nonce_commitment,
};
use lattice_contracts::ContentDigest;
use lattice_contracts::{
    ApprovalAuthority, ApprovalIdentity, ApprovalLane, ApprovalOrigin, ApprovalSubject,
    DaemonEpoch, GuardianRuntimeSubject, ProjectId, ProjectSnapshotId, ProtectedReleaseSubject,
    ReleaseSubject, SubjectBinding, TaskId, UpgradeDelta,
};
use lattice_postgres_approval_verifier::{
    ExtensionApplyOutcome, ExtensionTarget, PostgresApprovalVerifier, apply_extension,
    verify_extension,
};
use postgres::{Client, NoTls};

#[test]
fn exact_approval_extension_install_and_restart_profile() {
    if env::var("LATTICE_TASK024_APPROVAL_LIVE").as_deref() != Ok("1") {
        eprintln!("SKIP: LATTICE_TASK024_APPROVAL_LIVE is not enabled");
        return;
    }
    let phase = required("LATTICE_TASK024_APPROVAL_PHASE");
    let target = ExtensionTarget::new(
        required("LATTICE_APPROVAL_DATABASE_NAME"),
        env_digest("LATTICE_APPROVAL_DATABASE_IDENTITY_SHA256"),
        env_digest("LATTICE_APPROVAL_GLOBAL_MANIFEST_SHA256"),
        env_digest("LATTICE_APPROVAL_MEMORY_MANIFEST_SHA256"),
    )
    .expect("closed extension target");
    let mut migrator = Client::connect(&required("LATTICE_APPROVAL_MIGRATOR_URL"), NoTls)
        .expect("connect marker-owned migrator");

    match phase.as_str() {
        "initial" => {
            assert_eq!(
                apply_or_report(&mut migrator, &target, "INITIAL_APPLY"),
                ExtensionApplyOutcome::Installed
            );
            assert_eq!(
                apply_or_report(&mut migrator, &target, "INITIAL_REAPPLY"),
                ExtensionApplyOutcome::AlreadyCurrent
            );
            verify_or_report(&mut migrator, &target, "INITIAL_VERIFY");
            println!("TASK024_INITIAL_SETUP_PROFILE_PASS");
            assert_runtime_surface_is_closed();
            exercise_repository(&target, "initial");
            exercise_global_nonce_rejection(&target);
            exercise_revocation(&target);
            exercise_concurrent_claimers(&target);
            exercise_protected_denial(&target);
            exercise_commit_response_uncertainty(&target);
            exercise_committed_corruption_rejection(&mut migrator, &target);
            assert_physical_shape(&mut migrator, 16, 2);
            println!("TASK024_APPROVAL_EXTENSION_INITIAL_PASS");
        }
        "restart" => {
            assert_eq!(
                apply_or_report(&mut migrator, &target, "RESTART_REAPPLY"),
                ExtensionApplyOutcome::AlreadyCurrent
            );
            verify_or_report(&mut migrator, &target, "RESTART_VERIFY");
            println!("TASK024_RESTART_SETUP_PROFILE_PASS");
            assert_runtime_surface_is_closed();
            let mut repository = runtime_repository(&target);
            assert!(
                repository
                    .current_authority("approval-live-initial")
                    .expect("replay claimed initial approval")
                    .is_none()
            );
            exercise_repository(&target, "restart");
            assert_physical_shape(&mut migrator, 19, 3);
            println!("TASK024_APPROVAL_EXTENSION_RESTART_PASS");
        }
        _ => panic!("unsupported marker-owned phase"),
    }
}

fn exercise_global_nonce_rejection(target: &ExtensionTarget) {
    let first = normal_issue("nonce-owner");
    let mut second = normal_issue("nonce-denied");
    let ApprovalRepositoryCommand::Issue(first_request) = &first else {
        unreachable!("normal issue helper is closed")
    };
    let ApprovalRepositoryCommand::Issue(second_request) = &mut second else {
        unreachable!("normal issue helper is closed")
    };
    second_request.nonce_id.clone_from(&first_request.nonce_id);
    second_request
        .nonce_commitment
        .clone_from(&first_request.nonce_commitment);
    let mut repository = runtime_repository(target);
    assert_eq!(
        repository
            .execute(first)
            .expect("first global nonce owner")
            .outcome,
        ApprovalCommandOutcome::Applied
    );
    let denied = repository
        .execute(second.clone())
        .expect("global nonce denial persists");
    assert_eq!(
        denied.outcome,
        ApprovalCommandOutcome::Denied(ApprovalDenial::NonceAlreadyBound)
    );
    assert_eq!(
        runtime_repository(target)
            .execute(second)
            .expect("denied global nonce exact retry"),
        denied
    );
    println!("TASK024_GLOBAL_NONCE_REJECTION_PASS");
}

fn exercise_revocation(target: &ExtensionTarget) {
    let signer = normal_signer();
    let approval_id = "approval-live-revoke";
    let mut repository = runtime_repository(target);
    let issued = repository
        .execute(normal_issue("revoke"))
        .expect("revocation issue");
    let verified = repository
        .execute(ApprovalRepositoryCommand::Verify(ApprovalVerifyRequest {
            command_id: "verify-live-revoke".to_owned(),
            approval_id: approval_id.to_owned(),
            expected_head: issued.after.clone().expect("revocation challenged head"),
            proof: signer
                .sign(issued.challenge.as_ref().expect("revocation challenge"))
                .expect("revocation proof"),
        }))
        .expect("revocation verify");
    let revoke =
        ApprovalRepositoryCommand::Revoke(lattice_approval_verifier::ApprovalRevokeRequest {
            command_id: "revoke-live-revoke".to_owned(),
            approval_id: approval_id.to_owned(),
            expected_head: verified.after.expect("revocation verified head"),
            revoker_id: signer.approver_id().to_owned(),
            revocation_evidence_digest: digest('6'),
        });
    let revoked = repository
        .execute(revoke.clone())
        .expect("durable revocation");
    assert_eq!(revoked.outcome, ApprovalCommandOutcome::Applied);
    assert_eq!(
        runtime_repository(target)
            .execute(revoke)
            .expect("durable revocation exact retry"),
        revoked
    );
    assert!(
        repository
            .current_authority(approval_id)
            .expect("revoked current authority")
            .is_none()
    );
    println!("TASK024_REVOCATION_PASS");
}

fn exercise_commit_response_uncertainty(target: &ExtensionTarget) {
    println!("TASK024_COMMIT_RESPONSE_UNCERTAINTY_ENTER");
    let original_port = required("LATTICE_TASK019_PORT")
        .parse::<u16>()
        .expect("bounded marker-owned PostgreSQL port");
    let (proxy_port, proxy) = commit_response_drop_proxy(original_port);
    println!("TASK024_COMMIT_RESPONSE_UNCERTAINTY_PROXY_READY");
    let direct_url = required("LATTICE_APPROVAL_RUNTIME_URL");
    let proxy_url =
        direct_url.replacen(&format!(":{original_port}/"), &format!(":{proxy_port}/"), 1);
    assert_ne!(
        proxy_url, direct_url,
        "runtime URL must bind the fixture port"
    );
    let proxied_client = Client::connect(&proxy_url, NoTls).expect("proxied runtime connection");
    println!("TASK024_COMMIT_RESPONSE_UNCERTAINTY_CONNECTED");
    let mut proxied = PostgresApprovalVerifier::new(proxied_client, target.clone())
        .expect("proxied runtime adapter");
    println!("TASK024_COMMIT_RESPONSE_UNCERTAINTY_ADAPTER_READY");
    let issue = normal_issue("commit-unknown");
    let result = proxied.execute(issue.clone());
    println!(
        "TASK024_COMMIT_RESPONSE_UNCERTAINTY_RESULT_{}",
        match &result {
            Ok(_) => "UNEXPECTED_SUCCESS",
            Err(error) => error.code(),
        }
    );
    let error = result.expect_err("a discarded COMMIT response cannot return success");
    assert_eq!(
        error.kind(),
        ApprovalRepositoryErrorKind::CommitOutcomeUnknown
    );
    proxy.join().expect("commit-response proxy");
    println!("TASK024_COMMIT_RESPONSE_UNCERTAINTY_PROXY_CLOSED");

    let mut reconciler = runtime_repository(target);
    assert!(
        reconciler
            .current_authority("approval-live-commit-unknown")
            .expect("fresh state reconciliation")
            .is_none(),
        "an issued but unverified approval has no current authority"
    );
    let first_retry = reconciler
        .execute(issue.clone())
        .expect("fresh exact retry reconciles the committed command");
    let second_retry = runtime_repository(target)
        .execute(issue)
        .expect("second fresh exact retry");
    assert_eq!(first_retry, second_retry);
    println!("TASK024_COMMIT_RESPONSE_UNCERTAINTY_PASS");
}

fn commit_response_drop_proxy(postgres_port: u16) -> (u16, JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("loopback uncertainty proxy");
    let proxy_port = listener.local_addr().expect("proxy address").port();
    let handle = thread::spawn(move || {
        let (client, _) = listener.accept().expect("proxied client");
        println!("TASK024_COMMIT_PROXY_ACCEPTED");
        let server = TcpStream::connect(("127.0.0.1", postgres_port)).expect("fixture upstream");
        println!("TASK024_COMMIT_PROXY_UPSTREAM_CONNECTED");
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("client proxy timeout");
        server
            .set_read_timeout(Some(Duration::from_secs(15)))
            .expect("server proxy timeout");
        let mut client_reader = client.try_clone().expect("client reader");
        let mut client_writer = client;
        let mut server_reader = server.try_clone().expect("server reader");
        let mut server_writer = server;
        let commit_seen = Arc::new(AtomicBool::new(false));
        let request_commit_seen = Arc::clone(&commit_seen);
        let request = thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            let mut tail = Vec::new();
            loop {
                let count = match client_reader.read(&mut buffer) {
                    Ok(count) => count,
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                        ) =>
                    {
                        if request_commit_seen.load(Ordering::Acquire) {
                            break;
                        }
                        continue;
                    }
                    Err(error) => panic!("read proxied request: {error}"),
                };
                if count == 0 {
                    break;
                }
                server_writer
                    .write_all(&buffer[..count])
                    .expect("forward proxied request");
                let mut inspection = tail;
                inspection.extend_from_slice(&buffer[..count]);
                if inspection.windows(7).any(|window| window == b"COMMIT\0") {
                    request_commit_seen.store(true, Ordering::Release);
                    println!("TASK024_COMMIT_PROXY_COMMIT_SEEN");
                }
                tail = inspection.into_iter().rev().take(6).collect::<Vec<_>>();
                tail.reverse();
            }
        });
        let response = thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                let count = server_reader
                    .read(&mut buffer)
                    .expect("read fixture response");
                if count == 0 {
                    break;
                }
                if commit_seen.load(Ordering::Acquire) {
                    println!("TASK024_COMMIT_PROXY_RESPONSE_DROPPED");
                    let _ = client_writer.shutdown(Shutdown::Both);
                    let _ = server_reader.shutdown(Shutdown::Both);
                    break;
                }
                client_writer
                    .write_all(&buffer[..count])
                    .expect("forward fixture response");
            }
        });
        request.join().expect("request proxy");
        response.join().expect("response proxy");
    });
    (proxy_port, handle)
}

fn exercise_committed_corruption_rejection(migrator: &mut Client, target: &ExtensionTarget) {
    let mut tamper = migrator.transaction().expect("tamper transaction");
    tamper
        .batch_execute("SET LOCAL ROLE lattice_migrator")
        .expect("tamper role");
    let original: Vec<u8> = tamper
        .query_one(
            "SELECT command_bytes FROM approval_verifier.approval_commands \
             WHERE command_id='issue-live-initial'",
            &[],
        )
        .expect("original physical command")
        .get(0);
    tamper
        .execute(
            "UPDATE approval_verifier.approval_commands \
             SET command_bytes=command_bytes || decode('00','hex'), \
                 command_bytes_sha256=sha256(command_bytes || decode('00','hex')) \
             WHERE command_id='issue-live-initial'",
            &[],
        )
        .expect("commit coherent physical tamper");
    tamper.commit().expect("commit physical tamper");

    assert_eq!(
        runtime_repository(target)
            .current_authority("approval-live-protected")
            .expect_err("physical/domain divergence must fail closed")
            .kind(),
        ApprovalRepositoryErrorKind::Corrupt
    );

    let mut restore = migrator.transaction().expect("restore transaction");
    restore
        .batch_execute("SET LOCAL ROLE lattice_migrator")
        .expect("restore role");
    restore
        .execute(
            "UPDATE approval_verifier.approval_commands \
             SET command_bytes=$1,command_bytes_sha256=sha256($1) \
             WHERE command_id='issue-live-initial'",
            &[&original],
        )
        .expect("restore fixture command");
    restore.commit().expect("commit fixture restoration");
    assert!(
        runtime_repository(target)
            .current_authority("approval-live-protected")
            .expect("restored replay")
            .is_some()
    );
    println!("TASK024_COMMITTED_CORRUPTION_REJECTED_PASS");
}

fn exercise_protected_denial(target: &ExtensionTarget) {
    let signer = protected_signer();
    let approval_id = "approval-live-protected";
    let issue = ApprovalRepositoryCommand::Issue(ApprovalIssueRequest {
        command_id: "issue-live-protected".to_owned(),
        expected_head: None,
        identity: protected_identity(&signer, approval_id, "challenge-live-protected"),
        nonce_id: "nonce-live-protected".to_owned(),
        nonce_commitment: nonce_commitment(
            &SecretMaterial::new(b"nonce-secret-protected".to_vec()).expect("nonce"),
        )
        .expect("commitment"),
        ttl_seconds: 300,
        authenticator_id: signer.authenticator_id().to_owned(),
        key_id: signer.key_id().to_owned(),
        verification_key_commitment: signer.trust_root_digest().clone(),
        evidence_digest: signer.evidence_digest().clone(),
        review_set_digest: Some(digest('6')),
    });
    let mut repository = runtime_repository(target);
    let issued = repository.execute(issue).expect("protected issue");
    let verified = repository
        .execute(ApprovalRepositoryCommand::Verify(ApprovalVerifyRequest {
            command_id: "verify-live-protected".to_owned(),
            approval_id: approval_id.to_owned(),
            expected_head: issued.after.clone().expect("protected challenged head"),
            proof: signer
                .sign(issued.challenge.as_ref().expect("protected challenge"))
                .expect("protected fake proof"),
        }))
        .expect("protected verify");
    let request = ApprovalNormalClaimRequest::new(
        "claim-live-protected",
        approval_id,
        verified.after.clone().expect("protected pending head"),
        ApprovalEffectClaimIntent::new("release-activation", "effect-live-protected", digest('9'))
            .expect("protected-shaped effect"),
    )
    .expect("protected claim request shape");
    let denied = repository
        .claim_normal(request.clone())
        .expect("protected terminal denial");
    let ApprovalNormalClaimExecution::Denied(receipt) = &denied else {
        panic!("protected lane must not create an effect receipt")
    };
    assert_eq!(
        receipt.outcome,
        ApprovalCommandOutcome::Denied(ApprovalDenial::NormalClaimRequired)
    );
    assert_eq!(
        repository
            .claim_normal(request)
            .expect("protected exact denial retry"),
        denied
    );
    assert!(
        repository
            .current_authority(approval_id)
            .expect("protected pending authority")
            .is_some()
    );
    println!("TASK024_PROTECTED_NORMAL_CLAIM_DENIED_PASS");
}

fn protected_signer() -> FakeProtectedSigner {
    FakeProtectedSigner::new(
        "guardian-1",
        "fake-guardian-authenticator",
        "fake-guardian-key",
        SecretMaterial::new(b"guardian-root".to_vec()).expect("guardian root"),
        "guardian-daemon-1",
        7,
    )
    .expect("protected signer")
}

fn protected_identity(
    signer: &FakeProtectedSigner,
    approval_id: &str,
    challenge_id: &str,
) -> ApprovalIdentity {
    let binding = SubjectBinding::new(
        ProjectId::new("lattice-system").expect("system project"),
        ProjectSnapshotId::new("snapshot-protected").expect("snapshot"),
        TaskId::new("task-release").expect("task"),
        "1",
        digest('d'),
    )
    .expect("protected binding");
    let release = ReleaseSubject::new(
        "activation-1",
        "saga-1",
        "release-1",
        "1",
        digest('e'),
        "commit-1",
        digest('f'),
        digest('1'),
        vec![digest('2')],
        vec![digest('3')],
        digest('4'),
        "source-release-1",
        digest('5'),
        "slot-a",
        "slot-b",
        DaemonEpoch::new(signer.observed_epoch()).expect("epoch"),
        true,
        UpgradeDelta::new(false, true, false, true, false, false, false, true),
    )
    .expect("release subject");
    let guardian = GuardianRuntimeSubject::new(
        signer.guardian_id(),
        signer.trust_root_digest().clone(),
        signer.daemon_instance_id(),
        DaemonEpoch::new(signer.observed_epoch()).expect("guardian epoch"),
    )
    .expect("guardian subject");
    ApprovalIdentity::new(
        approval_id,
        challenge_id,
        binding,
        ApprovalSubject::ProtectedRelease(Box::new(ProtectedReleaseSubject::new(
            release, guardian,
        ))),
        "requester-protected",
        signer.guardian_id(),
        ApprovalAuthority::ProtectedGuardian,
        ApprovalOrigin::GuardianTrustRoot,
        ApprovalLane::Protected,
        "channel-protected",
        "session-protected",
    )
    .expect("protected identity")
}

fn exercise_concurrent_claimers(target: &ExtensionTarget) {
    let approval_id = "approval-live-concurrent";
    let signer = normal_signer();
    let issue = ApprovalRepositoryCommand::Issue(ApprovalIssueRequest {
        command_id: "issue-live-concurrent".to_owned(),
        expected_head: None,
        identity: normal_identity(approval_id, "challenge-live-concurrent"),
        nonce_id: "nonce-live-concurrent".to_owned(),
        nonce_commitment: nonce_commitment(
            &SecretMaterial::new(b"nonce-secret-concurrent".to_vec()).expect("nonce"),
        )
        .expect("commitment"),
        ttl_seconds: 300,
        authenticator_id: signer.authenticator_id().to_owned(),
        key_id: signer.key_id().to_owned(),
        verification_key_commitment: signer.verification_key_commitment().clone(),
        evidence_digest: signer.evidence_digest().clone(),
        review_set_digest: None,
    });
    let mut repository = runtime_repository(target);
    let issued = repository.execute(issue).expect("concurrent issue");
    let verified = repository
        .execute(ApprovalRepositoryCommand::Verify(ApprovalVerifyRequest {
            command_id: "verify-live-concurrent".to_owned(),
            approval_id: approval_id.to_owned(),
            expected_head: issued.after.clone().expect("challenged head"),
            proof: signer
                .sign(issued.challenge.as_ref().expect("challenge"))
                .expect("proof"),
        }))
        .expect("concurrent verify");
    drop(repository);
    let expected_head = verified.after.expect("verified head");
    let first = ApprovalNormalClaimRequest::new(
        "claim-live-concurrent-a",
        approval_id,
        expected_head.clone(),
        ApprovalEffectClaimIntent::new("task-transition", "effect-concurrent-a", digest('7'))
            .expect("effect a"),
    )
    .expect("claim a");
    let second = ApprovalNormalClaimRequest::new(
        "claim-live-concurrent-b",
        approval_id,
        expected_head,
        ApprovalEffectClaimIntent::new("task-transition", "effect-concurrent-b", digest('8'))
            .expect("effect b"),
    )
    .expect("claim b");
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for request in [first, second] {
        let barrier = Arc::clone(&barrier);
        let target = target.clone();
        handles.push(std::thread::spawn(move || {
            let mut repository = runtime_repository(&target);
            barrier.wait();
            repository.claim_normal(request)
        }));
    }
    let executions: Vec<ApprovalNormalClaimExecution> = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .expect("claim thread")
                .expect("serialized claim")
        })
        .collect();
    assert_eq!(
        executions
            .iter()
            .filter(|execution| matches!(execution, ApprovalNormalClaimExecution::Claimed(_)))
            .count(),
        1
    );
    assert_eq!(
        executions
            .iter()
            .filter(|execution| matches!(
                execution,
                ApprovalNormalClaimExecution::Denied(receipt)
                    if receipt.outcome
                        == ApprovalCommandOutcome::Denied(ApprovalDenial::StaleHead)
            ))
            .count(),
        1
    );
    println!("TASK024_CONCURRENT_CLAIMERS_PASS");
}

fn exercise_repository(target: &ExtensionTarget, suffix: &str) {
    let approval_id = format!("approval-live-{suffix}");
    let verify_command_id = format!("verify-live-{suffix}");
    let claim_command_id = format!("claim-live-{suffix}");
    let signer = normal_signer();
    let issue = normal_issue(suffix);
    let mut repository = runtime_repository(target);
    println!("TASK024_REPOSITORY_ENTER_ISSUE");
    let issued = repository.execute(issue.clone()).unwrap_or_else(|error| {
        println!("TASK024_REPOSITORY_ISSUE_ERROR_{}", error.code());
        panic!("durable issue failed")
    });
    println!("TASK024_REPOSITORY_PASS_ISSUE");
    assert_eq!(issued.outcome, ApprovalCommandOutcome::Applied);
    assert_eq!(
        repository.execute(issue).expect("exact issue retry"),
        issued
    );
    let challenge = issued.challenge.as_ref().expect("durable challenge");
    println!("TASK024_REPOSITORY_ENTER_VERIFY");
    let verified = repository
        .execute(ApprovalRepositoryCommand::Verify(ApprovalVerifyRequest {
            command_id: verify_command_id,
            approval_id: approval_id.clone(),
            expected_head: issued.after.clone().expect("challenged head"),
            proof: signer.sign(challenge).expect("fake proof"),
        }))
        .unwrap_or_else(|error| {
            println!("TASK024_REPOSITORY_VERIFY_ERROR_{}", error.code());
            panic!("durable verify failed")
        });
    println!("TASK024_REPOSITORY_PASS_VERIFY");
    assert_eq!(verified.outcome, ApprovalCommandOutcome::Applied);
    assert!(
        repository
            .current_authority(&approval_id)
            .expect("current durable authority")
            .is_some()
    );
    let claim_request = ApprovalNormalClaimRequest::new(
        claim_command_id,
        approval_id.clone(),
        verified.after.clone().expect("verified head"),
        ApprovalEffectClaimIntent::new(
            "task-transition",
            format!("effect-live-{suffix}"),
            digest('e'),
        )
        .expect("effect intent"),
    )
    .expect("normal claim request");
    println!("TASK024_REPOSITORY_ENTER_CLAIM");
    let claimed = repository
        .claim_normal(claim_request.clone())
        .unwrap_or_else(|error| {
            println!("TASK024_REPOSITORY_CLAIM_ERROR_{}", error.code());
            panic!("durable normal claim failed")
        });
    println!("TASK024_REPOSITORY_PASS_CLAIM");
    let ApprovalNormalClaimExecution::Claimed(receipt) = &claimed else {
        panic!("normal approval must atomically claim the effect")
    };
    assert_eq!(receipt.request(), &claim_request);
    assert_eq!(
        repository
            .claim_normal(claim_request.clone())
            .expect("exact claim retry"),
        claimed
    );
    let changed = ApprovalNormalClaimRequest::new(
        claim_request.command_id(),
        approval_id.clone(),
        claim_request.expected_head().clone(),
        ApprovalEffectClaimIntent::new(
            "task-transition",
            format!("effect-live-{suffix}-changed"),
            digest('f'),
        )
        .expect("changed effect"),
    )
    .expect("changed claim shape");
    assert_eq!(
        repository
            .claim_normal(changed)
            .expect_err("changed exact retry must fail closed")
            .kind(),
        ApprovalRepositoryErrorKind::Domain
    );
    assert!(
        repository
            .current_authority(&approval_id)
            .expect("claimed currentness")
            .is_none()
    );
}

fn normal_issue(suffix: &str) -> ApprovalRepositoryCommand {
    let signer = normal_signer();
    ApprovalRepositoryCommand::Issue(ApprovalIssueRequest {
        command_id: format!("issue-live-{suffix}"),
        expected_head: None,
        identity: normal_identity(
            &format!("approval-live-{suffix}"),
            &format!("challenge-live-{suffix}"),
        ),
        nonce_id: format!("nonce-live-{suffix}"),
        nonce_commitment: nonce_commitment(
            &SecretMaterial::new(format!("nonce-secret-{suffix}").into_bytes())
                .expect("ephemeral nonce"),
        )
        .expect("nonce commitment"),
        ttl_seconds: 300,
        authenticator_id: signer.authenticator_id().to_owned(),
        key_id: signer.key_id().to_owned(),
        verification_key_commitment: signer.verification_key_commitment().clone(),
        evidence_digest: signer.evidence_digest().clone(),
        review_set_digest: None,
    })
}

fn runtime_repository(target: &ExtensionTarget) -> PostgresApprovalVerifier {
    let client = Client::connect(&required("LATTICE_APPROVAL_RUNTIME_URL"), NoTls)
        .expect("runtime connection");
    PostgresApprovalVerifier::new(client, target.clone()).expect("runtime adapter")
}

fn normal_signer() -> FakeNormalSigner {
    FakeNormalSigner::new(
        "approver-1",
        "fake-os-authenticator",
        "fake-key-1",
        SecretMaterial::new(b"fake-key-secret-1".to_vec()).expect("fake signer secret"),
    )
    .expect("fake signer")
}

fn normal_identity(approval_id: &str, challenge_id: &str) -> ApprovalIdentity {
    let binding = SubjectBinding::new(
        ProjectId::new("project-task024").expect("project"),
        ProjectSnapshotId::new("snapshot-task024").expect("snapshot"),
        TaskId::new("task-024").expect("task"),
        "1",
        digest('a'),
    )
    .expect("binding");
    ApprovalIdentity::new(
        approval_id,
        challenge_id,
        binding,
        ApprovalSubject::Execution {
            task_spec_hash: digest('a'),
            external_cost: None,
        },
        "requester-1",
        "approver-1",
        ApprovalAuthority::ResponsibleUser,
        ApprovalOrigin::OsAuthenticatedUser,
        ApprovalLane::Normal,
        "channel-task024",
        "session-task024",
    )
    .expect("normal identity")
}

fn assert_physical_shape(migrator: &mut Client, commands: i64, effects: i64) {
    migrator
        .batch_execute("SET ROLE lattice_migrator; SET search_path=pg_catalog;")
        .expect("migrator inspection role");
    let row = migrator
        .query_one(
            "SELECT (SELECT pg_catalog.count(*) FROM ONLY approval_verifier.approval_commands), \
                    (SELECT pg_catalog.count(*) FROM ONLY approval_verifier.approval_effect_claims), \
                    (SELECT command_high_water FROM ONLY approval_verifier.approval_heads \
                      WHERE singleton);",
            &[],
        )
        .expect("physical closure");
    assert_eq!(row.get::<_, i64>(0), commands);
    assert_eq!(row.get::<_, i64>(1), effects);
    assert_eq!(row.get::<_, i64>(2), commands);
}

fn assert_runtime_surface_is_closed() {
    let mut runtime = Client::connect(&required("LATTICE_APPROVAL_RUNTIME_URL"), NoTls)
        .unwrap_or_else(|_| {
            println!("TASK024_RUNTIME_CONNECT_ERROR");
            panic!("marker-owned runtime connection failed")
        });
    runtime
        .batch_execute("SET ROLE lattice_runtime; SET search_path=pg_catalog;")
        .unwrap_or_else(|_| {
            println!("TASK024_RUNTIME_ROLE_ERROR");
            panic!("marker-owned runtime role failed")
        });
    let schema_usage: bool = runtime
        .query_one(
            "SELECT pg_catalog.has_schema_privilege('approval_verifier','USAGE')",
            &[],
        )
        .and_then(|row| row.try_get(0))
        .unwrap_or_else(|_| {
            println!("TASK024_RUNTIME_SCHEMA_ACL_QUERY_ERROR");
            panic!("runtime schema ACL inspection failed")
        });
    let table_select: bool = runtime
        .query_one(
            "SELECT pg_catalog.has_table_privilege(c.oid,'SELECT') \
               FROM pg_catalog.pg_class c \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='approval_verifier' AND c.relname='approval_heads'",
            &[],
        )
        .and_then(|row| row.try_get(0))
        .unwrap_or_else(|_| {
            println!("TASK024_RUNTIME_TABLE_ACL_QUERY_ERROR");
            panic!("runtime table ACL inspection failed")
        });
    let function_count: i64 = runtime
        .query_one(
            "SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
             JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
             WHERE n.nspname='approval_verifier' \
               AND pg_catalog.has_function_privilege(p.oid,'EXECUTE')",
            &[],
        )
        .and_then(|row| row.try_get(0))
        .unwrap_or_else(|_| {
            println!("TASK024_RUNTIME_FUNCTION_ACL_QUERY_ERROR");
            panic!("runtime function ACL inspection failed")
        });
    if !schema_usage {
        println!("TASK024_RUNTIME_SCHEMA_USAGE_MISSING");
        panic!("runtime schema usage missing")
    }
    if table_select {
        println!("TASK024_RUNTIME_TABLE_SELECT_UNEXPECTED");
        panic!("runtime table select unexpectedly granted")
    }
    if function_count != 5 {
        println!("TASK024_RUNTIME_FUNCTION_COUNT_REJECTED");
        panic!("runtime fixed-function allowlist mismatch")
    }
    if runtime
        .query("SELECT * FROM approval_verifier.approval_heads", &[])
        .is_ok()
    {
        println!("TASK024_RUNTIME_DIRECT_TABLE_ACCESS_UNEXPECTED");
        panic!("runtime direct table access was not closed")
    }
    for query in [
        "SELECT * FROM approval_verifier.approval_verifier_load_commands_v1()",
        "SELECT * FROM approval_verifier.approval_verifier_load_effects_v1()",
    ] {
        if runtime.query(query, &[]).is_ok() {
            println!("TASK024_RUNTIME_TRANSACTION_BYPASS_UNEXPECTED");
            panic!("runtime fixed function accepted a read-committed caller")
        }
    }
    println!("TASK024_RUNTIME_SURFACE_CLOSED");
}

fn apply_or_report(
    client: &mut Client,
    target: &ExtensionTarget,
    stage: &str,
) -> ExtensionApplyOutcome {
    apply_extension(client, target).unwrap_or_else(|error| {
        println!("TASK024_{stage}_ERROR_{}", error.code());
        panic!("closed Approval extension setup failure")
    })
}

fn verify_or_report(client: &mut Client, target: &ExtensionTarget, stage: &str) {
    verify_extension(client, target).unwrap_or_else(|error| {
        println!("TASK024_{stage}_ERROR_{}", error.code());
        panic!("closed Approval extension verification failure")
    });
}

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing marker-owned environment: {name}"))
}

fn env_digest(name: &str) -> ContentDigest {
    ContentDigest::from_sha256(required(name)).expect("valid fixture digest")
}

fn digest(character: char) -> ContentDigest {
    ContentDigest::from_sha256(character.to_string().repeat(64)).expect("test digest")
}
