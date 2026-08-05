use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, ContentDigest, HermesResearchRequest, Invocation,
    ProjectSnapshotId, RequestId, TaskId,
};
use lattice_hermes_adapter::{
    HERMES_LICENSE, HERMES_RELEASE, HERMES_SCHEMA_VERSION, HERMES_UPSTREAM_COMMIT,
    HermesAdapterConfig, HermesAdapterErrorKind, HermesMemoryPolicy, HermesProcessConfig,
    HermesReflectionAdapter, HermesReflectionJob, ReflectionEvidence, ReflectionEvidenceKind,
};

const SUBJECT_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const GRAPH_DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn capabilities_runs_events_and_status_produce_bound_canonical_reflection() {
    let request = request();
    let job = job(request.clone());
    let output = bound_output(&job);
    let server = ScriptedServer::start(vec![
        capabilities(),
        Response::json(202, r#"{"run_id":"run_abc123","status":"started"}"#),
        completed_events("run_abc123", &output),
        completed_status("run_abc123", "lattice-task-034-session", &output),
    ]);

    let mut adapter = HermesReflectionAdapter::connect(config(&server), job).expect("adapter");
    let reflection = adapter
        .run_reflection(&request)
        .expect("strict reflection accepted");

    assert_eq!(HERMES_RELEASE, "v2026.8.3");
    assert_eq!(
        HERMES_UPSTREAM_COMMIT,
        "3c27eb6234bf91b8ceee9e9071591b31e9b148cb"
    );
    assert_eq!(HERMES_LICENSE, "MIT");
    assert_eq!(
        reflection.summary(),
        "The adapter boundary is read-only and fail-closed."
    );
    assert_eq!(reflection.findings().len(), 1);
    assert_eq!(reflection.next_actions().len(), 1);
    assert_eq!(reflection.binding().request_id(), "request-hermes-1");
    assert_eq!(reflection.canonical_bytes()[0], b'{');
    assert_eq!(reflection.output_digest().as_str().len(), 64);

    let requests = server.finish();
    assert_eq!(requests.len(), 4);
    assert!(requests[0].starts_with("GET /v1/capabilities HTTP/1.1\r\n"));
    assert!(requests[1].starts_with("POST /v1/runs HTTP/1.1\r\n"));
    assert!(requests[1].contains("Authorization: Bearer test-only-loopback-key\r\n"));
    assert!(requests[1].contains(r#""session_id":"lattice-task-034-session""#));
    assert!(requests[1].contains(r#""model":"hermes-agent""#));
    assert!(requests[1].contains("Do not call tools"));
    assert!(requests[1].contains("Hermes session memory as unavailable"));
    assert!(requests[1].contains("do not read or write PostgreSQL"));
    assert!(requests[1].contains("Label every finding as inference"));
    assert!(requests[2].starts_with("GET /v1/runs/run_abc123/events HTTP/1.1\r\n"));
    assert!(requests[3].starts_with("GET /v1/runs/run_abc123 HTTP/1.1\r\n"));
}

#[test]
fn event_transport_loss_recovers_through_bound_status_polling() {
    let request = request();
    let job = job(request.clone());
    let output = bound_output(&job);
    let server = ScriptedServer::start(vec![
        capabilities(),
        Response::json(202, r#"{"run_id":"run_recoverable","status":"started"}"#),
        Response::json(503, r#"{"error":{"message":"event buffer expired"}}"#),
        completed_status("run_recoverable", "lattice-task-034-session", &output),
    ]);
    let mut adapter = HermesReflectionAdapter::connect(config(&server), job).expect("adapter");

    let reflection = adapter
        .run_reflection(&request)
        .expect("polling recovers an unavailable event stream");

    assert_eq!(
        reflection.summary(),
        "The adapter boundary is read-only and fail-closed."
    );
    assert_eq!(server.finish().len(), 4);
}

#[test]
fn restart_recovery_uses_capabilities_and_pollable_status_without_resubmission() {
    let request = request();
    let job = job(request.clone());
    let output = bound_output(&job);
    let server = ScriptedServer::start(vec![
        capabilities(),
        completed_status("run_restart", "lattice-task-034-session", &output),
    ]);
    let mut restarted =
        HermesReflectionAdapter::connect(config(&server), job).expect("restarted adapter");

    let reflection = restarted
        .recover_reflection(&request, "run_restart")
        .expect("status recovery");

    assert_eq!(reflection.binding().input_digest().len(), 64);
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /v1/capabilities"));
    assert!(requests[1].starts_with("GET /v1/runs/run_restart HTTP/1.1"));
}

#[test]
fn malformed_reflection_schema_fails_closed() {
    let request = request();
    let job = job(request.clone());
    let mut malformed_output: serde_json::Value =
        serde_json::from_str(&bound_output(&job)).expect("reflection fixture");
    malformed_output
        .as_object_mut()
        .expect("reflection object")
        .insert("unexpected".to_owned(), serde_json::json!(true));
    let malformed_output = malformed_output.to_string();
    let server = ScriptedServer::start(vec![
        capabilities(),
        Response::json(202, r#"{"run_id":"run_malformed","status":"started"}"#),
        completed_events("run_malformed", &malformed_output),
        completed_status(
            "run_malformed",
            "lattice-task-034-session",
            &malformed_output,
        ),
    ]);
    let mut adapter = HermesReflectionAdapter::connect(config(&server), job).expect("adapter");

    let failure = adapter
        .run_reflection(&request)
        .expect_err("unknown schema fields must fail closed");

    assert_eq!(failure.kind(), HermesAdapterErrorKind::Malformed);
    assert_eq!(failure.code(), "HERMES_REFLECTION_SCHEMA_REJECTED");
    assert_eq!(server.finish().len(), 4);
}

#[test]
fn status_session_cross_binding_fails_closed() {
    let request = request();
    let job = job(request.clone());
    let output = bound_output(&job);
    let server = ScriptedServer::start(vec![
        capabilities(),
        Response::json(202, r#"{"run_id":"run_cross","status":"started"}"#),
        completed_events("run_cross", &output),
        completed_status("run_cross", "foreign-session", &output),
    ]);
    let mut adapter = HermesReflectionAdapter::connect(config(&server), job).expect("adapter");

    let failure = adapter
        .run_reflection(&request)
        .expect_err("foreign session must fail closed");

    assert_eq!(failure.kind(), HermesAdapterErrorKind::CrossBinding);
    assert_eq!(failure.code(), "HERMES_STATUS_SESSION_BINDING_REJECTED");
    assert_eq!(server.finish().len(), 4);
}

#[test]
fn event_and_status_timeouts_fail_closed() {
    let request = request();
    let job = job(request.clone());
    let server = ScriptedServer::start(vec![
        capabilities(),
        Response::json(202, r#"{"run_id":"run_timeout","status":"started"}"#),
        Response::sse(200, ": late\n\n").with_delay(Duration::from_millis(60)),
        Response::json(200, r#"{"object":"hermes.run"}"#).with_delay(Duration::from_millis(60)),
    ]);
    let timeout_config = HermesAdapterConfig::new(
        server.address(),
        "test-only-loopback-key",
        Duration::from_millis(20),
        Duration::from_millis(1),
    )
    .expect("short timeout config");
    let mut adapter = HermesReflectionAdapter::connect(timeout_config, job).expect("adapter");

    let failure = adapter
        .run_reflection(&request)
        .expect_err("timeout must fail closed");

    assert_eq!(failure.kind(), HermesAdapterErrorKind::Timeout);
    assert_eq!(server.finish().len(), 4);
}

#[test]
fn process_command_uses_only_explicit_isolated_homes_and_loopback_api() {
    let executable = std::env::current_exe().expect("test executable");
    let scratch = std::env::temp_dir().join(format!(
        "lattice-hermes-process-contract-{}",
        std::process::id()
    ));
    let hermes_home = scratch.join("hermes-home");
    let codex_home = scratch.join("codex-home");
    let process = HermesProcessConfig::new(
        executable.clone(),
        hermes_home.clone(),
        codex_home.clone(),
        "127.0.0.1:8642".parse().expect("loopback"),
        "process-local-api-key",
        "hermes-agent",
        Duration::from_secs(2),
    )
    .expect("isolated process config");

    assert_eq!(
        process.memory_policy(),
        HermesMemoryPolicy::EphemeralIsolatedHome
    );

    let command = process.gateway_command().expect("gateway command");
    assert_eq!(command.get_program(), executable.as_os_str());
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        vec![std::ffi::OsStr::new("gateway")]
    );
    let environment = command
        .get_envs()
        .filter_map(|(key, value)| value.map(|value| (key.to_owned(), value.to_owned())))
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(
        environment.get(std::ffi::OsStr::new("HERMES_HOME")),
        Some(&hermes_home.into_os_string())
    );
    assert_eq!(
        environment.get(std::ffi::OsStr::new("CODEX_HOME")),
        Some(&codex_home.into_os_string())
    );
    assert_eq!(
        environment.get(std::ffi::OsStr::new("API_SERVER_HOST")),
        Some(&std::ffi::OsString::from("127.0.0.1"))
    );
    assert_eq!(
        environment.get(std::ffi::OsStr::new("API_SERVER_PORT")),
        Some(&std::ffi::OsString::from("8642"))
    );
    assert!(!environment.contains_key(std::ffi::OsStr::new("OPENAI_API_KEY")));
    assert!(!environment.contains_key(std::ffi::OsStr::new("DATABASE_URL")));
    assert!(!environment.contains_key(std::ffi::OsStr::new("GIT_ASKPASS")));
    assert!(!environment.contains_key(std::ffi::OsStr::new("HOME")));
    assert!(!environment.contains_key(std::ffi::OsStr::new("USERPROFILE")));

    let Err(existing_home_failure) = HermesProcessConfig::new(
        std::env::current_exe().expect("test executable"),
        std::env::temp_dir(),
        scratch.join("other-codex-home"),
        "127.0.0.1:8642".parse().expect("loopback"),
        "process-local-api-key",
        "hermes-agent",
        Duration::from_secs(2),
    ) else {
        panic!("an existing home cannot be reused as ephemeral state");
    };
    assert_eq!(
        existing_home_failure.kind(),
        HermesAdapterErrorKind::Configuration
    );
}

fn config(server: &ScriptedServer) -> HermesAdapterConfig {
    HermesAdapterConfig::new(
        server.address(),
        "test-only-loopback-key",
        Duration::from_secs(2),
        Duration::from_millis(1),
    )
    .expect("loopback config")
}

fn job(request: HermesResearchRequest) -> HermesReflectionJob {
    HermesReflectionJob::new(
        request,
        "lattice-task-034-session",
        "hermes-agent",
        vec![
            ReflectionEvidence::new(
                ReflectionEvidenceKind::Graphify,
                digest(GRAPH_DIGEST),
                "Graphify extracted the adapter boundary from the exact Git snapshot.",
            )
            .expect("bounded evidence"),
        ],
    )
    .expect("valid bound job")
}

fn bound_output(job: &HermesReflectionJob) -> String {
    format!(
        r#"{{"schema_version":"{HERMES_SCHEMA_VERSION}","binding":{{"request_id":"request-hermes-1","task_id":"task-034","attempt_id":"attempt-1","project_snapshot_id":"snapshot-79096b6","subject_digest":"{SUBJECT_DIGEST}","session_id":"lattice-task-034-session","input_digest":"{}","model":"hermes-agent"}},"summary":"The adapter boundary is read-only and fail-closed.","findings":[{{"classification":"inference","statement":"Graphify evidence is bound to the requested snapshot.","evidence_digests":["{GRAPH_DIGEST}"]}}],"next_actions":["Persist only through the later LATTICE Memory port."]}}"#,
        job.input_digest().as_str(),
    )
}

fn capabilities() -> Response {
    Response::json(
        200,
        serde_json::json!({
            "object": "hermes.api_server.capabilities",
            "platform": "hermes-agent",
            "model": "hermes-agent",
            "auth": {"type": "bearer", "required": true},
            "runtime": {
                "mode": "server_agent",
                "tool_execution": "server",
                "split_runtime": false
            },
            "features": {
                "run_submission": true,
                "run_status": true,
                "run_events_sse": true,
                "run_stop": true,
                "admin_config_rw": false,
                "memory_write_api": false
            }
        })
        .to_string(),
    )
}

fn completed_events(run_id: &str, output: &str) -> Response {
    let event = serde_json::json!({
        "event": "run.completed",
        "run_id": run_id,
        "timestamp": 1.0,
        "output": output,
    });
    Response::sse(200, format!("data: {event}\n\n: stream closed\n\n"))
}

fn completed_status(run_id: &str, session_id: &str, output: &str) -> Response {
    Response::json(
        200,
        serde_json::json!({
            "object": "hermes.run",
            "run_id": run_id,
            "status": "completed",
            "session_id": session_id,
            "model": "hermes-agent",
            "output": output,
            "usage": {"input_tokens": 10, "output_tokens": 20, "total_tokens": 30},
        })
        .to_string(),
    )
}

fn request() -> HermesResearchRequest {
    HermesResearchRequest::new(
        Invocation::new(
            CONTRACT_VERSION,
            RequestId::new("request-hermes-1").expect("request id"),
            TaskId::new("task-034").expect("task id"),
            AttemptId::new("attempt-1").expect("attempt id"),
            ProjectSnapshotId::new("snapshot-79096b6").expect("snapshot id"),
            digest(SUBJECT_DIGEST),
        )
        .expect("invocation"),
    )
}

fn digest(value: &str) -> ContentDigest {
    ContentDigest::from_sha256(value.to_owned()).expect("sha256")
}

struct Response {
    status: u16,
    content_type: &'static str,
    body: String,
    delay: Duration,
}

impl Response {
    fn json(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    fn sse(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "text/event-stream",
            body: body.into(),
            delay: Duration::ZERO,
        }
    }

    fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

struct ScriptedServer {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    handle: JoinHandle<()>,
}

impl ScriptedServer {
    fn start(responses: Vec<Response>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback fixture");
        let address = listener.local_addr().expect("fixture address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let handle = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("fixture accept");
                let request = read_request(&mut stream);
                captured.lock().expect("request lock").push(request);
                if !response.delay.is_zero() {
                    thread::sleep(response.delay);
                }
                write_response(&mut stream, &response);
            }
        });
        Self {
            address,
            requests,
            handle,
        }
    }

    fn address(&self) -> SocketAddr {
        self.address
    }

    fn finish(self) -> Vec<String> {
        self.handle.join().expect("fixture server");
        Arc::try_unwrap(self.requests)
            .expect("sole request owner")
            .into_inner()
            .expect("request lock")
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    let header_end = loop {
        let count = stream.read(&mut buffer).expect("read request");
        assert!(count > 0, "request closed before headers");
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(index) = find_bytes(&bytes, b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8(bytes[..header_end].to_vec()).expect("utf8 headers");
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.strip_prefix("Content-Length: ")
                .and_then(|value| value.parse::<usize>().ok())
        })
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let count = stream.read(&mut buffer).expect("read body");
        assert!(count > 0, "request closed before body");
        bytes.extend_from_slice(&buffer[..count]);
    }
    String::from_utf8(bytes).expect("utf8 request")
}

fn write_response(stream: &mut TcpStream, response: &Response) {
    let reason = match response.status {
        200 => "OK",
        202 => "Accepted",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        response.content_type,
        response.body.len()
    );
    if stream.write_all(head.as_bytes()).is_ok() {
        let _ = stream.write_all(response.body.as_bytes());
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
