use std::env;
use std::error::Error;
use std::io::{self, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use lattice_contracts::{
    ContentDigest, GatewayActorId, GatewayActorKind, GatewayAdapterId, GatewayChannelId,
    GatewayClientKind, GatewayInstanceId, GatewayPeerContext, GatewayReply, GatewayReplyBody,
    GatewayRequest, GatewayRequestBody, GatewaySessionId, GatewayStatusObservation, ProjectId,
    ProjectSnapshotId, SubjectBinding, TaskId, TaskSpecSubmission,
};
use lattice_gateway_ipc::{build_reply, task_spec_document_digest};
use lattice_openclaw_adapter::{
    AuthenticationKey, OPENCLAW_OFFICIAL_ENTRYPOINT, OPENCLAW_OFFICIAL_PACKAGE_INTEGRITY,
    OPENCLAW_OFFICIAL_PACKAGE_LICENSE, OPENCLAW_OFFICIAL_PACKAGE_NAME,
    OPENCLAW_OFFICIAL_PACKAGE_VERSION, OPENCLAW_OFFICIAL_SOURCE_COMMIT, OpenClawGatewayConfig,
    OpenClawGatewayServer, OpenClawLaunchAttestationKey, OpenClawLaunchAttestationTag,
    OpenClawOfficialLaunchEvidence, OpenClawOfficialLaunchRecord, OpenClawProcessStartNonce,
};
use lattice_ports::{GatewayService, GatewayServiceError, GatewayServiceResult, PortErrorKind};

const AUTH_KEY_ENV: &str = "LATTICE_OPENCLAW_AUTH_KEY_HEX";
const DEADLINE_ENV: &str = "LATTICE_OPENCLAW_DEADLINE_MS";
const ENTRYPOINT_DIGEST_ENV: &str = "LATTICE_OPENCLAW_ENTRYPOINT_SHA256";
const GATEWAY_PORT_ENV: &str = "LATTICE_OPENCLAW_GATEWAY_PORT";
const LAUNCH_ATTESTATION_KEY_ENV: &str = "LATTICE_OPENCLAW_LAUNCH_ATTESTATION_KEY_HEX";
const LAUNCH_ATTESTATION_TAG_ENV: &str = "LATTICE_OPENCLAW_LAUNCH_ATTESTATION_TAG_HEX";
const LAUNCH_RECORD_ID_ENV: &str = "LATTICE_OPENCLAW_LAUNCH_RECORD_ID";
const PACKAGE_DIGEST_ENV: &str = "LATTICE_OPENCLAW_PACKAGE_TARBALL_SHA256";
const PROCESS_ID_ENV: &str = "LATTICE_OPENCLAW_PROCESS_ID";
const PROCESS_NONCE_ENV: &str = "LATTICE_OPENCLAW_PROCESS_START_NONCE";
const PROFILE_DIGEST_ENV: &str = "LATTICE_OPENCLAW_PROFILE_SHA256";

fn digest(fill: char) -> ContentDigest {
    ContentDigest::from_sha256(fill.to_string().repeat(64)).expect("non-zero fixture digest")
}

fn frozen_submission() -> TaskSpecSubmission {
    let document = br#"{"project_id":"project-a","project_snapshot_id":"snapshot-a","revision":"1","schema_version":"2.1","task_id":"task-a"}"#.to_vec();
    let document_digest = task_spec_document_digest(&document).expect("fixed Task Spec digest");
    let binding = SubjectBinding::new(
        ProjectId::new("project-a").expect("project"),
        ProjectSnapshotId::new("snapshot-a").expect("snapshot"),
        TaskId::new("task-a").expect("task"),
        "1",
        document_digest.clone(),
    )
    .expect("fixed binding");
    TaskSpecSubmission::new(binding, document, document_digest).expect("fixed submission")
}

fn transport_peer() -> GatewayPeerContext {
    GatewayPeerContext::new_fake(
        GatewayClientKind::OpenClaw,
        GatewayInstanceId::new("gateway-official-preflight").expect("gateway"),
        GatewayAdapterId::new("openclaw-lattice-plugin").expect("adapter"),
        "1.0.0",
        digest('1'),
        digest('2'),
        GatewayActorId::new("responsible-user-preflight").expect("actor"),
        GatewayActorKind::ResponsibleUser,
        GatewayChannelId::new("openclaw-official-loopback").expect("channel"),
        GatewaySessionId::new("session-official-preflight").expect("session"),
        1,
        digest('3'),
        digest('3'),
    )
    .expect("transport-only peer")
}

#[derive(Default)]
struct PreflightRecordingService {
    observations: Vec<String>,
}

impl GatewayService for PreflightRecordingService {
    fn handle(
        &mut self,
        _peer: GatewayPeerContext,
        request: GatewayRequest,
    ) -> GatewayServiceResult<GatewayReply> {
        let body = match request.body() {
            GatewayRequestBody::Status(target) => {
                self.observations
                    .push(format!("status:{}", target.project_id().as_str()));
                GatewayReplyBody::StatusObserved(GatewayStatusObservation::Project {
                    project_id: target.project_id().clone(),
                    tasks: Vec::new(),
                    next_cursor: None,
                })
            }
            GatewayRequestBody::Submit(submission) => {
                self.observations.push(format!(
                    "submit:{}",
                    submission.claimed_spec_digest().as_str()
                ));
                GatewayReplyBody::SubmitAccepted {
                    binding: submission.binding().clone(),
                    command_receipt_digest: digest('9'),
                }
            }
            GatewayRequestBody::Plan(_)
            | GatewayRequestBody::Approve(_)
            | GatewayRequestBody::Reject(_)
            | GatewayRequestBody::Stop(_) => {
                return Err(GatewayServiceError::new(
                    PortErrorKind::Denied,
                    "OPENCLAW_PREFLIGHT_ACTION_DENIED",
                ));
            }
        };
        build_reply(&request, body)
            .map_err(|error| GatewayServiceError::new(PortErrorKind::Malformed, error.code()))
    }
}

fn required_env(name: &'static str) -> Result<String, Box<dyn Error>> {
    env::var(name).map_err(|_| format!("missing required environment variable {name}").into())
}

fn parse_lowercase_hex<const N: usize>(name: &'static str) -> Result<[u8; N], Box<dyn Error>> {
    let value = required_env(name)?;
    if value.len() != N * 2
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{name} must be exactly {} lowercase hex characters", N * 2).into());
    }
    let mut bytes = [0_u8; N];
    for (index, output) in bytes.iter_mut().enumerate() {
        let start = index * 2;
        *output = u8::from_str_radix(&value[start..start + 2], 16)?;
    }
    Ok(bytes)
}

fn content_digest_env(name: &'static str) -> Result<ContentDigest, Box<dyn Error>> {
    ContentDigest::from_sha256(required_env(name)?).map_err(|error| error.to_string().into())
}

fn main() -> Result<(), Box<dyn Error>> {
    let port = required_env(GATEWAY_PORT_ENV)?.parse::<u16>()?;
    if port == 0 {
        return Err(format!("{GATEWAY_PORT_ENV} must be non-zero").into());
    }
    let deadline_ms = required_env(DEADLINE_ENV)?.parse::<u64>()?;
    let timeout = Duration::from_millis(deadline_ms);
    let authentication_key = AuthenticationKey::new(parse_lowercase_hex::<32>(AUTH_KEY_ENV)?)?;
    let launch_attestation_key =
        OpenClawLaunchAttestationKey::new(parse_lowercase_hex::<32>(LAUNCH_ATTESTATION_KEY_ENV)?)?;
    let launch_attestation_tag =
        OpenClawLaunchAttestationTag::new(parse_lowercase_hex::<32>(LAUNCH_ATTESTATION_TAG_ENV)?)?;
    let launch_evidence = OpenClawOfficialLaunchEvidence::new(
        required_env(LAUNCH_RECORD_ID_ENV)?,
        required_env(PROCESS_ID_ENV)?.parse::<u32>()?,
        OpenClawProcessStartNonce::new(parse_lowercase_hex::<16>(PROCESS_NONCE_ENV)?)?,
        content_digest_env(PACKAGE_DIGEST_ENV)?,
        content_digest_env(ENTRYPOINT_DIGEST_ENV)?,
        content_digest_env(PROFILE_DIGEST_ENV)?,
    )?;
    let launch_record = OpenClawOfficialLaunchRecord::verify_lattice_attestation(
        launch_evidence,
        &launch_attestation_key,
        launch_attestation_tag,
    )?;
    let submission = frozen_submission();
    let submit_digest = submission.claimed_spec_digest().as_str().to_owned();
    let config = OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        timeout,
        ProjectId::new("project-a")?,
        transport_peer(),
        authentication_key,
    )?
    .with_frozen_submission(submission)?;
    let mut server = OpenClawGatewayServer::bind_official_launch(
        config,
        PreflightRecordingService::default(),
        launch_record.clone(),
    )?;
    let endpoint = server.local_addr()?;

    println!(
        "{{\"classification\":\"official-package-preflight-only\",\"durability\":\"process-memory\",\"endpoint\":\"{}\",\"entrypoint\":\"{}\",\"event\":\"ready\",\"launch_record_id\":\"{}\",\"package\":\"{}\",\"package_integrity\":\"{}\",\"package_license\":\"{}\",\"package_version\":\"{}\",\"runtime_kind\":\"Fake\",\"source_commit\":\"{}\",\"submit_digest\":\"{}\"}}",
        endpoint,
        OPENCLAW_OFFICIAL_ENTRYPOINT,
        launch_record.launch_record_id(),
        OPENCLAW_OFFICIAL_PACKAGE_NAME,
        OPENCLAW_OFFICIAL_PACKAGE_INTEGRITY,
        OPENCLAW_OFFICIAL_PACKAGE_LICENSE,
        OPENCLAW_OFFICIAL_PACKAGE_VERSION,
        OPENCLAW_OFFICIAL_SOURCE_COMMIT,
        submit_digest,
    );
    io::stdout().flush()?;

    server.serve_once()?;
    server.serve_once()?;
    let service = server
        .service()
        .ok_or("preflight service remained ambiguously owned")?;
    if service.observations.len() != 2 {
        return Err("preflight did not record exactly two commands".into());
    }
    println!(
        "{{\"classification\":\"official-package-preflight-only\",\"durability\":\"process-memory\",\"event\":\"complete\",\"observations\":[\"{}\",\"{}\"],\"runtime_kind\":\"Fake\"}}",
        service.observations[0], service.observations[1]
    );
    io::stdout().flush()?;
    Ok(())
}
