use lattice_contracts::{
    AttemptId, Boundary, CONTRACT_VERSION, CodexEvidence, CodexRunRequest, Component,
    ContentDigest, GatewayAction, GatewayActorId, GatewayActorKind, GatewayAdapterId,
    GatewayChannelId, GatewayClientKind, GatewayCommandId, GatewayCorrelationId, GatewayInstanceId,
    GatewayPeerContext, GatewayReply, GatewayReplyBody, GatewayRequest, GatewayRequestBody,
    GatewaySessionId, GatewayTaskTarget, GraphifyBuildRequest, GraphifyEvidence, HermesEvidence,
    HermesResearchRequest, Invocation, ProjectId, ProjectSnapshotId, RequestId, RuntimeKind,
    SubjectBinding, TaskId,
};
use lattice_ports::{
    CodexPort, GatewayService, GatewayServiceError, GatewayServiceResult, GraphifyPort, HermesPort,
    PortError, PortErrorKind, PortResult,
};

struct AllPorts;

impl GatewayService for AllPorts {
    fn handle(
        &mut self,
        peer: GatewayPeerContext,
        request: GatewayRequest,
    ) -> GatewayServiceResult<GatewayReply> {
        assert_eq!(peer.runtime(), RuntimeKind::Fake);
        let GatewayRequestBody::Plan(target) = request.body() else {
            panic!("test request must be plan");
        };
        GatewayReply::new(
            &request,
            GatewayReplyBody::PlanRouted {
                binding: target.binding().clone(),
                command_receipt_digest: digest('b'),
            },
            digest('c'),
        )
        .map_err(|_| GatewayServiceError::new(PortErrorKind::Malformed, "reply-binding"))
    }
}

impl CodexPort for AllPorts {
    fn run(&mut self, request: CodexRunRequest) -> PortResult<CodexEvidence> {
        Ok(CodexEvidence::new(
            request.into_invocation(),
            RuntimeKind::Fake,
            digest('b'),
        ))
    }

    fn interrupt(&mut self, _request_id: &RequestId) -> PortResult<()> {
        Ok(())
    }
}

impl GraphifyPort for AllPorts {
    fn build_code_graph(&mut self, request: GraphifyBuildRequest) -> PortResult<GraphifyEvidence> {
        Ok(GraphifyEvidence::new(
            request.into_invocation(),
            RuntimeKind::Fake,
            digest('b'),
        ))
    }
}

impl HermesPort for AllPorts {
    fn research(&mut self, request: HermesResearchRequest) -> PortResult<HermesEvidence> {
        Ok(HermesEvidence::new(
            request.into_invocation(),
            RuntimeKind::Fake,
            digest('b'),
        ))
    }

    fn interrupt(&mut self, _request_id: &RequestId) -> PortResult<()> {
        Ok(())
    }
}

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid test digest")
}

fn invocation() -> Invocation {
    Invocation::new(
        CONTRACT_VERSION,
        RequestId::new("request-1").expect("valid request id"),
        TaskId::new("task-9").expect("valid task id"),
        AttemptId::new("attempt-1").expect("valid attempt id"),
        ProjectSnapshotId::new("snapshot-1").expect("valid snapshot id"),
        digest('a'),
    )
    .expect("supported contract")
}

fn gateway_request() -> GatewayRequest {
    let binding = SubjectBinding::new(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        TaskId::new("task-9").expect("task"),
        "1",
        digest('a'),
    )
    .expect("binding");
    GatewayRequest::new(
        1,
        GatewayCommandId::new("command-1").expect("command"),
        GatewayCorrelationId::new("correlation-1").expect("correlation"),
        GatewayRequestBody::Plan(
            GatewayTaskTarget::new(binding, digest('d')).expect("gateway target"),
        ),
        digest('e'),
    )
    .expect("gateway request")
}

fn gateway_peer() -> GatewayPeerContext {
    GatewayPeerContext::new_fake(
        GatewayClientKind::TestFake,
        GatewayInstanceId::new("gateway-1").expect("gateway"),
        GatewayAdapterId::new("fake-adapter").expect("adapter"),
        "1.0",
        digest('1'),
        digest('2'),
        GatewayActorId::new("actor-1").expect("actor"),
        GatewayActorKind::TestFixture,
        GatewayChannelId::new("channel-1").expect("channel"),
        GatewaySessionId::new("session-1").expect("session"),
        1,
        digest('3'),
        digest('3'),
    )
    .expect("fake peer")
}

#[test]
fn four_non_store_role_traits_keep_lane_classification_explicit() {
    let mut ports = AllPorts;

    assert_eq!(
        ports
            .handle(gateway_peer(), gateway_request())
            .expect("gateway")
            .action(),
        GatewayAction::Plan
    );
    assert_eq!(
        ports
            .run(CodexRunRequest::new(invocation(), digest('d')))
            .expect("writer")
            .boundary(),
        Boundary::ProductCodeWriter
    );
    assert_eq!(
        ports
            .build_code_graph(GraphifyBuildRequest::new(invocation()))
            .expect("knowledge")
            .boundary(),
        Boundary::DerivedReadOnlyEvidence
    );
    assert_eq!(
        ports
            .research(HermesResearchRequest::new(invocation()))
            .expect("research")
            .boundary(),
        Boundary::UntrustedCandidate
    );
}

#[test]
fn port_errors_keep_unknown_outcomes_explicit() {
    let error = PortError::new(
        Component::Codex,
        PortErrorKind::Ambiguous,
        "completion-missing",
    );

    assert_eq!(error.component(), Component::Codex);
    assert_eq!(error.kind(), PortErrorKind::Ambiguous);
    assert_eq!(error.code(), "completion-missing");
}

#[test]
fn gateway_service_errors_do_not_misattribute_an_external_component() {
    let error = GatewayServiceError::new(PortErrorKind::Malformed, "reply-binding");

    assert_eq!(error.kind(), PortErrorKind::Malformed);
    assert_eq!(error.code(), "reply-binding");
    assert_eq!(error.to_string(), "GatewayService Malformed: reply-binding");
}
