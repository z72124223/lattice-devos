use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use lattice_contracts::{
    ContentDigest, GatewayActorId, GatewayAdapterId, GatewayChannelId, GatewayCommandId,
    GatewayCorrelationId, GatewayInstanceId, GatewayPeerContext, GatewayProjectStatusTarget,
    GatewayReply, GatewayReplyBody, GatewayRequest, GatewayRequestBody, GatewaySessionId,
    GatewayStatusObservation, GatewayStatusTarget, ProjectId, ProjectSnapshotId, RuntimeKind,
    SubjectBinding, TaskId, TaskSpecSubmission,
};
use lattice_gateway_ipc::{build_reply, build_request, task_spec_document_digest};
use lattice_openclaw_adapter::{
    AuthenticationKey, GatewayTransportErrorKind, OpenClawGatewayClient, OpenClawGatewayConfig,
    OpenClawGatewayServer, TransportNonce,
};
use lattice_ports::{GatewayService, GatewayServiceResult};

fn digest(fill: char) -> ContentDigest {
    ContentDigest::from_sha256(fill.to_string().repeat(64)).expect("digest")
}

fn live_peer() -> GatewayPeerContext {
    GatewayPeerContext::new_authenticated_openclaw(
        GatewayInstanceId::new("gateway-live-a").expect("gateway"),
        GatewayAdapterId::new("openclaw-adapter").expect("adapter"),
        "1.0.0",
        digest('a'),
        digest('b'),
        GatewayActorId::new("responsible-user-a").expect("actor"),
        GatewayChannelId::new("openclaw-local").expect("channel"),
        GatewaySessionId::new("session-live-a").expect("session"),
        1,
        digest('c'),
        digest('c'),
    )
    .expect("live peer")
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

#[test]
fn exact_retry_across_connections_reuses_terminal_reply_without_second_dispatch() {
    let key = AuthenticationKey::new([0x6a; 32]).expect("authentication key");
    let config = OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        Duration::from_secs(2),
        ProjectId::new("project-a").expect("project"),
        live_peer(),
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
        live_peer(),
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
        live_peer(),
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
        live_peer(),
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
        live_peer(),
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
    saw_live_peer: bool,
}

impl GatewayService for RecordingService {
    fn handle(
        &mut self,
        peer: GatewayPeerContext,
        request: GatewayRequest,
    ) -> GatewayServiceResult<GatewayReply> {
        self.calls += 1;
        self.saw_live_peer = peer.runtime() == RuntimeKind::Live;
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
fn canonical_submit_document_is_rejected_before_service_even_with_valid_digest() {
    let key = AuthenticationKey::new([0x7f; 32]).expect("authentication key");
    let config = OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        Duration::from_secs(2),
        ProjectId::new("project-a").expect("project"),
        live_peer(),
        key.clone(),
    )
    .expect("config");
    let server = OpenClawGatewayServer::bind(config, RecordingService::default())
        .expect("bind loopback server");
    let endpoint = server.local_addr().expect("local address");
    let server_thread = thread::spawn(move || {
        let mut server = server;
        let error = server
            .serve_once()
            .expect_err("submit document must fail at the edge");
        (server, error.kind())
    });
    let client = OpenClawGatewayClient::new(endpoint, Duration::from_secs(2), key).expect("client");
    let _client_error = client
        .send(
            &forbidden_submit_request(),
            TransportNonce::new([0x45; 16]).expect("nonce"),
        )
        .expect_err("forbidden submit must not receive a reply");

    let (server, server_error) = server_thread.join().expect("server thread");
    assert_eq!(server_error, GatewayTransportErrorKind::ForbiddenPayload);
    assert_eq!(server.service().expect("service").calls, 0);
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
        live_peer(),
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
        live_peer(),
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
fn authenticated_loopback_status_reaches_gateway_service_with_live_peer() {
    let key = AuthenticationKey::new([0x5a; 32]).expect("authentication key");
    let config = OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        Duration::from_secs(2),
        ProjectId::new("project-a").expect("project"),
        live_peer(),
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
    assert!(server.service().expect("service").saw_live_peer);
}
