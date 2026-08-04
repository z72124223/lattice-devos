use lattice_contracts::{
    AttemptId, ContentDigest, GatewayAction, GatewayActorId, GatewayActorKind, GatewayAdapterId,
    GatewayApprovalDisposition, GatewayApprovalId, GatewayApprovalRoute, GatewayChallengeId,
    GatewayChannelId, GatewayClientKind, GatewayCommandId, GatewayCorrelationId, GatewayDenialCode,
    GatewayInstanceId, GatewayNormalApprovalKind, GatewayPeerContext, GatewayProjectStatusTarget,
    GatewayReply, GatewayReplyBody, GatewayRequest, GatewayRequestBody, GatewaySessionId,
    GatewayStatusObservation, GatewayStatusTarget, GatewayStopDisposition, GatewayStopReason,
    GatewayStopTarget, GatewayTaskProjection, GatewayTaskState, GatewayTaskTarget,
    GatewayUnknownCode, ProjectId, ProjectSnapshotId, SubjectBinding, TaskId, TaskSpecSubmission,
};
use lattice_gateway_ipc::{
    FakeFault, FakeGatewayClient, FakeGatewayServer, LoopbackErrorKind, MAX_REPLAY_ENTRIES,
    build_reply, build_request, decode_reply, decode_request, encode_reply, encode_request,
    task_spec_document_digest,
};
use lattice_ports::{GatewayService, GatewayServiceError, GatewayServiceResult, PortErrorKind};

fn digest(fill: char) -> ContentDigest {
    ContentDigest::from_sha256(fill.to_string().repeat(64)).unwrap()
}

fn submit_body(project: &str, task: &str) -> GatewayRequestBody {
    let document = format!(
        "{{\"project_id\":\"{project}\",\"project_snapshot_id\":\"snapshot-a\",\"revision\":\"1\",\"schema_version\":\"2.1\",\"task_id\":\"{task}\"}}"
    );
    let spec_digest = task_spec_document_digest(document.as_bytes()).unwrap();
    let binding = SubjectBinding::new(
        ProjectId::new(project).unwrap(),
        ProjectSnapshotId::new("snapshot-a").unwrap(),
        TaskId::new(task).unwrap(),
        "1",
        spec_digest.clone(),
    )
    .unwrap();
    GatewayRequestBody::Submit(
        TaskSpecSubmission::new(binding, document.into_bytes(), spec_digest).unwrap(),
    )
}

fn task_target(project: &str, task: &str) -> GatewayTaskTarget {
    GatewayTaskTarget::new(
        SubjectBinding::new(
            ProjectId::new(project).unwrap(),
            ProjectSnapshotId::new("snapshot-a").unwrap(),
            TaskId::new(task).unwrap(),
            "1",
            digest('b'),
        )
        .unwrap(),
        digest('c'),
    )
    .unwrap()
}

fn request(command: &str, body: GatewayRequestBody) -> GatewayRequest {
    build_request(
        GatewayCommandId::new(command).unwrap(),
        GatewayCorrelationId::new(format!("correlation-{command}")).unwrap(),
        body,
    )
    .unwrap()
}

fn peer(kind: GatewayClientKind) -> GatewayPeerContext {
    peer_with_actor(kind, "actor-a")
}

fn peer_with_actor(kind: GatewayClientKind, actor: &str) -> GatewayPeerContext {
    let actor_kind = match kind {
        GatewayClientKind::OpenClaw => GatewayActorKind::ResponsibleUser,
        GatewayClientKind::RecoveryCli => GatewayActorKind::RecoveryOperator,
        GatewayClientKind::TestFake => GatewayActorKind::TestFixture,
    };
    GatewayPeerContext::new_fake(
        kind,
        GatewayInstanceId::new("gateway-a").unwrap(),
        GatewayAdapterId::new("adapter-a").unwrap(),
        "1.0",
        digest('d'),
        digest('e'),
        GatewayActorId::new(actor).unwrap(),
        actor_kind,
        GatewayChannelId::new("channel-a").unwrap(),
        GatewaySessionId::new("session-a").unwrap(),
        1,
        digest('f'),
        digest('f'),
    )
    .unwrap()
}

#[test]
fn six_request_variants_round_trip_with_action_derived_from_body() {
    let target = task_target("project-a", "task-a");
    let approval = GatewayApprovalRoute::new(
        target.binding().clone(),
        GatewayNormalApprovalKind::Execution,
        GatewayApprovalId::new("approval-a").unwrap(),
        GatewayChallengeId::new("challenge-a").unwrap(),
        digest('1'),
        digest('2'),
        digest('3'),
    )
    .unwrap();
    let bodies = vec![
        submit_body("project-a", "task-a"),
        GatewayRequestBody::Plan(target.clone()),
        GatewayRequestBody::Status(GatewayStatusTarget::Project(
            GatewayProjectStatusTarget::new(ProjectId::new("project-a").unwrap(), 10, None)
                .unwrap(),
        )),
        GatewayRequestBody::Approve(approval.clone()),
        GatewayRequestBody::Reject(approval),
        GatewayRequestBody::Stop(
            GatewayStopTarget::new(
                target,
                AttemptId::new("attempt-a").unwrap(),
                GatewayStopReason::UserRequested,
            )
            .unwrap(),
        ),
    ];

    for (index, body) in bodies.into_iter().enumerate() {
        let original = request(&format!("command-{index}"), body);
        let bytes = encode_request(&original).unwrap();
        let decoded = decode_request(&bytes).unwrap();
        assert_eq!(decoded, original);
        assert_eq!(encode_request(&decoded).unwrap(), bytes);
        assert_eq!(decoded.action(), GatewayAction::ALL[index]);
    }
}

fn assert_reply_round_trip(request: &GatewayRequest, body: GatewayReplyBody) {
    let reply = build_reply(request, body).unwrap();
    let bytes = encode_reply(&reply).unwrap();
    assert_eq!(decode_reply(request, &bytes).unwrap(), reply);
    assert_eq!(encode_reply(&reply).unwrap(), bytes);
}

#[test]
fn submit_plan_and_status_reply_variants_round_trip() {
    let binding = task_target("project-a", "task-a").binding().clone();
    let submit = request("reply-submit", submit_body("project-a", "task-a"));
    let submit_binding = match submit.body() {
        GatewayRequestBody::Submit(submission) => submission.binding().clone(),
        _ => unreachable!("submit helper always constructs Submit"),
    };
    assert_reply_round_trip(
        &submit,
        GatewayReplyBody::SubmitAccepted {
            binding: submit_binding,
            command_receipt_digest: digest('4'),
        },
    );

    let plan = request(
        "reply-plan",
        GatewayRequestBody::Plan(task_target("project-a", "task-a")),
    );
    assert_reply_round_trip(
        &plan,
        GatewayReplyBody::PlanRouted {
            binding: binding.clone(),
            command_receipt_digest: digest('5'),
        },
    );

    let project = request(
        "reply-status-project",
        GatewayRequestBody::Status(GatewayStatusTarget::Project(
            GatewayProjectStatusTarget::new(ProjectId::new("project-a").unwrap(), 1, None).unwrap(),
        )),
    );
    assert_reply_round_trip(
        &project,
        GatewayReplyBody::StatusObserved(GatewayStatusObservation::Project {
            project_id: ProjectId::new("project-a").unwrap(),
            tasks: vec![
                GatewayTaskProjection::new(
                    binding.clone(),
                    GatewayTaskState::Draft,
                    digest('6'),
                    digest('7'),
                )
                .unwrap(),
            ],
            next_cursor: Some("cursor-a".to_owned()),
        }),
    );

    let task = request(
        "reply-status-task",
        GatewayRequestBody::Status(GatewayStatusTarget::Task(task_target(
            "project-a",
            "task-a",
        ))),
    );
    assert_reply_round_trip(
        &task,
        GatewayReplyBody::StatusObserved(GatewayStatusObservation::Task(
            GatewayTaskProjection::new(
                binding,
                GatewayTaskState::Executing,
                digest('8'),
                digest('9'),
            )
            .unwrap(),
        )),
    );

    let command = request(
        "reply-status-command",
        GatewayRequestBody::Status(GatewayStatusTarget::Command {
            project_id: ProjectId::new("project-a").unwrap(),
            original_command_id: GatewayCommandId::new("original-command").unwrap(),
        }),
    );
    assert_reply_round_trip(
        &command,
        GatewayReplyBody::StatusObserved(GatewayStatusObservation::Command {
            project_id: ProjectId::new("project-a").unwrap(),
            original_command_id: GatewayCommandId::new("original-command").unwrap(),
            terminal_reply_digest: digest('a'),
        }),
    );
}

#[test]
fn approval_denial_and_unknown_reply_variants_round_trip() {
    let binding = task_target("project-a", "task-a").binding().clone();
    let approval = GatewayApprovalRoute::new(
        binding.clone(),
        GatewayNormalApprovalKind::Execution,
        GatewayApprovalId::new("reply-approval").unwrap(),
        GatewayChallengeId::new("reply-challenge").unwrap(),
        digest('1'),
        digest('2'),
        digest('3'),
    )
    .unwrap();
    for (action, disposition) in [
        (
            GatewayAction::Approve,
            GatewayApprovalDisposition::RoutedForVerification,
        ),
        (
            GatewayAction::Reject,
            GatewayApprovalDisposition::RejectionRecorded,
        ),
    ] {
        let body = if action == GatewayAction::Approve {
            GatewayRequestBody::Approve(approval.clone())
        } else {
            GatewayRequestBody::Reject(approval.clone())
        };
        let request = request(&format!("reply-{}", action.as_str()), body);
        assert_reply_round_trip(
            &request,
            GatewayReplyBody::ApprovalRouted {
                binding: binding.clone(),
                approval_id: approval.approval_id().clone(),
                challenge_id: approval.challenge_id().clone(),
                challenge_digest: approval.challenge_digest().clone(),
                disposition,
                routing_receipt_digest: digest('b'),
            },
        );
    }
    let submit = request("reply-terminal", submit_body("project-a", "task-a"));
    assert_reply_round_trip(
        &submit,
        GatewayReplyBody::Denied(GatewayDenialCode::ScopeDenied),
    );
    assert_reply_round_trip(
        &submit,
        GatewayReplyBody::UnknownOutcome(GatewayUnknownCode::ReconciliationRequired),
    );
}

#[test]
fn all_stop_dispositions_round_trip_without_claiming_completion() {
    let target = GatewayStopTarget::new(
        task_target("project-a", "task-a"),
        AttemptId::new("attempt-a").unwrap(),
        GatewayStopReason::UserRequested,
    )
    .unwrap();
    for (index, disposition) in [
        GatewayStopDisposition::Requested,
        GatewayStopDisposition::AlreadyTerminal,
        GatewayStopDisposition::ReconciliationRequired,
    ]
    .into_iter()
    .enumerate()
    {
        let request = request(
            &format!("reply-stop-{index}"),
            GatewayRequestBody::Stop(target.clone()),
        );
        assert_reply_round_trip(
            &request,
            GatewayReplyBody::StopRouted {
                target: target.clone(),
                disposition,
                routing_receipt_digest: digest('d'),
            },
        );
    }
}

#[test]
fn reply_digest_and_request_substitution_are_rejected() {
    let original = request("reply-binding", submit_body("project-a", "task-a"));
    let binding = match original.body() {
        GatewayRequestBody::Submit(submission) => submission.binding().clone(),
        _ => unreachable!("submit helper always constructs Submit"),
    };
    let reply = build_reply(
        &original,
        GatewayReplyBody::SubmitAccepted {
            binding,
            command_receipt_digest: digest('e'),
        },
    )
    .unwrap();
    let bytes = encode_reply(&reply).unwrap();
    let changed_request = request("reply-binding-other", submit_body("project-a", "task-a"));
    assert!(decode_reply(&changed_request, &bytes).is_err());

    let text = String::from_utf8(bytes).unwrap();
    let changed_digest = if reply.reply_digest().as_str() == digest('f').as_str() {
        digest('e')
    } else {
        digest('f')
    };
    let mutated = text.replace(reply.reply_digest().as_str(), changed_digest.as_str());
    assert!(decode_reply(&original, mutated.as_bytes()).is_err());
}

#[test]
fn unknown_missing_action_and_version_are_rejected_before_dispatch() {
    for raw in [
        br#"{"action":"submit","body":{},"command_id":"c","correlation_id":"r","protocol":"lattice-gateway-ipc","request_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","unknown":"x","version":"1"}"#.as_slice(),
        br#"{"action":"submit","body":{},"command_id":"c","correlation_id":"r","protocol":"lattice-gateway-ipc","request_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#.as_slice(),
        br#"{"action":"shell","body":{},"command_id":"c","correlation_id":"r","protocol":"lattice-gateway-ipc","request_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","version":"1"}"#.as_slice(),
        br#"{"action":"status","body":{},"command_id":"c","correlation_id":"r","protocol":"lattice-gateway-ipc","request_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","version":"2"}"#.as_slice(),
    ] {
        assert!(decode_request(raw).is_err());
    }
}

#[test]
fn raw_version_mismatch_is_not_collapsed_into_generic_codec_failure() {
    let raw = br#"{"action":"status","body":{},"command_id":"c","correlation_id":"r","protocol":"lattice-gateway-ipc","request_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","version":"2"}"#;
    let mut server = FakeGatewayServer::new(CountingService::default());
    let error = server
        .handle_frame(peer(GatewayClientKind::OpenClaw), raw)
        .unwrap_err();
    assert_eq!(error.kind(), LoopbackErrorKind::VersionMismatch);
}

#[test]
fn malformed_oversized_auth_claim_and_task_binding_fail_before_service() {
    let mut server = FakeGatewayServer::new(CountingService::default());
    for raw in [
        vec![0xff],
        vec![b'!'; lattice_gateway_ipc::MAX_FRAME_BYTES + 1],
    ] {
        assert_eq!(
            server
                .handle_frame(peer(GatewayClientKind::OpenClaw), &raw)
                .unwrap_err()
                .kind(),
            LoopbackErrorKind::Codec
        );
    }

    let submit = request("raw-adversarial", submit_body("project-a", "task-a"));
    let encoded = String::from_utf8(encode_request(&submit).unwrap()).unwrap();
    let auth_claim = encoded.replacen("\",\"body\"", "\",\"authenticated\":\"true\",\"body\"", 1);
    assert_eq!(
        server
            .handle_frame(peer(GatewayClientKind::OpenClaw), auth_claim.as_bytes(),)
            .unwrap_err()
            .kind(),
        LoopbackErrorKind::Codec
    );

    let changed_binding = encoded.replacen(
        "\"project_id\":\"project-a\"",
        "\"project_id\":\"project-b\"",
        1,
    );
    assert_eq!(
        server
            .handle_frame(
                peer(GatewayClientKind::OpenClaw),
                changed_binding.as_bytes(),
            )
            .unwrap_err()
            .kind(),
        LoopbackErrorKind::Codec
    );
    assert_eq!(server.service().calls, 0);
}

#[derive(Default)]
struct CountingService {
    calls: usize,
}

impl GatewayService for CountingService {
    fn handle(
        &mut self,
        _peer: GatewayPeerContext,
        request: GatewayRequest,
    ) -> GatewayServiceResult<GatewayReply> {
        self.calls += 1;
        let binding = match request.body() {
            GatewayRequestBody::Submit(value) => value.binding().clone(),
            _ => {
                return Err(GatewayServiceError::new(
                    PortErrorKind::Denied,
                    "TEST_ONLY_UNSUPPORTED",
                ));
            }
        };
        build_reply(
            &request,
            GatewayReplyBody::SubmitAccepted {
                binding,
                command_receipt_digest: digest('9'),
            },
        )
        .map_err(|error| GatewayServiceError::new(PortErrorKind::Malformed, error.code()))
    }
}

struct ErrorService {
    kind: PortErrorKind,
    calls: usize,
}

impl GatewayService for ErrorService {
    fn handle(
        &mut self,
        _peer: GatewayPeerContext,
        _request: GatewayRequest,
    ) -> GatewayServiceResult<GatewayReply> {
        self.calls += 1;
        Err(GatewayServiceError::new(
            self.kind,
            "TEST_ONLY_PORT_FAILURE",
        ))
    }
}

#[test]
fn exact_retry_is_cached_and_changed_content_is_denied_without_second_call() {
    let mut server = FakeGatewayServer::new(CountingService::default());
    let client = FakeGatewayClient::new(peer(GatewayClientKind::OpenClaw));
    let original = request("command-retry", submit_body("project-a", "task-a"));

    let first = client.send(&mut server, &original).unwrap();
    let second = client.send(&mut server, &original).unwrap();
    assert_eq!(first, second);
    assert_eq!(server.service().calls, 1);

    let substituted = request("command-retry", submit_body("project-a", "task-b"));
    let denial = client.send(&mut server, &substituted).unwrap();
    assert!(matches!(
        denial.body(),
        GatewayReplyBody::Denied(lattice_contracts::GatewayDenialCode::CommandSubstitution)
    ));
    assert_eq!(server.service().calls, 1);
}

#[test]
fn replay_capacity_rejects_new_commands_before_service_but_keeps_exact_retries() {
    let mut server = FakeGatewayServer::new(CountingService::default());
    let client = FakeGatewayClient::new(peer(GatewayClientKind::OpenClaw));
    let first = request("capacity-0", submit_body("project-a", "task-a"));
    let first_reply = client.send(&mut server, &first).unwrap();

    for index in 1..MAX_REPLAY_ENTRIES {
        client
            .send(
                &mut server,
                &request(
                    &format!("capacity-{index}"),
                    submit_body("project-a", "task-a"),
                ),
            )
            .unwrap();
    }
    assert_eq!(server.service().calls, MAX_REPLAY_ENTRIES);

    let overflow = request("capacity-overflow", submit_body("project-a", "task-a"));
    assert_eq!(
        client.send(&mut server, &overflow).unwrap_err().kind(),
        LoopbackErrorKind::Capacity
    );
    assert_eq!(server.service().calls, MAX_REPLAY_ENTRIES);

    assert_eq!(client.send(&mut server, &first).unwrap(), first_reply);
    assert_eq!(server.service().calls, MAX_REPLAY_ENTRIES);
}

#[test]
fn recovery_role_fails_before_service() {
    let mut server = FakeGatewayServer::new(CountingService::default());
    let recovery = FakeGatewayClient::new(peer(GatewayClientKind::RecoveryCli));
    let denial = recovery
        .send(
            &mut server,
            &request("command-role", submit_body("project-a", "task-a")),
        )
        .unwrap();
    assert!(matches!(
        denial.body(),
        GatewayReplyBody::Denied(lattice_contracts::GatewayDenialCode::RoleDenied)
    ));
    assert_eq!(server.service().calls, 0);
}

#[test]
fn recovery_role_matrix_allows_only_status_and_stop() {
    let target = task_target("project-a", "task-a");
    let approval = GatewayApprovalRoute::new(
        target.binding().clone(),
        GatewayNormalApprovalKind::Execution,
        GatewayApprovalId::new("approval-role").unwrap(),
        GatewayChallengeId::new("challenge-role").unwrap(),
        digest('1'),
        digest('2'),
        digest('3'),
    )
    .unwrap();
    let denied = [
        submit_body("project-a", "task-a"),
        GatewayRequestBody::Plan(target.clone()),
        GatewayRequestBody::Approve(approval.clone()),
        GatewayRequestBody::Reject(approval),
    ];
    let recovery = FakeGatewayClient::new(peer(GatewayClientKind::RecoveryCli));
    let mut server = FakeGatewayServer::new(CountingService::default());
    for (index, body) in denied.into_iter().enumerate() {
        assert!(matches!(
            recovery
                .send(&mut server, &request(&format!("role-denied-{index}"), body))
                .unwrap()
                .body(),
            GatewayReplyBody::Denied(GatewayDenialCode::RoleDenied)
        ));
    }
    assert_eq!(server.service().calls, 0);

    let allowed = [
        GatewayRequestBody::Status(GatewayStatusTarget::Task(target.clone())),
        GatewayRequestBody::Stop(
            GatewayStopTarget::new(
                target,
                AttemptId::new("attempt-role").unwrap(),
                GatewayStopReason::SafetyConcern,
            )
            .unwrap(),
        ),
    ];
    for (index, body) in allowed.into_iter().enumerate() {
        assert!(matches!(
            recovery
                .send(
                    &mut server,
                    &request(&format!("role-allowed-{index}"), body)
                )
                .unwrap()
                .body(),
            GatewayReplyBody::Denied(GatewayDenialCode::DownstreamDenied)
        ));
    }
    assert_eq!(server.service().calls, 2);
}

#[test]
fn replay_scope_is_independent_across_actor_and_project() {
    let mut server = FakeGatewayServer::new(CountingService::default());
    let actor_a = FakeGatewayClient::new(peer_with_actor(GatewayClientKind::OpenClaw, "actor-a"));
    let actor_b = FakeGatewayClient::new(peer_with_actor(GatewayClientKind::OpenClaw, "actor-b"));
    let project_a = request("scoped-command", submit_body("project-a", "task-a"));
    actor_a.send(&mut server, &project_a).unwrap();
    actor_b.send(&mut server, &project_a).unwrap();
    actor_a
        .send(
            &mut server,
            &request("scoped-command", submit_body("project-b", "task-a")),
        )
        .unwrap();
    assert_eq!(server.service().calls, 3);
}

#[test]
fn every_downstream_port_error_has_an_explicit_gateway_outcome() {
    let command = request("port-error", submit_body("project-a", "task-a"));
    let client = FakeGatewayClient::new(peer(GatewayClientKind::OpenClaw));
    for (kind, expected) in [
        (PortErrorKind::Unavailable, LoopbackErrorKind::Unavailable),
        (
            PortErrorKind::VersionMismatch,
            LoopbackErrorKind::VersionMismatch,
        ),
        (
            PortErrorKind::CapabilityMismatch,
            LoopbackErrorKind::Malformed,
        ),
        (PortErrorKind::Malformed, LoopbackErrorKind::Malformed),
        (PortErrorKind::Timeout, LoopbackErrorKind::Timeout),
        (PortErrorKind::Cancelled, LoopbackErrorKind::Cancelled),
    ] {
        let mut server = FakeGatewayServer::new(ErrorService { kind, calls: 0 });
        assert_eq!(
            client.send(&mut server, &command).unwrap_err().kind(),
            expected
        );
        assert_eq!(server.service().calls, 1);
    }

    let mut denied = FakeGatewayServer::new(ErrorService {
        kind: PortErrorKind::Denied,
        calls: 0,
    });
    assert!(matches!(
        client.send(&mut denied, &command).unwrap().body(),
        GatewayReplyBody::Denied(GatewayDenialCode::DownstreamDenied)
    ));
    let mut ambiguous = FakeGatewayServer::new(ErrorService {
        kind: PortErrorKind::Ambiguous,
        calls: 0,
    });
    assert!(matches!(
        client.send(&mut ambiguous, &command).unwrap().body(),
        GatewayReplyBody::UnknownOutcome(GatewayUnknownCode::DownstreamAmbiguous)
    ));
}

#[test]
fn protected_change_routes_normally_while_protected_release_is_unrepresentable() {
    let target = task_target("project-a", "task-a");
    let protected_change = request(
        "command-protected-change",
        GatewayRequestBody::Approve(
            GatewayApprovalRoute::new(
                target.binding().clone(),
                GatewayNormalApprovalKind::ProtectedChange,
                GatewayApprovalId::new("approval-protected").unwrap(),
                GatewayChallengeId::new("challenge-protected").unwrap(),
                digest('1'),
                digest('2'),
                digest('3'),
            )
            .unwrap(),
        ),
    );
    let client = FakeGatewayClient::new(peer(GatewayClientKind::OpenClaw));
    let mut server = FakeGatewayServer::new(CountingService::default());
    assert!(matches!(
        client.send(&mut server, &protected_change).unwrap().body(),
        GatewayReplyBody::Denied(lattice_contracts::GatewayDenialCode::DownstreamDenied)
    ));
    assert_eq!(server.service().calls, 1);

    let encoded = String::from_utf8(encode_request(&protected_change).unwrap()).unwrap();
    let protected_release = encoded.replace("PROTECTED_CHANGE", "PROTECTED_RELEASE");
    let error = server
        .handle_frame(
            peer(GatewayClientKind::OpenClaw),
            protected_release.as_bytes(),
        )
        .unwrap_err();
    assert_eq!(error.kind(), LoopbackErrorKind::Codec);
    assert_eq!(server.service().calls, 1);
}

#[test]
fn authorization_precedes_replay_and_role_denials_do_not_poison_allowed_peers() {
    let command = request("command-cross-role", submit_body("project-a", "task-a"));

    let mut success_first = FakeGatewayServer::new(CountingService::default());
    let openclaw = FakeGatewayClient::new(peer(GatewayClientKind::OpenClaw));
    let recovery = FakeGatewayClient::new(peer(GatewayClientKind::RecoveryCli));
    assert!(matches!(
        openclaw.send(&mut success_first, &command).unwrap().body(),
        GatewayReplyBody::SubmitAccepted { .. }
    ));
    assert!(matches!(
        recovery.send(&mut success_first, &command).unwrap().body(),
        GatewayReplyBody::Denied(lattice_contracts::GatewayDenialCode::RoleDenied)
    ));
    assert_eq!(success_first.service().calls, 1);

    let mut denial_first = FakeGatewayServer::new(CountingService::default());
    assert!(matches!(
        recovery.send(&mut denial_first, &command).unwrap().body(),
        GatewayReplyBody::Denied(lattice_contracts::GatewayDenialCode::RoleDenied)
    ));
    assert!(matches!(
        openclaw.send(&mut denial_first, &command).unwrap().body(),
        GatewayReplyBody::SubmitAccepted { .. }
    ));
    assert_eq!(denial_first.service().calls, 1);
}

#[test]
fn zero_digest_and_oversized_subjects_fail_before_dispatch() {
    let binding = task_target("project-a", "task-a").binding().clone();
    assert!(GatewayTaskTarget::new(binding.clone(), digest('0')).is_err());

    assert!(
        GatewayApprovalRoute::new(
            binding.clone(),
            GatewayNormalApprovalKind::Execution,
            GatewayApprovalId::new("approval-zero").unwrap(),
            GatewayChallengeId::new("challenge-zero").unwrap(),
            digest('0'),
            digest('2'),
            digest('3'),
        )
        .is_err()
    );

    let long_binding = SubjectBinding::new(
        ProjectId::new("project-a").unwrap(),
        ProjectSnapshotId::new("s".repeat(257)).unwrap(),
        TaskId::new("t".repeat(257)).unwrap(),
        "1",
        digest('a'),
    )
    .unwrap();
    assert!(GatewayTaskTarget::new(long_binding, digest('b')).is_err());

    assert!(
        GatewayStopTarget::new(
            GatewayTaskTarget::new(binding, digest('b')).unwrap(),
            AttemptId::new("a".repeat(257)).unwrap(),
            GatewayStopReason::UserRequested,
        )
        .is_err()
    );
}

#[test]
fn non_nfc_reused_identifiers_fail_before_hash_encode_or_dispatch() {
    let non_nfc = "e\u{301}";
    let binding = |snapshot: &str, task: &str| {
        SubjectBinding::new(
            ProjectId::new("project-a").unwrap(),
            ProjectSnapshotId::new(snapshot).unwrap(),
            TaskId::new(task).unwrap(),
            "1",
            digest('a'),
        )
        .unwrap()
    };
    let plan = |command: &str, target| {
        build_request(
            GatewayCommandId::new(command).unwrap(),
            GatewayCorrelationId::new(format!("correlation-{command}")).unwrap(),
            GatewayRequestBody::Plan(target),
        )
        .unwrap_err()
        .kind()
    };

    assert_eq!(
        plan(
            "non-nfc-task",
            GatewayTaskTarget::new(binding("snapshot-a", non_nfc), digest('b')).unwrap(),
        ),
        lattice_gateway_ipc::CodecErrorKind::NonCanonical
    );
    assert_eq!(
        plan(
            "non-nfc-snapshot",
            GatewayTaskTarget::new(binding(non_nfc, "task-a"), digest('b')).unwrap(),
        ),
        lattice_gateway_ipc::CodecErrorKind::NonCanonical
    );

    let stop = GatewayStopTarget::new(
        GatewayTaskTarget::new(binding("snapshot-a", "task-a"), digest('b')).unwrap(),
        AttemptId::new(non_nfc).unwrap(),
        GatewayStopReason::UserRequested,
    )
    .unwrap();
    assert_eq!(
        build_request(
            GatewayCommandId::new("non-nfc-attempt").unwrap(),
            GatewayCorrelationId::new("correlation-non-nfc-attempt").unwrap(),
            GatewayRequestBody::Stop(stop),
        )
        .unwrap_err()
        .kind(),
        lattice_gateway_ipc::CodecErrorKind::NonCanonical
    );

    let expanding_256 = "\u{0344}".repeat(128);
    assert_eq!(expanding_256.len(), 256);
    assert_eq!(
        plan(
            "non-nfc-expanding-bound",
            GatewayTaskTarget::new(binding("snapshot-a", &expanding_256), digest('b')).unwrap(),
        ),
        lattice_gateway_ipc::CodecErrorKind::NonCanonical
    );

    let status = request(
        "non-nfc-project-reply",
        GatewayRequestBody::Status(GatewayStatusTarget::Project(
            GatewayProjectStatusTarget::new(ProjectId::new("project-a").unwrap(), 1, None).unwrap(),
        )),
    );
    let non_nfc_projection = GatewayTaskProjection::new(
        binding("snapshot-a", non_nfc),
        GatewayTaskState::Draft,
        digest('4'),
        digest('5'),
    )
    .unwrap();
    assert_eq!(
        build_reply(
            &status,
            GatewayReplyBody::StatusObserved(GatewayStatusObservation::Project {
                project_id: ProjectId::new("project-a").unwrap(),
                tasks: vec![non_nfc_projection],
                next_cursor: None,
            }),
        )
        .unwrap_err()
        .kind(),
        lattice_gateway_ipc::CodecErrorKind::NonCanonical
    );
}

#[test]
fn reply_receipts_and_project_pages_are_validated_before_hashing() {
    let plan = request(
        "command-zero-receipt",
        GatewayRequestBody::Plan(task_target("project-a", "task-a")),
    );
    assert!(
        build_reply(
            &plan,
            GatewayReplyBody::PlanRouted {
                binding: task_target("project-a", "task-a").binding().clone(),
                command_receipt_digest: digest('0'),
            },
        )
        .is_err()
    );

    let status = request(
        "command-small-page",
        GatewayRequestBody::Status(GatewayStatusTarget::Project(
            GatewayProjectStatusTarget::new(ProjectId::new("project-a").unwrap(), 1, None).unwrap(),
        )),
    );
    let tasks = ["task-a", "task-b"]
        .into_iter()
        .map(|task| {
            GatewayTaskProjection::new(
                task_target("project-a", task).binding().clone(),
                GatewayTaskState::Draft,
                digest('4'),
                digest('5'),
            )
            .unwrap()
        })
        .collect();
    assert!(
        build_reply(
            &status,
            GatewayReplyBody::StatusObserved(GatewayStatusObservation::Project {
                project_id: ProjectId::new("project-a").unwrap(),
                tasks,
                next_cursor: None,
            }),
        )
        .is_err()
    );
}

fn project_status_tasks(count: usize) -> Vec<GatewayTaskProjection> {
    (0..count)
        .map(|index| {
            GatewayTaskProjection::new(
                task_target("project-a", &format!("task-{index}"))
                    .binding()
                    .clone(),
                GatewayTaskState::Draft,
                digest('4'),
                digest('5'),
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn project_status_page_limits_accept_exact_edges_and_reject_plus_one() {
    for page_size in [1, 100] {
        let status = request(
            &format!("page-exact-{page_size}"),
            GatewayRequestBody::Status(GatewayStatusTarget::Project(
                GatewayProjectStatusTarget::new(
                    ProjectId::new("project-a").unwrap(),
                    page_size,
                    None,
                )
                .unwrap(),
            )),
        );
        assert!(
            build_reply(
                &status,
                GatewayReplyBody::StatusObserved(GatewayStatusObservation::Project {
                    project_id: ProjectId::new("project-a").unwrap(),
                    tasks: project_status_tasks(usize::from(page_size)),
                    next_cursor: None,
                }),
            )
            .is_ok()
        );
    }

    let status = request(
        "page-global-plus-one",
        GatewayRequestBody::Status(GatewayStatusTarget::Project(
            GatewayProjectStatusTarget::new(ProjectId::new("project-a").unwrap(), 100, None)
                .unwrap(),
        )),
    );
    assert!(
        build_reply(
            &status,
            GatewayReplyBody::StatusObserved(GatewayStatusObservation::Project {
                project_id: ProjectId::new("project-a").unwrap(),
                tasks: project_status_tasks(101),
                next_cursor: None,
            }),
        )
        .is_err()
    );
}

#[test]
fn reply_codec_and_fault_classes_stay_distinct() {
    let request = request("command-fault", submit_body("project-a", "task-a"));
    let mut server = FakeGatewayServer::new(CountingService::default());
    let client = FakeGatewayClient::new(peer(GatewayClientKind::OpenClaw));

    for (fault, expected) in [
        (FakeFault::Unavailable, LoopbackErrorKind::Unavailable),
        (FakeFault::Timeout, LoopbackErrorKind::Timeout),
        (FakeFault::Cancelled, LoopbackErrorKind::Cancelled),
    ] {
        server.push_fault(fault);
        assert_eq!(
            client.send(&mut server, &request).unwrap_err().kind(),
            expected
        );
    }
    assert_eq!(server.service().calls, 0);

    server.push_fault(FakeFault::AmbiguousAfterDispatch);
    assert_eq!(
        client.send(&mut server, &request).unwrap_err().kind(),
        LoopbackErrorKind::Ambiguous
    );
    assert_eq!(server.service().calls, 1);
    let recovered = client.send(&mut server, &request).unwrap();
    assert_eq!(server.service().calls, 1);

    let bytes = encode_reply(&recovered).unwrap();
    assert_eq!(decode_reply(&request, &bytes).unwrap(), recovered);
    assert_eq!(encode_reply(&recovered).unwrap(), bytes);
}
