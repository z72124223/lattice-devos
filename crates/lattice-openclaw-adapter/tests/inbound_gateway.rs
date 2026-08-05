use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use lattice_contracts::{
    ContentDigest, GatewayActorId, GatewayActorKind, GatewayAdapterId, GatewayChannelId,
    GatewayClientKind, GatewayCommandId, GatewayCorrelationId, GatewayInstanceId,
    GatewayPeerContext, GatewayProjectStatusTarget, GatewayReply, GatewayReplyBody, GatewayRequest,
    GatewayRequestBody, GatewaySessionId, GatewayStatusObservation, GatewayStatusTarget, ProjectId,
    ProjectSnapshotId, RuntimeKind, SubjectBinding, TaskId, TaskSpecSubmission,
};
use lattice_gateway_ipc::{build_reply, build_request, task_spec_document_digest};
use lattice_openclaw_adapter::{
    AuthenticationKey, GatewayTransportErrorKind, OpenClawGatewayClient, OpenClawGatewayConfig,
    OpenClawGatewayServer, OpenClawSubmitReplyBody, OpenClawSubmitRequest, TransportNonce,
    encode_openclaw_submit_request,
};
use lattice_ports::{GatewayService, GatewayServiceResult};

fn digest(fill: char) -> ContentDigest {
    ContentDigest::from_sha256(fill.to_string().repeat(64)).expect("digest")
}

fn transport_peer() -> GatewayPeerContext {
    GatewayPeerContext::new_fake(
        GatewayClientKind::OpenClaw,
        GatewayInstanceId::new("gateway-transport-a").expect("gateway"),
        GatewayAdapterId::new("openclaw-adapter").expect("adapter"),
        "1.0.0",
        digest('a'),
        digest('b'),
        GatewayActorId::new("responsible-user-a").expect("actor"),
        GatewayActorKind::ResponsibleUser,
        GatewayChannelId::new("openclaw-local").expect("channel"),
        GatewaySessionId::new("session-transport-a").expect("session"),
        1,
        digest('c'),
        digest('c'),
    )
    .expect("transport peer")
}

#[test]
fn rust_loopback_peer_is_transport_only_and_never_live() {
    let peer = transport_peer();
    assert_eq!(peer.runtime(), RuntimeKind::Fake);
    OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        Duration::from_secs(2),
        ProjectId::new("project-a").expect("project"),
        peer,
        AuthenticationKey::new([0x5c; 32]).expect("authentication key"),
    )
    .expect("transport-only peer must configure the loopback adapter");
}

fn status_request(project: &str, command: &str) -> GatewayRequest {
    status_request_with_page_size(project, command, 10)
}

fn status_request_with_page_size(project: &str, command: &str, page_size: u16) -> GatewayRequest {
    build_request(
        GatewayCommandId::new(command).expect("command"),
        GatewayCorrelationId::new(format!("correlation-{command}")).expect("correlation"),
        GatewayRequestBody::Status(GatewayStatusTarget::Project(
            GatewayProjectStatusTarget::new(
                ProjectId::new(project).expect("project"),
                page_size,
                None,
            )
            .expect("status target"),
        )),
    )
    .expect("request")
}

fn forbidden_submit_request() -> GatewayRequest {
    let document = br#"{"credential":"forbidden","memory":"forbidden","path":"forbidden","project_id":"project-a","project_snapshot_id":"snapshot-a","provider":"forbidden","revision":"1","schema_version":"2.1","shell":"forbidden","sql":"forbidden","task_id":"task-a","task_text":"forbidden"}"#.to_vec();
    let digest = task_spec_document_digest(&document).expect("document digest");
    let binding = SubjectBinding::new(
        ProjectId::new("project-a").expect("project"),
        ProjectSnapshotId::new("snapshot-a").expect("snapshot"),
        TaskId::new("task-a").expect("task"),
        "1",
        digest.clone(),
    )
    .expect("binding");
    build_request(
        GatewayCommandId::new("command-forbidden-submit").expect("command"),
        GatewayCorrelationId::new("correlation-forbidden-submit").expect("correlation"),
        GatewayRequestBody::Submit(
            TaskSpecSubmission::new(binding, document, digest).expect("submission"),
        ),
    )
    .expect("request")
}

fn frozen_submit_submission() -> TaskSpecSubmission {
    let document = br#"{"project_id":"project-a","project_snapshot_id":"snapshot-a","revision":"1","schema_version":"2.1","task_id":"task-a"}"#.to_vec();
    let digest = task_spec_document_digest(&document).expect("document digest");
    let binding = SubjectBinding::new(
        ProjectId::new("project-a").expect("project"),
        ProjectSnapshotId::new("snapshot-a").expect("snapshot"),
        TaskId::new("task-a").expect("task"),
        "1",
        digest.clone(),
    )
    .expect("binding");
    TaskSpecSubmission::new(binding, document, digest).expect("submission")
}

#[derive(Default)]
struct SubmitRecordingService {
    calls: usize,
    saw_server_owned_document: bool,
}

impl GatewayService for SubmitRecordingService {
    fn handle(
        &mut self,
        _peer: GatewayPeerContext,
        request: GatewayRequest,
    ) -> GatewayServiceResult<GatewayReply> {
        self.calls += 1;
        let GatewayRequestBody::Submit(submission) = request.body() else {
            panic!("expected reconstructed Submit");
        };
        self.saw_server_owned_document =
            submission.canonical_document() == frozen_submit_submission().canonical_document();
        build_reply(
            &request,
            GatewayReplyBody::SubmitAccepted {
                binding: submission.binding().clone(),
                command_receipt_digest: digest('d'),
            },
        )
        .map_err(|error| {
            lattice_ports::GatewayServiceError::new(
                lattice_ports::PortErrorKind::Malformed,
                error.code(),
            )
        })
    }
}

#[test]
fn binding_only_submit_reconstructs_frozen_spec_and_reaches_gateway_once() {
    let key = AuthenticationKey::new([0x60; 32]).expect("authentication key");
    let submission = frozen_submit_submission();
    let transport_request = OpenClawSubmitRequest::new(
        GatewayCommandId::new("command-submit-a").expect("command"),
        GatewayCorrelationId::new("correlation-submit-a").expect("correlation"),
        submission.binding().clone(),
    );
    let request_bytes = encode_openclaw_submit_request(&transport_request).expect("closed codec");
    for forbidden in [
        b"canonical_document".as_slice(),
        b"task_text".as_slice(),
        b"memory".as_slice(),
        b"sql".as_slice(),
        b"path".as_slice(),
        b"credential".as_slice(),
        b"provider".as_slice(),
    ] {
        assert!(
            !request_bytes
                .windows(forbidden.len())
                .any(|window| window == forbidden)
        );
    }
    let config = OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        Duration::from_secs(2),
        ProjectId::new("project-a").expect("project"),
        transport_peer(),
        key.clone(),
    )
    .expect("config")
    .with_frozen_submission(submission)
    .expect("frozen submission");
    let server = OpenClawGatewayServer::bind(config, SubmitRecordingService::default())
        .expect("bind loopback server");
    let endpoint = server.local_addr().expect("local address");
    let server_thread = thread::spawn(move || {
        let mut server = server;
        server.serve_once().expect("serve binding-only Submit");
        server
    });
    let client = OpenClawGatewayClient::new(endpoint, Duration::from_secs(2), key).expect("client");

    let reply = client
        .send_submit(
            &transport_request,
            TransportNonce::new([0x61; 16]).expect("nonce"),
        )
        .expect("typed Submit reply");

    assert!(matches!(
        reply.body(),
        OpenClawSubmitReplyBody::Accepted {
            binding,
            command_receipt_digest,
        } if binding == transport_request.binding() && command_receipt_digest == &digest('d')
    ));
    let server = server_thread.join().expect("server thread");
    let service = server.service().expect("service");
    assert_eq!(service.calls, 1);
    assert!(service.saw_server_owned_document);
}

#[test]
fn binding_only_submit_exact_retry_across_connections_dispatches_once() {
    let key = AuthenticationKey::new([0x62; 32]).expect("authentication key");
    let submission = frozen_submit_submission();
    let request = OpenClawSubmitRequest::new(
        GatewayCommandId::new("command-submit-retry-a").expect("command"),
        GatewayCorrelationId::new("correlation-submit-retry-a").expect("correlation"),
        submission.binding().clone(),
    );
    let config = OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        Duration::from_secs(2),
        ProjectId::new("project-a").expect("project"),
        transport_peer(),
        key.clone(),
    )
    .expect("config")
    .with_frozen_submission(submission)
    .expect("frozen submission");
    let server = OpenClawGatewayServer::bind(config, SubmitRecordingService::default())
        .expect("bind loopback server");
    let endpoint = server.local_addr().expect("local address");
    let server_thread = thread::spawn(move || {
        let mut server = server;
        server.serve_once().expect("serve first Submit");
        server.serve_once().expect("serve exact retry");
        server
    });
    let client = OpenClawGatewayClient::new(endpoint, Duration::from_secs(2), key).expect("client");

    let first = client
        .send_submit(
            &request,
            TransportNonce::new([0x63; 16]).expect("first nonce"),
        )
        .expect("first typed reply");
    let retry = client
        .send_submit(
            &request,
            TransportNonce::new([0x64; 16]).expect("retry nonce"),
        )
        .expect("cached typed reply");

    assert_eq!(first, retry);
    let server = server_thread.join().expect("server thread");
    assert_eq!(server.service().expect("service").calls, 1);
}

#[test]
fn unknown_submit_digest_is_rejected_before_gateway_service() {
    let key = AuthenticationKey::new([0x65; 32]).expect("authentication key");
    let submission = frozen_submit_submission();
    let unknown_binding = SubjectBinding::new(
        ProjectId::new("project-a").expect("project"),
        ProjectSnapshotId::new("snapshot-a").expect("snapshot"),
        TaskId::new("task-a").expect("task"),
        "1",
        digest('e'),
    )
    .expect("unknown binding");
    let request = OpenClawSubmitRequest::new(
        GatewayCommandId::new("command-submit-unknown").expect("command"),
        GatewayCorrelationId::new("correlation-submit-unknown").expect("correlation"),
        unknown_binding,
    );
    let config = OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        Duration::from_secs(2),
        ProjectId::new("project-a").expect("project"),
        transport_peer(),
        key.clone(),
    )
    .expect("config")
    .with_frozen_submission(submission)
    .expect("frozen submission");
    let server = OpenClawGatewayServer::bind(config, SubmitRecordingService::default())
        .expect("bind loopback server");
    let endpoint = server.local_addr().expect("local address");
    let server_thread = thread::spawn(move || {
        let mut server = server;
        let error = server
            .serve_once()
            .expect_err("unknown digest must not dispatch");
        (server, error.kind())
    });
    let client = OpenClawGatewayClient::new(endpoint, Duration::from_secs(2), key).expect("client");

    client
        .send_submit(&request, TransportNonce::new([0x66; 16]).expect("nonce"))
        .expect_err("unknown digest must close without a typed reply");

    let (server, error) = server_thread.join().expect("server thread");
    assert_eq!(error, GatewayTransportErrorKind::ForbiddenPayload);
    assert_eq!(server.service().expect("service").calls, 0);
}

#[test]
fn exact_retry_across_connections_reuses_terminal_reply_without_second_dispatch() {
    let key = AuthenticationKey::new([0x6a; 32]).expect("authentication key");
    let config = OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        Duration::from_secs(2),
        ProjectId::new("project-a").expect("project"),
        transport_peer(),
        key.clone(),
    )
    .expect("config");
    let server = OpenClawGatewayServer::bind(config, RecordingService::default())
        .expect("bind loopback server");
    let endpoint = server.local_addr().expect("local address");
    let server_thread = thread::spawn(move || {
        let mut server = server;
        server.serve_once().expect("serve first request");
        server.serve_once().expect("serve exact retry");
        server
    });
    let client = OpenClawGatewayClient::new(endpoint, Duration::from_secs(2), key).expect("client");
    let request = status_request("project-a", "command-retry-a");

    let first = client
        .send(
            &request,
            TransportNonce::new([0x21; 16]).expect("first nonce"),
        )
        .expect("first reply");
    let retry = client
        .send(
            &request,
            TransportNonce::new([0x22; 16]).expect("retry nonce"),
        )
        .expect("retry reply");

    assert_eq!(first, retry);
    let server = server_thread.join().expect("server thread");
    assert_eq!(server.service().expect("service").calls, 1);
}

#[test]
fn changed_content_under_one_command_is_denied_without_second_dispatch() {
    let key = AuthenticationKey::new([0x6b; 32]).expect("authentication key");
    let config = OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        Duration::from_secs(2),
        ProjectId::new("project-a").expect("project"),
        transport_peer(),
        key.clone(),
    )
    .expect("config");
    let server = OpenClawGatewayServer::bind(config, RecordingService::default())
        .expect("bind loopback server");
    let endpoint = server.local_addr().expect("local address");
    let server_thread = thread::spawn(move || {
        let mut server = server;
        server.serve_once().expect("serve original request");
        server.serve_once().expect("serve substitution denial");
        server
    });
    let client = OpenClawGatewayClient::new(endpoint, Duration::from_secs(2), key).expect("client");
    let original = status_request_with_page_size("project-a", "command-substitution-a", 10);
    let substituted = status_request_with_page_size("project-a", "command-substitution-a", 9);

    client
        .send(
            &original,
            TransportNonce::new([0x31; 16]).expect("first nonce"),
        )
        .expect("original reply");
    let denial = client
        .send(
            &substituted,
            TransportNonce::new([0x32; 16]).expect("substitution nonce"),
        )
        .expect("typed substitution denial");

    assert!(matches!(
        denial.body(),
        GatewayReplyBody::Denied(lattice_contracts::GatewayDenialCode::CommandSubstitution)
    ));
    let server = server_thread.join().expect("server thread");
    assert_eq!(server.service().expect("service").calls, 1);
}

#[test]
fn wrong_authentication_key_is_rejected_before_codec_or_service() {
    let server_key = AuthenticationKey::new([0x7a; 32]).expect("server key");
    let client_key = AuthenticationKey::new([0x7b; 32]).expect("client key");
    let config = OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        Duration::from_secs(2),
        ProjectId::new("project-a").expect("project"),
        transport_peer(),
        server_key,
    )
    .expect("config");
    let server = OpenClawGatewayServer::bind(config, RecordingService::default())
        .expect("bind loopback server");
    let endpoint = server.local_addr().expect("local address");
    let server_thread = thread::spawn(move || {
        let mut server = server;
        let error = server.serve_once().expect_err("wrong key must fail");
        (server, error.kind())
    });
    let client =
        OpenClawGatewayClient::new(endpoint, Duration::from_secs(2), client_key).expect("client");
    let _client_error = client
        .send(
            &status_request("project-a", "command-auth-a"),
            TransportNonce::new([0x41; 16]).expect("nonce"),
        )
        .expect_err("server must close unauthenticated request");

    let (server, server_error) = server_thread.join().expect("server thread");
    assert_eq!(server_error, GatewayTransportErrorKind::Authentication);
    assert_eq!(server.service().expect("service").calls, 0);
}

#[test]
fn reused_authenticated_nonce_is_rejected_before_second_dispatch() {
    let key = AuthenticationKey::new([0x7c; 32]).expect("authentication key");
    let config = OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        Duration::from_secs(2),
        ProjectId::new("project-a").expect("project"),
        transport_peer(),
        key.clone(),
    )
    .expect("config");
    let server = OpenClawGatewayServer::bind(config, RecordingService::default())
        .expect("bind loopback server");
    let endpoint = server.local_addr().expect("local address");
    let server_thread = thread::spawn(move || {
        let mut server = server;
        server.serve_once().expect("serve first request");
        let error = server.serve_once().expect_err("nonce replay must fail");
        (server, error.kind())
    });
    let client = OpenClawGatewayClient::new(endpoint, Duration::from_secs(2), key).expect("client");
    let request = status_request("project-a", "command-nonce-a");
    let nonce = TransportNonce::new([0x42; 16]).expect("nonce");
    client.send(&request, nonce).expect("first reply");
    let _client_error = client
        .send(&request, nonce)
        .expect_err("server must close replayed nonce");

    let (server, server_error) = server_thread.join().expect("server thread");
    assert_eq!(server_error, GatewayTransportErrorKind::Replay);
    assert_eq!(server.service().expect("service").calls, 1);
}

#[test]
fn cross_project_request_is_rejected_before_gateway_service() {
    let key = AuthenticationKey::new([0x7d; 32]).expect("authentication key");
    let config = OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        Duration::from_secs(2),
        ProjectId::new("project-a").expect("project"),
        transport_peer(),
        key.clone(),
    )
    .expect("config");
    let server = OpenClawGatewayServer::bind(config, RecordingService::default())
        .expect("bind loopback server");
    let endpoint = server.local_addr().expect("local address");
    let server_thread = thread::spawn(move || {
        let mut server = server;
        let error = server.serve_once().expect_err("foreign project must fail");
        (server, error.kind())
    });
    let client = OpenClawGatewayClient::new(endpoint, Duration::from_secs(2), key).expect("client");
    let _client_error = client
        .send(
            &status_request("project-b", "command-project-b"),
            TransportNonce::new([0x43; 16]).expect("nonce"),
        )
        .expect_err("server must close cross-project request");

    let (server, server_error) = server_thread.join().expect("server thread");
    assert_eq!(server_error, GatewayTransportErrorKind::CrossProject);
    assert_eq!(server.service().expect("service").calls, 0);
}

#[derive(Default)]
struct RecordingService {
    calls: usize,
    saw_transport_only_peer: bool,
}

impl GatewayService for RecordingService {
    fn handle(
        &mut self,
        peer: GatewayPeerContext,
        request: GatewayRequest,
    ) -> GatewayServiceResult<GatewayReply> {
        self.calls += 1;
        self.saw_transport_only_peer = peer.runtime() == RuntimeKind::Fake;
        build_reply(
            &request,
            GatewayReplyBody::StatusObserved(GatewayStatusObservation::Project {
                project_id: request.project_id().clone(),
                tasks: Vec::new(),
                next_cursor: None,
            }),
        )
        .map_err(|error| {
            lattice_ports::GatewayServiceError::new(
                lattice_ports::PortErrorKind::Malformed,
                error.code(),
            )
        })
    }
}

struct TimeoutService {
    calls: usize,
}

struct SlowService {
    calls: Arc<AtomicUsize>,
    delay: Duration,
}

struct SlowSubmitService {
    calls: Arc<AtomicUsize>,
    delay: Duration,
}

impl GatewayService for SlowSubmitService {
    fn handle(
        &mut self,
        _peer: GatewayPeerContext,
        request: GatewayRequest,
    ) -> GatewayServiceResult<GatewayReply> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        thread::sleep(self.delay);
        let GatewayRequestBody::Submit(submission) = request.body() else {
            panic!("expected reconstructed Submit");
        };
        build_reply(
            &request,
            GatewayReplyBody::SubmitAccepted {
                binding: submission.binding().clone(),
                command_receipt_digest: digest('f'),
            },
        )
        .map_err(|error| {
            lattice_ports::GatewayServiceError::new(
                lattice_ports::PortErrorKind::Malformed,
                error.code(),
            )
        })
    }
}

impl GatewayService for SlowService {
    fn handle(
        &mut self,
        _peer: GatewayPeerContext,
        request: GatewayRequest,
    ) -> GatewayServiceResult<GatewayReply> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        thread::sleep(self.delay);
        build_reply(
            &request,
            GatewayReplyBody::StatusObserved(GatewayStatusObservation::Project {
                project_id: request.project_id().clone(),
                tasks: Vec::new(),
                next_cursor: None,
            }),
        )
        .map_err(|error| {
            lattice_ports::GatewayServiceError::new(
                lattice_ports::PortErrorKind::Malformed,
                error.code(),
            )
        })
    }
}

#[test]
fn client_rejects_full_submit_before_encoding_or_connect() {
    let key = AuthenticationKey::new([0x7f; 32]).expect("authentication key");
    let closed_listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .expect("reserve endpoint");
    let unavailable_endpoint = closed_listener.local_addr().expect("reserved endpoint");
    drop(closed_listener);
    let client = OpenClawGatewayClient::new(unavailable_endpoint, Duration::from_secs(2), key)
        .expect("client");
    let error = client
        .send(
            &forbidden_submit_request(),
            TransportNonce::new([0x45; 16]).expect("nonce"),
        )
        .expect_err("full Submit must fail before the unavailable endpoint is contacted");

    assert_eq!(error.kind(), GatewayTransportErrorKind::ForbiddenPayload);
}

#[test]
fn hung_service_times_out_ambiguously_and_exact_retry_never_creates_second_writer() {
    let calls = Arc::new(AtomicUsize::new(0));
    let key = AuthenticationKey::new([0x6f; 32]).expect("authentication key");
    let timeout = Duration::from_millis(75);
    let config = OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        timeout,
        ProjectId::new("project-a").expect("project"),
        transport_peer(),
        key.clone(),
    )
    .expect("config");
    let server = OpenClawGatewayServer::bind(
        config,
        SlowService {
            calls: calls.clone(),
            delay: Duration::from_millis(300),
        },
    )
    .expect("bind loopback server");
    let endpoint = server.local_addr().expect("local address");
    let request = status_request("project-a", "command-hung-service");
    let first_server_thread = thread::spawn(move || {
        let mut server = server;
        let started = Instant::now();
        let error = server
            .serve_once()
            .expect_err("hung service must become ambiguous");
        (server, error.kind(), started.elapsed())
    });
    let client = OpenClawGatewayClient::new(endpoint, timeout, key.clone()).expect("client");
    let _client_error = client
        .send(&request, TransportNonce::new([0x46; 16]).expect("nonce"))
        .expect_err("hung dispatch must not report success");
    let (server, server_error, elapsed) = first_server_thread.join().expect("server thread");
    assert_eq!(server_error, GatewayTransportErrorKind::Ambiguous);
    assert!(elapsed < Duration::from_millis(250));
    assert!(server.service().is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    thread::sleep(Duration::from_millis(350));
    let retry_server_thread = thread::spawn(move || {
        let mut server = server;
        server.serve_once().expect("reconcile exact retry");
        server
    });
    let retry_client = OpenClawGatewayClient::new(endpoint, timeout, key).expect("client");
    retry_client
        .send(
            &request,
            TransportNonce::new([0x47; 16]).expect("retry nonce"),
        )
        .expect("reconciled reply");
    let server = retry_server_thread.join().expect("retry server thread");
    assert!(server.service().is_some());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn submit_deadline_is_ambiguous_and_reconciled_retry_never_dispatches_twice() {
    let calls = Arc::new(AtomicUsize::new(0));
    let key = AuthenticationKey::new([0x67; 32]).expect("authentication key");
    let timeout = Duration::from_millis(75);
    let submission = frozen_submit_submission();
    let request = OpenClawSubmitRequest::new(
        GatewayCommandId::new("command-submit-timeout").expect("command"),
        GatewayCorrelationId::new("correlation-submit-timeout").expect("correlation"),
        submission.binding().clone(),
    );
    let config = OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        timeout,
        ProjectId::new("project-a").expect("project"),
        transport_peer(),
        key.clone(),
    )
    .expect("config")
    .with_frozen_submission(submission)
    .expect("frozen submission");
    let server = OpenClawGatewayServer::bind(
        config,
        SlowSubmitService {
            calls: calls.clone(),
            delay: Duration::from_millis(300),
        },
    )
    .expect("bind loopback server");
    let endpoint = server.local_addr().expect("local address");
    let first_server_thread = thread::spawn(move || {
        let mut server = server;
        let error = server
            .serve_once()
            .expect_err("slow Submit must become ambiguous");
        (server, error.kind())
    });
    let client = OpenClawGatewayClient::new(endpoint, timeout, key.clone()).expect("client");
    client
        .send_submit(
            &request,
            TransportNonce::new([0x68; 16]).expect("first nonce"),
        )
        .expect_err("ambiguous dispatch must not report success");
    let (server, error) = first_server_thread.join().expect("server thread");
    assert_eq!(error, GatewayTransportErrorKind::Ambiguous);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    thread::sleep(Duration::from_millis(350));
    let retry_server_thread = thread::spawn(move || {
        let mut server = server;
        server.serve_once().expect("reconcile exact Submit retry");
        server
    });
    let retry_client = OpenClawGatewayClient::new(endpoint, timeout, key).expect("client");
    let reply = retry_client
        .send_submit(
            &request,
            TransportNonce::new([0x69; 16]).expect("retry nonce"),
        )
        .expect("reconciled typed reply");
    assert!(matches!(
        reply.body(),
        OpenClawSubmitReplyBody::Accepted { .. }
    ));
    let server = retry_server_thread.join().expect("retry server thread");
    assert!(server.service().is_some());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

impl GatewayService for TimeoutService {
    fn handle(
        &mut self,
        _peer: GatewayPeerContext,
        _request: GatewayRequest,
    ) -> GatewayServiceResult<GatewayReply> {
        self.calls += 1;
        Err(lattice_ports::GatewayServiceError::new(
            lattice_ports::PortErrorKind::Timeout,
            "TEST_ONLY_SERVICE_TIMEOUT",
        ))
    }
}

#[test]
fn gateway_service_timeout_never_becomes_a_success_reply() {
    let key = AuthenticationKey::new([0x7e; 32]).expect("authentication key");
    let config = OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        Duration::from_secs(2),
        ProjectId::new("project-a").expect("project"),
        transport_peer(),
        key.clone(),
    )
    .expect("config");
    let server = OpenClawGatewayServer::bind(config, TimeoutService { calls: 0 })
        .expect("bind loopback server");
    let endpoint = server.local_addr().expect("local address");
    let server_thread = thread::spawn(move || {
        let mut server = server;
        let error = server
            .serve_once()
            .expect_err("service timeout must fail closed");
        (server, error.kind())
    });
    let client = OpenClawGatewayClient::new(endpoint, Duration::from_secs(2), key).expect("client");
    let _client_error = client
        .send(
            &status_request("project-a", "command-service-timeout"),
            TransportNonce::new([0x44; 16]).expect("nonce"),
        )
        .expect_err("timeout must not produce a typed success");

    let (server, server_error) = server_thread.join().expect("server thread");
    assert_eq!(server_error, GatewayTransportErrorKind::Timeout);
    assert_eq!(server.service().expect("service").calls, 1);
}

#[test]
fn authenticated_loopback_status_reaches_gateway_service_with_transport_only_peer() {
    let key = AuthenticationKey::new([0x5a; 32]).expect("authentication key");
    let config = OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        Duration::from_secs(2),
        ProjectId::new("project-a").expect("project"),
        transport_peer(),
        key.clone(),
    )
    .expect("config");
    let server = OpenClawGatewayServer::bind(config, RecordingService::default())
        .expect("bind loopback server");
    let endpoint = server.local_addr().expect("local address");

    let server_thread = thread::spawn(move || {
        let mut server = server;
        server.serve_once().expect("serve authenticated request");
        server
    });
    let client = OpenClawGatewayClient::new(endpoint, Duration::from_secs(2), key).expect("client");
    let request = status_request("project-a", "command-status-a");
    let reply = client
        .send(&request, TransportNonce::new([0x11; 16]).expect("nonce"))
        .expect("authenticated reply");

    assert!(matches!(
        reply.body(),
        GatewayReplyBody::StatusObserved(GatewayStatusObservation::Project {
            project_id,
            tasks,
            next_cursor: None,
        }) if project_id.as_str() == "project-a" && tasks.is_empty()
    ));
    let server = server_thread.join().expect("server thread");
    assert_eq!(server.service().expect("service").calls, 1);
    assert!(server.service().expect("service").saw_transport_only_peer);
}
