//! Authenticated, loopback-only `OpenClaw` transport for the frozen gateway IPC.

mod wire;

pub use wire::{
    OpenClawStatusRequest, OpenClawStopRequest, OpenClawSubmitReply, OpenClawSubmitReplyBody,
    OpenClawSubmitRequest, encode_openclaw_client_hello, encode_openclaw_status_request,
    encode_openclaw_stop_request, encode_openclaw_submit_request,
};

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

use lattice_contracts::{
    ContentDigest, GatewayClientKind, GatewayDenialCode, GatewayPeerContext, GatewayReply,
    GatewayReplyBody, GatewayRequest, GatewayRequestBody, GatewayUnknownCode, ProjectId,
    RuntimeKind, TaskSpecSubmission,
};
use lattice_gateway_ipc::{
    MAX_FRAME_BYTES, build_reply, build_request, decode_reply, decode_request, encode_reply,
    verify_task_spec_document,
};
use lattice_ports::{GatewayService, PortErrorKind};
pub use lattice_ports::{
    OpenClawCommandScope, OpenClawIdempotencyDecision, OpenClawIdempotencyDurability,
    OpenClawIdempotencyError, OpenClawIdempotencyStore, OpenClawTerminalCommandRecord,
};
use sha2::{Digest, Sha256};

/// Authenticated request-packet magic for wire protocol version 1.
pub const OPENCLAW_WIRE_REQUEST_MAGIC: [u8; 8] = *b"LATGW001";
/// Authenticated response-packet magic for wire protocol version 1.
pub const OPENCLAW_WIRE_RESPONSE_MAGIC: [u8; 8] = *b"LATGR001";
/// Server session-greeting magic for wire protocol version 1.
pub const OPENCLAW_WIRE_SESSION_MAGIC: [u8; 8] = *b"LATSN001";

/// Deterministic root key used only by the cross-language parity fixture.
pub const OPENCLAW_PARITY_ROOT_KEY_HEX: &str =
    "1111111111111111111111111111111111111111111111111111111111111111";
/// Deterministic session epoch used only by the cross-language parity fixture.
pub const OPENCLAW_PARITY_SESSION_EPOCH_HEX: &str = "000102030405060708090a0b0c0d0e0f";
/// Deterministic packet nonce used only by the cross-language parity fixture.
pub const OPENCLAW_PARITY_NONCE_HEX: &str = "101112131415161718191a1b1c1d1e1f";
/// Complete session greeting golden bytes, encoded as lowercase hex.
pub const OPENCLAW_PARITY_SESSION_GREETING_HEX: &str =
    "4c4154534e303031000102030405060708090a0b0c0d0e0f";
/// Canonical status-command request payload used by the packet golden.
pub const OPENCLAW_PARITY_STATUS_COMMAND_REQUEST_JSON: &str = "{\"action\":\"status\",\"body\":{\"kind\":\"command\",\"project_id\":\"project-a\",\"target_command_id\":\"target-command-a\"},\"command_id\":\"command-status-a\",\"correlation_id\":\"correlation-status-a\",\"protocol\":\"lattice-openclaw-inbound\",\"version\":\"1\"}";
/// Complete authenticated request packet golden, encoded as lowercase hex.
pub const OPENCLAW_PARITY_REQUEST_PACKET_HEX: &str = "4c41544757303031000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f000000e98008e8f0a4240aa68620878b3dfd255d5f707d855e9eb0027d7ce3ce921374317b22616374696f6e223a22737461747573222c22626f6479223a7b226b696e64223a22636f6d6d616e64222c2270726f6a6563745f6964223a2270726f6a6563742d61222c227461726765745f636f6d6d616e645f6964223a227461726765742d636f6d6d616e642d61227d2c22636f6d6d616e645f6964223a22636f6d6d616e642d7374617475732d61222c22636f7272656c6174696f6e5f6964223a22636f7272656c6174696f6e2d7374617475732d61222c2270726f746f636f6c223a226c6174746963652d6f70656e636c61772d696e626f756e64222c2276657273696f6e223a2231227d";
/// Canonical status-command response payload used by the packet golden.
pub const OPENCLAW_PARITY_STATUS_COMMAND_RESPONSE_JSON: &str = "{\"action\":\"status\",\"body\":{\"code\":\"DOWNSTREAM_DENIED\",\"kind\":\"denied\"},\"command_id\":\"command-status-a\",\"correlation_id\":\"correlation-status-a\",\"protocol\":\"lattice-gateway-ipc\",\"reply_digest\":\"433253c474608f0d74306dc23042636c44de19d070016b8537467a305b071d89\",\"request_digest\":\"b23c60531600afa1fc45996a5e810231dba0a90b20b7bf90d5fc98ab4de05bb6\",\"version\":\"1\"}";
/// Complete authenticated response packet golden, encoded as lowercase hex.
pub const OPENCLAW_PARITY_RESPONSE_PACKET_HEX: &str = "4c41544752303031000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f0000016467d8a3a47e3b2e9f128b7ec8e42fda071d0405d66e5e82b01f756466bfec6eb77b22616374696f6e223a22737461747573222c22626f6479223a7b22636f6465223a22444f574e53545245414d5f44454e494544222c226b696e64223a2264656e696564227d2c22636f6d6d616e645f6964223a22636f6d6d616e642d7374617475732d61222c22636f7272656c6174696f6e5f6964223a22636f7272656c6174696f6e2d7374617475732d61222c2270726f746f636f6c223a226c6174746963652d676174657761792d697063222c227265706c795f646967657374223a2234333332353363343734363038663064373433303664633233303432363336633434646531396430373030313662383533373436376133303562303731643839222c22726571756573745f646967657374223a2262323363363035333136303061666131666334353939366135653831303233316462613061393062323062376266393064356663393861623464653035626236222c2276657273696f6e223a2231227d";
/// JSON copy of all parity inputs and goldens for non-Rust consumers.
pub const OPENCLAW_WIRE_PARITY_GOLDEN_JSON: &str =
    include_str!("../tests/fixtures/openclaw_wire_parity.json");

/// Official stable package name pinned by the LATTICE launcher.
pub const OPENCLAW_OFFICIAL_PACKAGE_NAME: &str = "openclaw";
/// Official stable package version pinned by the LATTICE launcher.
pub const OPENCLAW_OFFICIAL_PACKAGE_VERSION: &str = "2026.7.1-2";
/// Verified official source commit pinned by the LATTICE launcher.
pub const OPENCLAW_OFFICIAL_SOURCE_COMMIT: &str = "0790d9f593ad30c940ed93b5872a8cf6d6f3cf8c";
/// Verified official package license pinned by the LATTICE launcher.
pub const OPENCLAW_OFFICIAL_PACKAGE_LICENSE: &str = "MIT";
/// Registry integrity pinned by the LATTICE launcher for the official package.
pub const OPENCLAW_OFFICIAL_PACKAGE_INTEGRITY: &str = "sha512-ycF3yPcbjN6bUPeaUx6Mh6vze1hQWoD3CT/wWcmD7a8xaHHHRUaAlaq+lFxMHf1ssEgODVAwjlzYqp2twkYZ7g==";
/// Official package CLI entrypoint pinned by the LATTICE launcher.
pub const OPENCLAW_OFFICIAL_ENTRYPOINT: &str = "openclaw.mjs";
/// Domain prefix for the LATTICE-owned official-launch HMAC.
pub const OPENCLAW_LAUNCH_ATTESTATION_DOMAIN: &str = "lattice-openclaw-launch-attestation-v1";
/// Closed protocol identifier for the first authenticated official-mode frame.
pub const OPENCLAW_CLIENT_HELLO_PROTOCOL: &str = "lattice-openclaw-client-hello";

const REQUEST_MAGIC: [u8; 8] = OPENCLAW_WIRE_REQUEST_MAGIC;
const RESPONSE_MAGIC: [u8; 8] = OPENCLAW_WIRE_RESPONSE_MAGIC;
const SESSION_MAGIC: [u8; 8] = OPENCLAW_WIRE_SESSION_MAGIC;
const SESSION_EPOCH_BYTES: usize = 16;
const NONCE_BYTES: usize = 16;
const TAG_BYTES: usize = 32;
const LENGTH_BYTES: usize = 4;
const HEADER_BYTES: usize =
    REQUEST_MAGIC.len() + SESSION_EPOCH_BYTES + NONCE_BYTES + LENGTH_BYTES + TAG_BYTES;
const SESSION_GREETING_BYTES: usize = SESSION_MAGIC.len() + SESSION_EPOCH_BYTES;
const HMAC_BLOCK_BYTES: usize = 64;
const MAX_TIMEOUT: Duration = Duration::from_secs(30);
const PROCESS_START_NONCE_BYTES: usize = 16;
const MAX_LAUNCH_RECORD_ID_BYTES: usize = 128;

/// Independent 256-bit key held only by the LATTICE-owned launcher and verifier.
///
/// This key is deliberately a distinct type from [`AuthenticationKey`], which
/// prevents a plugin transport key from satisfying the launch-evidence gate.
#[derive(Eq, PartialEq)]
pub struct OpenClawLaunchAttestationKey([u8; TAG_BYTES]);

impl OpenClawLaunchAttestationKey {
    /// Constructs a non-zero launch-attestation key.
    ///
    /// # Errors
    ///
    /// Rejects the all-zero sentinel.
    pub fn new(bytes: [u8; TAG_BYTES]) -> Result<Self, GatewayTransportError> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Configuration,
            ));
        }
        Ok(Self(bytes))
    }

    fn bytes(&self) -> &[u8; TAG_BYTES] {
        &self.0
    }
}

impl fmt::Debug for OpenClawLaunchAttestationKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OpenClawLaunchAttestationKey([REDACTED])")
    }
}

/// LATTICE launcher HMAC over one exact official-process evidence record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenClawLaunchAttestationTag([u8; TAG_BYTES]);

impl OpenClawLaunchAttestationTag {
    /// Wraps a non-zero HMAC produced by the LATTICE-owned launcher.
    ///
    /// # Errors
    ///
    /// Rejects the all-zero sentinel.
    pub fn new(bytes: [u8; TAG_BYTES]) -> Result<Self, GatewayTransportError> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Configuration,
            ));
        }
        Ok(Self(bytes))
    }
}

/// Per-process nonce issued by the LATTICE-owned launcher, not by the plugin.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenClawProcessStartNonce([u8; PROCESS_START_NONCE_BYTES]);

impl OpenClawProcessStartNonce {
    /// Constructs a non-zero process-start nonce.
    ///
    /// # Errors
    ///
    /// Rejects the all-zero sentinel.
    pub fn new(bytes: [u8; PROCESS_START_NONCE_BYTES]) -> Result<Self, GatewayTransportError> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Configuration,
            ));
        }
        Ok(Self(bytes))
    }

    fn bytes(self) -> [u8; PROCESS_START_NONCE_BYTES] {
        self.0
    }
}

/// Closed authenticated hello. It carries no caller-supplied package or runtime claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenClawClientHello {
    launch_record_id: String,
    process_start_nonce: OpenClawProcessStartNonce,
}

impl OpenClawClientHello {
    /// Returns the LATTICE-issued launch-record identity.
    #[must_use]
    pub fn launch_record_id(&self) -> &str {
        &self.launch_record_id
    }

    /// Returns the LATTICE-issued process-start nonce.
    #[must_use]
    pub const fn process_start_nonce(&self) -> OpenClawProcessStartNonce {
        self.process_start_nonce
    }
}

/// Untrusted observed fields for one isolated official package launch.
///
/// Constructing this value does not attest or promote the process. It becomes
/// trusted only after [`OpenClawOfficialLaunchRecord::verify_lattice_attestation`]
/// authenticates every field with the launcher-owned key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenClawOfficialLaunchEvidence {
    launch_record_id: String,
    process_id: u32,
    process_start_nonce: OpenClawProcessStartNonce,
    package_tarball_digest: ContentDigest,
    entrypoint_digest: ContentDigest,
    isolated_profile_digest: ContentDigest,
}

impl OpenClawOfficialLaunchEvidence {
    /// Parses bounded observed fields without claiming launcher provenance.
    ///
    /// # Errors
    ///
    /// Rejects unsafe record IDs, zero process IDs, and sentinel digests.
    pub fn new(
        launch_record_id: impl Into<String>,
        process_id: u32,
        process_start_nonce: OpenClawProcessStartNonce,
        package_tarball_digest: ContentDigest,
        entrypoint_digest: ContentDigest,
        isolated_profile_digest: ContentDigest,
    ) -> Result<Self, GatewayTransportError> {
        let launch_record_id = launch_record_id.into();
        validate_launch_record_fields(
            &launch_record_id,
            process_id,
            &package_tarball_digest,
            &entrypoint_digest,
            &isolated_profile_digest,
        )?;
        Ok(Self {
            launch_record_id,
            process_id,
            process_start_nonce,
            package_tarball_digest,
            entrypoint_digest,
            isolated_profile_digest,
        })
    }
}

/// LATTICE-owned evidence for one isolated official package launch.
///
/// Package identity, version, registry integrity, and entrypoint are compile-time
/// pins. The plugin can only prove possession of this record's ID and start nonce;
/// it cannot self-assert package or `RuntimeKind::Live` evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenClawOfficialLaunchRecord {
    evidence: OpenClawOfficialLaunchEvidence,
}

impl OpenClawOfficialLaunchRecord {
    /// Verifies and creates one launcher-owned official process record.
    ///
    /// # Errors
    ///
    /// Rejects invalid evidence fields or an HMAC that does not bind every field
    /// and immutable official package pin. The transport authentication key is
    /// not accepted by this API.
    pub fn verify_lattice_attestation(
        evidence: OpenClawOfficialLaunchEvidence,
        attestation_key: &OpenClawLaunchAttestationKey,
        attestation_tag: OpenClawLaunchAttestationTag,
    ) -> Result<Self, GatewayTransportError> {
        let expected_tag = launch_attestation_tag(attestation_key, &evidence)?;
        if !constant_time_eq(&expected_tag, &attestation_tag.0) {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Authentication,
            ));
        }
        Ok(Self { evidence })
    }

    /// Returns the launcher-issued record identity.
    #[must_use]
    pub fn launch_record_id(&self) -> &str {
        &self.evidence.launch_record_id
    }

    /// Returns the observed official process ID.
    #[must_use]
    pub const fn process_id(&self) -> u32 {
        self.evidence.process_id
    }

    /// Returns the verified package tarball digest.
    #[must_use]
    pub const fn package_tarball_digest(&self) -> &ContentDigest {
        &self.evidence.package_tarball_digest
    }

    /// Returns the verified entrypoint digest.
    #[must_use]
    pub const fn entrypoint_digest(&self) -> &ContentDigest {
        &self.evidence.entrypoint_digest
    }

    /// Returns the verified isolated-profile configuration digest.
    #[must_use]
    pub const fn isolated_profile_digest(&self) -> &ContentDigest {
        &self.evidence.isolated_profile_digest
    }

    /// Returns the immutable official package name pin.
    #[must_use]
    pub const fn package_name(&self) -> &'static str {
        OPENCLAW_OFFICIAL_PACKAGE_NAME
    }

    /// Returns the immutable official package version pin.
    #[must_use]
    pub const fn package_version(&self) -> &'static str {
        OPENCLAW_OFFICIAL_PACKAGE_VERSION
    }

    /// Returns the immutable verified source commit pin.
    #[must_use]
    pub const fn source_commit(&self) -> &'static str {
        OPENCLAW_OFFICIAL_SOURCE_COMMIT
    }

    /// Returns the immutable verified package license pin.
    #[must_use]
    pub const fn package_license(&self) -> &'static str {
        OPENCLAW_OFFICIAL_PACKAGE_LICENSE
    }

    /// Returns the immutable registry integrity pin.
    #[must_use]
    pub const fn package_integrity(&self) -> &'static str {
        OPENCLAW_OFFICIAL_PACKAGE_INTEGRITY
    }

    /// Returns the immutable package entrypoint pin.
    #[must_use]
    pub const fn entrypoint(&self) -> &'static str {
        OPENCLAW_OFFICIAL_ENTRYPOINT
    }

    /// Builds the only `ClientHello` accepted for this launch record.
    #[must_use]
    pub fn client_hello(&self) -> OpenClawClientHello {
        OpenClawClientHello {
            launch_record_id: self.evidence.launch_record_id.clone(),
            process_start_nonce: self.evidence.process_start_nonce,
        }
    }
}

fn validate_launch_record_fields(
    launch_record_id: &str,
    process_id: u32,
    package_tarball_digest: &ContentDigest,
    entrypoint_digest: &ContentDigest,
    isolated_profile_digest: &ContentDigest,
) -> Result<(), GatewayTransportError> {
    if launch_record_id.is_empty()
        || launch_record_id.len() > MAX_LAUNCH_RECORD_ID_BYTES
        || !launch_record_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || matches!(launch_record_id, "." | "..")
        || process_id == 0
        || [
            package_tarball_digest,
            entrypoint_digest,
            isolated_profile_digest,
        ]
        .into_iter()
        .any(|digest| digest.as_str().bytes().all(|byte| byte == b'0'))
    {
        return Err(GatewayTransportError::new(
            GatewayTransportErrorKind::Configuration,
        ));
    }
    Ok(())
}

fn append_launch_attestation_field(
    payload: &mut Vec<u8>,
    name: &str,
    value: &[u8],
) -> Result<(), GatewayTransportError> {
    let name_length = u32::try_from(name.len())
        .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Configuration))?;
    let value_length = u32::try_from(value.len())
        .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Configuration))?;
    payload.extend_from_slice(&name_length.to_be_bytes());
    payload.extend_from_slice(name.as_bytes());
    payload.extend_from_slice(&value_length.to_be_bytes());
    payload.extend_from_slice(value);
    Ok(())
}

fn launch_attestation_tag(
    key: &OpenClawLaunchAttestationKey,
    evidence: &OpenClawOfficialLaunchEvidence,
) -> Result<[u8; TAG_BYTES], GatewayTransportError> {
    let mut payload = Vec::with_capacity(768);
    payload.extend_from_slice(OPENCLAW_LAUNCH_ATTESTATION_DOMAIN.as_bytes());
    payload.push(0);
    append_launch_attestation_field(
        &mut payload,
        "launch_record_id",
        evidence.launch_record_id.as_bytes(),
    )?;
    append_launch_attestation_field(
        &mut payload,
        "process_id",
        &evidence.process_id.to_be_bytes(),
    )?;
    append_launch_attestation_field(
        &mut payload,
        "process_start_nonce",
        &evidence.process_start_nonce.bytes(),
    )?;
    append_launch_attestation_field(
        &mut payload,
        "package_name",
        OPENCLAW_OFFICIAL_PACKAGE_NAME.as_bytes(),
    )?;
    append_launch_attestation_field(
        &mut payload,
        "package_version",
        OPENCLAW_OFFICIAL_PACKAGE_VERSION.as_bytes(),
    )?;
    append_launch_attestation_field(
        &mut payload,
        "source_commit",
        OPENCLAW_OFFICIAL_SOURCE_COMMIT.as_bytes(),
    )?;
    append_launch_attestation_field(
        &mut payload,
        "package_license",
        OPENCLAW_OFFICIAL_PACKAGE_LICENSE.as_bytes(),
    )?;
    append_launch_attestation_field(
        &mut payload,
        "package_integrity",
        OPENCLAW_OFFICIAL_PACKAGE_INTEGRITY.as_bytes(),
    )?;
    append_launch_attestation_field(
        &mut payload,
        "entrypoint",
        OPENCLAW_OFFICIAL_ENTRYPOINT.as_bytes(),
    )?;
    append_launch_attestation_field(
        &mut payload,
        "package_tarball_digest",
        evidence.package_tarball_digest.as_str().as_bytes(),
    )?;
    append_launch_attestation_field(
        &mut payload,
        "entrypoint_digest",
        evidence.entrypoint_digest.as_str().as_bytes(),
    )?;
    append_launch_attestation_field(
        &mut payload,
        "isolated_profile_digest",
        evidence.isolated_profile_digest.as_str().as_bytes(),
    )?;
    Ok(hmac_sha256(key.bytes(), &payload))
}

fn hmac_sha256(key: &[u8; TAG_BYTES], payload: &[u8]) -> [u8; TAG_BYTES] {
    let mut inner_pad = [0x36_u8; HMAC_BLOCK_BYTES];
    let mut outer_pad = [0x5c_u8; HMAC_BLOCK_BYTES];
    for (index, byte) in key.iter().copied().enumerate() {
        inner_pad[index] ^= byte;
        outer_pad[index] ^= byte;
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(payload);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    let digest = outer.finalize();
    let mut tag = [0_u8; TAG_BYTES];
    tag.copy_from_slice(&digest);
    tag
}

/// Maximum authenticated transport nonces retained for one server process.
pub const MAX_AUTH_REPLAY_ENTRIES: usize = 4_096;
/// Maximum terminal replies retained by the process-memory checkpoint store.
pub const MAX_COMMAND_REPLAY_ENTRIES: usize = 1_024;
/// Maximum immutable Task Specs preloaded into one transport process.
pub const MAX_FROZEN_SUBMISSIONS: usize = 8;
/// Maximum total canonical Task Spec bytes retained by one transport process.
pub const MAX_FROZEN_SUBMISSION_BYTES: usize = 4 * MAX_FRAME_BYTES;
/// Maximum concurrent unauthenticated connection readers.
pub const MAX_INFLIGHT_CONNECTIONS: usize = 8;

enum ProcessMemoryOpenClawIdempotencyEntry {
    Claimed(ContentDigest),
    Terminal(Box<OpenClawTerminalCommandRecord>),
}

/// Explicitly non-durable store used by transport-only tests/checkpoints.
#[derive(Default)]
pub struct ProcessMemoryOpenClawIdempotencyStore {
    records: BTreeMap<OpenClawCommandScope, ProcessMemoryOpenClawIdempotencyEntry>,
}

impl OpenClawIdempotencyStore for ProcessMemoryOpenClawIdempotencyStore {
    fn durability(&self) -> OpenClawIdempotencyDurability {
        OpenClawIdempotencyDurability::ProcessMemory
    }

    fn reconcile_and_claim(
        &mut self,
        scope: &OpenClawCommandScope,
        request: &GatewayRequest,
    ) -> Result<OpenClawIdempotencyDecision, OpenClawIdempotencyError> {
        if request.project_id() != scope.project_id() || request.command_id() != scope.command_id()
        {
            return Err(OpenClawIdempotencyError::Malformed);
        }
        let request_digest = request.request_digest();
        if let Some(entry) = self.records.get(scope) {
            return Ok(match entry {
                ProcessMemoryOpenClawIdempotencyEntry::Claimed(digest)
                    if digest == request_digest =>
                {
                    OpenClawIdempotencyDecision::InFlight
                }
                ProcessMemoryOpenClawIdempotencyEntry::Terminal(record)
                    if record.request_digest() == request_digest =>
                {
                    OpenClawIdempotencyDecision::Exact(Box::new(record.reply().clone()))
                }
                ProcessMemoryOpenClawIdempotencyEntry::Claimed(_)
                | ProcessMemoryOpenClawIdempotencyEntry::Terminal(_) => {
                    OpenClawIdempotencyDecision::CommandSubstitution
                }
            });
        }
        if self.records.len() >= MAX_COMMAND_REPLAY_ENTRIES {
            return Err(OpenClawIdempotencyError::Capacity);
        }
        self.records.insert(
            scope.clone(),
            ProcessMemoryOpenClawIdempotencyEntry::Claimed(request_digest.clone()),
        );
        Ok(OpenClawIdempotencyDecision::Claimed)
    }

    fn finalize_terminal(
        &mut self,
        record: OpenClawTerminalCommandRecord,
    ) -> Result<(), OpenClawIdempotencyError> {
        match self.records.get(record.scope()) {
            Some(ProcessMemoryOpenClawIdempotencyEntry::Claimed(digest))
                if digest == record.request_digest() =>
            {
                self.records.insert(
                    record.scope().clone(),
                    ProcessMemoryOpenClawIdempotencyEntry::Terminal(Box::new(record)),
                );
                Ok(())
            }
            Some(ProcessMemoryOpenClawIdempotencyEntry::Terminal(existing))
                if existing.as_ref() == &record =>
            {
                Ok(())
            }
            Some(
                ProcessMemoryOpenClawIdempotencyEntry::Claimed(_)
                | ProcessMemoryOpenClawIdempotencyEntry::Terminal(_),
            )
            | None => Err(OpenClawIdempotencyError::Malformed),
        }
    }
}

struct PendingDispatch<S> {
    receiver: Receiver<(S, lattice_ports::GatewayServiceResult<GatewayReply>)>,
    request: GatewayRequest,
    replay_key: OpenClawCommandScope,
    reply_encoding: ReplyEncoding,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReplyEncoding {
    Gateway,
    OpenClawSubmit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TransportSessionEpoch([u8; SESSION_EPOCH_BYTES]);

struct PreparedConnection {
    stream: TcpStream,
    hello_nonce: Option<[u8; NONCE_BYTES]>,
    nonce: [u8; NONCE_BYTES],
    frame: Vec<u8>,
    deadline: Instant,
}

type ConnectionResult = Result<PreparedConnection, GatewayTransportError>;

/// Process-owned 256-bit authentication key, never serialized in gateway data.
#[derive(Clone, Eq, PartialEq)]
pub struct AuthenticationKey([u8; TAG_BYTES]);

impl AuthenticationKey {
    /// Constructs a non-zero fixed-size authentication key.
    ///
    /// # Errors
    ///
    /// Rejects the all-zero sentinel.
    pub fn new(bytes: [u8; TAG_BYTES]) -> Result<Self, GatewayTransportError> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Configuration,
            ));
        }
        Ok(Self(bytes))
    }

    fn bytes(&self) -> &[u8; TAG_BYTES] {
        &self.0
    }
}

impl fmt::Debug for AuthenticationKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthenticationKey([REDACTED])")
    }
}

/// One caller-generated 128-bit nonce used exactly once per authenticated frame.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TransportNonce([u8; NONCE_BYTES]);

impl TransportNonce {
    /// Constructs a non-zero nonce.
    ///
    /// # Errors
    ///
    /// Rejects the all-zero sentinel.
    pub fn new(bytes: [u8; NONCE_BYTES]) -> Result<Self, GatewayTransportError> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Configuration,
            ));
        }
        Ok(Self(bytes))
    }

    fn bytes(self) -> [u8; NONCE_BYTES] {
        self.0
    }
}

/// Stable fail-closed transport classification without payload or secret data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatewayTransportErrorKind {
    /// Process-owned configuration is invalid.
    Configuration,
    /// A listener or connection could not be established.
    Unavailable,
    /// A non-loopback endpoint or peer was observed.
    NonLocal,
    /// A bounded read, write, or service deadline expired.
    Timeout,
    /// A dispatched service call exceeded its deadline and must be reconciled.
    Ambiguous,
    /// The transport header or stream shape was malformed.
    Malformed,
    /// The frame authentication tag was invalid.
    Authentication,
    /// An authenticated nonce was reused.
    Replay,
    /// The fail-closed nonce set cannot retain another value.
    Capacity,
    /// The frozen gateway codec rejected the payload.
    Codec,
    /// The request carried a document payload forbidden at this edge.
    ForbiddenPayload,
    /// The request did not match the process-owned project binding.
    CrossProject,
    /// The injected Rust service failed without a safe typed reply.
    Service,
    /// The service returned a reply outside the exact request binding.
    Reply,
}

/// Payload-free transport error safe for logs and callers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GatewayTransportError {
    kind: GatewayTransportErrorKind,
}

impl GatewayTransportError {
    const fn new(kind: GatewayTransportErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the stable failure class.
    #[must_use]
    pub const fn kind(self) -> GatewayTransportErrorKind {
        self.kind
    }

    /// Returns a static machine-facing code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self.kind {
            GatewayTransportErrorKind::Configuration => "OPENCLAW_GATEWAY_CONFIGURATION_REJECTED",
            GatewayTransportErrorKind::Unavailable => "OPENCLAW_GATEWAY_UNAVAILABLE",
            GatewayTransportErrorKind::NonLocal => "OPENCLAW_GATEWAY_NONLOCAL_REJECTED",
            GatewayTransportErrorKind::Timeout => "OPENCLAW_GATEWAY_TIMEOUT",
            GatewayTransportErrorKind::Ambiguous => "OPENCLAW_GATEWAY_AMBIGUOUS",
            GatewayTransportErrorKind::Malformed => "OPENCLAW_GATEWAY_MALFORMED",
            GatewayTransportErrorKind::Authentication => "OPENCLAW_GATEWAY_AUTH_REJECTED",
            GatewayTransportErrorKind::Replay => "OPENCLAW_GATEWAY_REPLAY_REJECTED",
            GatewayTransportErrorKind::Capacity => "OPENCLAW_GATEWAY_REPLAY_CAPACITY",
            GatewayTransportErrorKind::Codec => "OPENCLAW_GATEWAY_CODEC_REJECTED",
            GatewayTransportErrorKind::ForbiddenPayload => "OPENCLAW_GATEWAY_PAYLOAD_REJECTED",
            GatewayTransportErrorKind::CrossProject => "OPENCLAW_GATEWAY_PROJECT_REJECTED",
            GatewayTransportErrorKind::Service => "OPENCLAW_GATEWAY_SERVICE_REJECTED",
            GatewayTransportErrorKind::Reply => "OPENCLAW_GATEWAY_REPLY_REJECTED",
        }
    }
}

impl fmt::Display for GatewayTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for GatewayTransportError {}

/// Process-owned binding for one loopback-only `OpenClaw` gateway listener.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenClawGatewayConfig {
    bind_address: SocketAddr,
    timeout: Duration,
    project_id: ProjectId,
    peer: GatewayPeerContext,
    authentication_key: AuthenticationKey,
    frozen_submissions: BTreeMap<String, TaskSpecSubmission>,
    frozen_submission_bytes: usize,
}

impl OpenClawGatewayConfig {
    /// Validates the transport-only peer, project, local endpoint, timeout, and key.
    ///
    /// # Errors
    ///
    /// Rejects a non-loopback endpoint, non-OpenClaw peer, or timeout outside
    /// the bounded transport range. This checkpoint never marks the peer live.
    pub fn new(
        bind_address: SocketAddr,
        timeout: Duration,
        project_id: ProjectId,
        peer: GatewayPeerContext,
        authentication_key: AuthenticationKey,
    ) -> Result<Self, GatewayTransportError> {
        if !bind_address.ip().is_loopback() {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::NonLocal,
            ));
        }
        if timeout.is_zero() || timeout > MAX_TIMEOUT {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Configuration,
            ));
        }
        if peer.runtime() != RuntimeKind::Fake || peer.client_kind() != GatewayClientKind::OpenClaw
        {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Configuration,
            ));
        }
        Ok(Self {
            bind_address,
            timeout,
            project_id,
            peer,
            authentication_key,
            frozen_submissions: BTreeMap::new(),
            frozen_submission_bytes: 0,
        })
    }

    /// Adds one immutable Task Spec that may be selected only by its exact digest.
    ///
    /// The document remains server-side and is mechanically verified before the
    /// listener can bind. The bounded catalog never performs a runtime DB/path
    /// lookup on behalf of `OpenClaw`.
    ///
    /// # Errors
    ///
    /// Rejects a foreign project, invalid document binding, duplicate digest,
    /// or catalog entry/byte overflow.
    pub fn with_frozen_submission(
        mut self,
        submission: TaskSpecSubmission,
    ) -> Result<Self, GatewayTransportError> {
        if submission.binding().project_id() != &self.project_id {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::CrossProject,
            ));
        }
        verify_task_spec_document(
            submission.canonical_document(),
            submission.claimed_spec_digest(),
            submission.binding(),
        )
        .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Configuration))?;
        let next_bytes = self
            .frozen_submission_bytes
            .checked_add(submission.canonical_document().len())
            .ok_or_else(|| GatewayTransportError::new(GatewayTransportErrorKind::Capacity))?;
        let key = submission.claimed_spec_digest().as_str().to_owned();
        if self.frozen_submissions.len() >= MAX_FROZEN_SUBMISSIONS
            || next_bytes > MAX_FROZEN_SUBMISSION_BYTES
        {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Capacity,
            ));
        }
        if self.frozen_submissions.contains_key(&key) {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Configuration,
            ));
        }
        self.frozen_submissions.insert(key, submission);
        self.frozen_submission_bytes = next_bytes;
        Ok(self)
    }
}

/// Stateful transport-only listener around one injected Rust [`GatewayService`].
pub struct OpenClawGatewayServer<S> {
    listener: TcpListener,
    timeout: Duration,
    project_id: ProjectId,
    peer: GatewayPeerContext,
    session_epoch: TransportSessionEpoch,
    authentication_key: AuthenticationKey,
    official_launch_record: Option<OpenClawOfficialLaunchRecord>,
    frozen_submissions: BTreeMap<String, TaskSpecSubmission>,
    seen_nonces: BTreeSet<[u8; NONCE_BYTES]>,
    idempotency_store: Box<dyn OpenClawIdempotencyStore>,
    service: Option<S>,
    pending: Option<PendingDispatch<S>>,
    pending_terminal_record: Option<OpenClawTerminalCommandRecord>,
    connection_sender: SyncSender<ConnectionResult>,
    connection_receiver: Receiver<ConnectionResult>,
    inflight_connections: usize,
}

impl<S> OpenClawGatewayServer<S>
where
    S: GatewayService + Send + 'static,
{
    /// Binds the configured loopback endpoint with process-memory idempotency.
    ///
    /// This transport-only convenience path is never restart-safe and never
    /// establishes official-package or live-runtime evidence.
    ///
    /// # Errors
    ///
    /// Returns a static unavailable or non-local failure when binding is unsafe.
    pub fn bind(config: OpenClawGatewayConfig, service: S) -> Result<Self, GatewayTransportError> {
        Self::bind_with_store(
            config,
            service,
            Box::<ProcessMemoryOpenClawIdempotencyStore>::default(),
            None,
        )
    }

    /// Binds a transport with a LATTICE-owned official-package launch gate.
    ///
    /// Every accepted connection must send an authenticated closed `ClientHello`
    /// matching `launch_record` before its command frame. This records official
    /// process evidence but does not by itself claim durable or live runtime
    /// acceptance.
    ///
    /// # Errors
    ///
    /// Returns a static unavailable or non-local failure when binding is unsafe.
    pub fn bind_official_launch(
        config: OpenClawGatewayConfig,
        service: S,
        launch_record: OpenClawOfficialLaunchRecord,
    ) -> Result<Self, GatewayTransportError> {
        Self::bind_with_store(
            config,
            service,
            Box::<ProcessMemoryOpenClawIdempotencyStore>::default(),
            Some(launch_record),
        )
    }

    /// Binds with a durable terminal-command idempotency provider.
    ///
    /// This is the required production integration seam for restart-safe command
    /// reconciliation. The provider must ultimately own `PostgreSQL` terminal
    /// command receipts; this transport checkpoint does not wire that backend
    /// or establish official-package/live-runtime evidence.
    ///
    /// # Errors
    ///
    /// Rejects a provider that identifies itself as process-memory-only.
    pub fn bind_with_durable_idempotency<I>(
        config: OpenClawGatewayConfig,
        service: S,
        store: I,
    ) -> Result<Self, GatewayTransportError>
    where
        I: OpenClawIdempotencyStore + 'static,
    {
        if store.durability() != OpenClawIdempotencyDurability::DurableTerminalReceipts {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Configuration,
            ));
        }
        Self::bind_with_store(config, service, Box::new(store), None)
    }

    /// Binds an official-package launch gate with durable command reconciliation.
    ///
    /// # Errors
    ///
    /// Rejects a provider that identifies itself as process-memory-only.
    pub fn bind_official_launch_with_durable_idempotency<I>(
        config: OpenClawGatewayConfig,
        service: S,
        launch_record: OpenClawOfficialLaunchRecord,
        store: I,
    ) -> Result<Self, GatewayTransportError>
    where
        I: OpenClawIdempotencyStore + 'static,
    {
        if store.durability() != OpenClawIdempotencyDurability::DurableTerminalReceipts {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Configuration,
            ));
        }
        Self::bind_with_store(config, service, Box::new(store), Some(launch_record))
    }

    fn bind_with_store(
        config: OpenClawGatewayConfig,
        service: S,
        idempotency_store: Box<dyn OpenClawIdempotencyStore>,
        official_launch_record: Option<OpenClawOfficialLaunchRecord>,
    ) -> Result<Self, GatewayTransportError> {
        let listener = TcpListener::bind(config.bind_address)
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Unavailable))?;
        let local_address = listener
            .local_addr()
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Unavailable))?;
        if !local_address.ip().is_loopback() {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::NonLocal,
            ));
        }
        listener
            .set_nonblocking(true)
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Unavailable))?;
        let session_epoch = new_transport_session_epoch()?;
        let authentication_key =
            derive_transport_session_key(&config.authentication_key, session_epoch)?;
        let (connection_sender, connection_receiver) = sync_channel(MAX_INFLIGHT_CONNECTIONS);
        Ok(Self {
            listener,
            timeout: config.timeout,
            project_id: config.project_id,
            peer: config.peer,
            session_epoch,
            authentication_key,
            official_launch_record,
            frozen_submissions: config.frozen_submissions,
            seen_nonces: BTreeSet::new(),
            idempotency_store,
            service: Some(service),
            pending: None,
            pending_terminal_record: None,
            connection_sender,
            connection_receiver,
            inflight_connections: 0,
        })
    }

    /// Returns the actual loopback address, including an OS-selected port.
    ///
    /// # Errors
    ///
    /// Returns an unavailable failure if the OS no longer exposes the address.
    pub fn local_addr(&self) -> Result<SocketAddr, GatewayTransportError> {
        self.listener
            .local_addr()
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Unavailable))
    }

    /// Returns the injected service while no timed-out dispatch owns it.
    #[must_use]
    pub const fn service(&self) -> Option<&S> {
        self.service.as_ref()
    }

    /// Accepts, authenticates, validates, dispatches, and replies to one frame.
    ///
    /// # Errors
    ///
    /// Fails closed for non-local peers, authentication/replay failures,
    /// malformed or cross-project requests, timeouts, and invalid replies.
    pub fn serve_once(&mut self) -> Result<(), GatewayTransportError> {
        let PreparedConnection {
            mut stream,
            hello_nonce,
            nonce,
            frame,
            deadline: connection_deadline,
        } = self.accept_authenticated_connection()?;
        if hello_nonce == Some(nonce)
            || hello_nonce.is_some_and(|value| self.seen_nonces.contains(&value))
            || self.seen_nonces.contains(&nonce)
        {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Replay,
            ));
        }
        let new_nonce_count = usize::from(hello_nonce.is_some()) + 1;
        if self
            .seen_nonces
            .len()
            .checked_add(new_nonce_count)
            .is_none_or(|count| count > MAX_AUTH_REPLAY_ENTRIES)
        {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Capacity,
            ));
        }
        if let Some(value) = hello_nonce {
            self.seen_nonces.insert(value);
        }
        self.seen_nonces.insert(nonce);

        let (request, reply_encoding) = self.decode_ingress(&frame)?;
        if request.project_id() != &self.project_id {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::CrossProject,
            ));
        }
        let replay_key = OpenClawCommandScope::new(
            request.project_id().clone(),
            self.peer.actor_id().clone(),
            self.peer.session_epoch(),
            request.command_id().clone(),
        )
        .map_err(map_idempotency_error)?;
        self.reconcile_pending()?;
        match self
            .idempotency_store
            .reconcile_and_claim(&replay_key, &request)
            .map_err(map_idempotency_error)?
        {
            OpenClawIdempotencyDecision::Exact(reply) => {
                let reply_frame = encode_transport_reply(&request, &reply, reply_encoding)?;
                return write_authenticated_packet(
                    &mut stream,
                    RESPONSE_MAGIC,
                    self.session_epoch,
                    nonce,
                    &reply_frame,
                    &self.authentication_key,
                    connection_deadline,
                );
            }
            OpenClawIdempotencyDecision::CommandSubstitution => {
                let denial = build_reply(
                    &request,
                    GatewayReplyBody::Denied(GatewayDenialCode::CommandSubstitution),
                )
                .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Reply))?;
                let denial_frame = encode_transport_reply(&request, &denial, reply_encoding)?;
                return write_authenticated_packet(
                    &mut stream,
                    RESPONSE_MAGIC,
                    self.session_epoch,
                    nonce,
                    &denial_frame,
                    &self.authentication_key,
                    connection_deadline,
                );
            }
            OpenClawIdempotencyDecision::InFlight => {
                return Err(GatewayTransportError::new(
                    GatewayTransportErrorKind::Ambiguous,
                ));
            }
            OpenClawIdempotencyDecision::Claimed => {}
        }
        let reply_frame =
            self.dispatch_with_deadline(request, replay_key, reply_encoding, connection_deadline)?;
        write_authenticated_packet(
            &mut stream,
            RESPONSE_MAGIC,
            self.session_epoch,
            nonce,
            &reply_frame,
            &self.authentication_key,
            connection_deadline,
        )
    }

    fn decode_ingress(
        &self,
        frame: &[u8],
    ) -> Result<(GatewayRequest, ReplyEncoding), GatewayTransportError> {
        if let Ok(request) = wire::decode_openclaw_control_request(frame) {
            return Ok((request, ReplyEncoding::Gateway));
        }
        if let Ok(selector) = wire::decode_openclaw_submit_request(frame) {
            let submission = self
                .frozen_submissions
                .get(selector.task_spec_digest.as_str())
                .cloned()
                .ok_or_else(|| {
                    GatewayTransportError::new(GatewayTransportErrorKind::ForbiddenPayload)
                })?;
            let request = build_request(
                selector.command_id,
                selector.correlation_id,
                GatewayRequestBody::Submit(submission),
            )
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Codec))?;
            return Ok((request, ReplyEncoding::OpenClawSubmit));
        }
        if decode_request(frame)
            .is_ok_and(|request| matches!(request.body(), GatewayRequestBody::Submit(_)))
        {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::ForbiddenPayload,
            ));
        }
        Err(GatewayTransportError::new(GatewayTransportErrorKind::Codec))
    }

    fn accept_authenticated_connection(
        &mut self,
    ) -> Result<PreparedConnection, GatewayTransportError> {
        loop {
            while self.inflight_connections < MAX_INFLIGHT_CONNECTIONS {
                let (stream, remote_address) = match self.listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        return Err(GatewayTransportError::new(
                            GatewayTransportErrorKind::Unavailable,
                        ));
                    }
                };
                let sender = self.connection_sender.clone();
                let key = self.authentication_key.clone();
                let session_epoch = self.session_epoch;
                let timeout = self.timeout;
                let official_launch_record = self.official_launch_record.clone();
                thread::Builder::new()
                    .name("lattice-openclaw-gateway-auth".to_owned())
                    .spawn(move || {
                        let result = prepare_authenticated_connection(
                            stream,
                            remote_address,
                            timeout,
                            session_epoch,
                            &key,
                            official_launch_record.as_ref(),
                        );
                        let _ignored = sender.send(result);
                    })
                    .map_err(|_| {
                        GatewayTransportError::new(GatewayTransportErrorKind::Unavailable)
                    })?;
                self.inflight_connections += 1;
            }

            match self.connection_receiver.try_recv() {
                Ok(result) => {
                    self.inflight_connections = self.inflight_connections.saturating_sub(1);
                    return result;
                }
                Err(TryRecvError::Empty) => thread::sleep(Duration::from_millis(1)),
                Err(TryRecvError::Disconnected) => {
                    return Err(GatewayTransportError::new(
                        GatewayTransportErrorKind::Unavailable,
                    ));
                }
            }
        }
    }

    fn dispatch_with_deadline(
        &mut self,
        request: GatewayRequest,
        replay_key: OpenClawCommandScope,
        reply_encoding: ReplyEncoding,
        connection_deadline: Instant,
    ) -> Result<Vec<u8>, GatewayTransportError> {
        let remaining = remaining_connection_time(connection_deadline)?;
        let mut service = self
            .service
            .take()
            .ok_or_else(|| GatewayTransportError::new(GatewayTransportErrorKind::Ambiguous))?;
        let (sender, receiver) = sync_channel(1);
        let peer = self.peer.clone();
        let dispatch_request = request.clone();
        thread::Builder::new()
            .name("lattice-openclaw-gateway-service".to_owned())
            .spawn(move || {
                let result = service.handle(peer, dispatch_request);
                let _ignored = sender.send((service, result));
            })
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Unavailable))?;

        match receiver.recv_timeout(remaining) {
            Ok((service, result)) => {
                self.service = Some(service);
                self.prepare_terminal_reply(&request, replay_key, reply_encoding, result)
            }
            Err(RecvTimeoutError::Timeout) => {
                self.pending = Some(PendingDispatch {
                    receiver,
                    request,
                    replay_key,
                    reply_encoding,
                });
                Err(GatewayTransportError::new(
                    GatewayTransportErrorKind::Ambiguous,
                ))
            }
            Err(RecvTimeoutError::Disconnected) => Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Ambiguous,
            )),
        }
    }

    fn reconcile_pending(&mut self) -> Result<(), GatewayTransportError> {
        self.reconcile_pending_terminal_record()?;
        let Some(pending) = self.pending.take() else {
            return Ok(());
        };
        match pending.receiver.try_recv() {
            Ok((service, result)) => {
                self.service = Some(service);
                self.prepare_terminal_reply(
                    &pending.request,
                    pending.replay_key,
                    pending.reply_encoding,
                    result,
                )?;
                Ok(())
            }
            Err(TryRecvError::Empty) => {
                self.pending = Some(pending);
                Err(GatewayTransportError::new(
                    GatewayTransportErrorKind::Ambiguous,
                ))
            }
            Err(TryRecvError::Disconnected) => Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Ambiguous,
            )),
        }
    }

    fn reconcile_pending_terminal_record(&mut self) -> Result<(), GatewayTransportError> {
        let Some(record) = self.pending_terminal_record.take() else {
            return Ok(());
        };
        if self
            .idempotency_store
            .finalize_terminal(record.clone())
            .is_err()
        {
            self.pending_terminal_record = Some(record);
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Ambiguous,
            ));
        }
        Ok(())
    }

    fn prepare_terminal_reply(
        &mut self,
        request: &GatewayRequest,
        replay_key: OpenClawCommandScope,
        reply_encoding: ReplyEncoding,
        result: lattice_ports::GatewayServiceResult<GatewayReply>,
    ) -> Result<Vec<u8>, GatewayTransportError> {
        let reply = map_service_result(request, result)?;
        let reply_frame = encode_transport_reply(request, &reply, reply_encoding)?;
        let record = OpenClawTerminalCommandRecord::new(replay_key, request.clone(), reply)
            .map_err(map_idempotency_error)?;
        if self
            .idempotency_store
            .finalize_terminal(record.clone())
            .is_err()
        {
            self.pending_terminal_record = Some(record);
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Ambiguous,
            ));
        }
        Ok(reply_frame)
    }
}

fn encode_transport_reply(
    request: &GatewayRequest,
    reply: &GatewayReply,
    encoding: ReplyEncoding,
) -> Result<Vec<u8>, GatewayTransportError> {
    let gateway_frame = encode_reply(reply)
        .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Reply))?;
    decode_reply(request, &gateway_frame)
        .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Reply))?;
    match encoding {
        ReplyEncoding::Gateway => Ok(gateway_frame),
        ReplyEncoding::OpenClawSubmit => wire::encode_openclaw_submit_reply(reply),
    }
}

/// Authenticated loopback client used by the transport checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenClawGatewayClient {
    endpoint: SocketAddr,
    timeout: Duration,
    authentication_key: AuthenticationKey,
}

impl OpenClawGatewayClient {
    /// Constructs a client for one loopback-only endpoint.
    ///
    /// # Errors
    ///
    /// Rejects a non-loopback endpoint or an invalid timeout.
    pub fn new(
        endpoint: SocketAddr,
        timeout: Duration,
        authentication_key: AuthenticationKey,
    ) -> Result<Self, GatewayTransportError> {
        if !endpoint.ip().is_loopback() {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::NonLocal,
            ));
        }
        if endpoint.port() == 0 || timeout.is_zero() || timeout > MAX_TIMEOUT {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Configuration,
            ));
        }
        Ok(Self {
            endpoint,
            timeout,
            authentication_key,
        })
    }

    /// Sends one frozen typed request under an explicit one-use nonce.
    ///
    /// # Errors
    ///
    /// Returns a bounded transport, authentication, codec, or reply failure.
    pub fn send(
        &self,
        request: &GatewayRequest,
        nonce: TransportNonce,
    ) -> Result<GatewayReply, GatewayTransportError> {
        let request_frame = match request.body() {
            GatewayRequestBody::Status(target) => {
                wire::encode_openclaw_status_request(&OpenClawStatusRequest::from_target(
                    request.command_id().clone(),
                    request.correlation_id().clone(),
                    target.clone(),
                ))?
            }
            GatewayRequestBody::Stop(target) => {
                wire::encode_openclaw_stop_request(&OpenClawStopRequest::new(
                    request.command_id().clone(),
                    request.correlation_id().clone(),
                    target.clone(),
                ))?
            }
            GatewayRequestBody::Submit(_) => {
                return Err(GatewayTransportError::new(
                    GatewayTransportErrorKind::ForbiddenPayload,
                ));
            }
            GatewayRequestBody::Plan(_)
            | GatewayRequestBody::Approve(_)
            | GatewayRequestBody::Reject(_) => {
                return Err(GatewayTransportError::new(GatewayTransportErrorKind::Codec));
            }
        };
        self.send_control_frame(request, &request_frame, nonce)
    }

    /// Sends one closed typed status request.
    ///
    /// # Errors
    ///
    /// Returns a bounded transport, authentication, codec, or reply failure.
    pub fn send_status(
        &self,
        request: &OpenClawStatusRequest,
        nonce: TransportNonce,
    ) -> Result<GatewayReply, GatewayTransportError> {
        let gateway_request = request.gateway_request()?;
        let frame = encode_openclaw_status_request(request)?;
        self.send_control_frame(&gateway_request, &frame, nonce)
    }

    /// Sends one closed typed stop request.
    ///
    /// # Errors
    ///
    /// Returns a bounded transport, authentication, codec, or reply failure.
    pub fn send_stop(
        &self,
        request: &OpenClawStopRequest,
        nonce: TransportNonce,
    ) -> Result<GatewayReply, GatewayTransportError> {
        let gateway_request = request.gateway_request()?;
        let frame = encode_openclaw_stop_request(request)?;
        self.send_control_frame(&gateway_request, &frame, nonce)
    }

    fn send_control_frame(
        &self,
        request: &GatewayRequest,
        request_frame: &[u8],
        nonce: TransportNonce,
    ) -> Result<GatewayReply, GatewayTransportError> {
        let mut stream = TcpStream::connect_timeout(&self.endpoint, self.timeout)
            .map_err(|error| map_connect_error(&error))?;
        configure_stream(&stream, self.timeout)?;
        let connection_deadline = connection_deadline(self.timeout)?;
        let (session_epoch, session_key) = read_transport_session_greeting(
            &mut stream,
            &self.authentication_key,
            connection_deadline,
        )?;
        let nonce_bytes = nonce.bytes();
        write_authenticated_packet(
            &mut stream,
            REQUEST_MAGIC,
            session_epoch,
            nonce_bytes,
            request_frame,
            &session_key,
            connection_deadline,
        )?;
        let (reply_nonce, reply_frame) = read_authenticated_packet(
            &mut stream,
            RESPONSE_MAGIC,
            session_epoch,
            &session_key,
            connection_deadline,
        )?;
        if reply_nonce != nonce_bytes {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Authentication,
            ));
        }
        decode_reply(request, &reply_frame)
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Reply))
    }

    /// Sends one binding-only Submit selector under an explicit one-use nonce.
    ///
    /// Only the frozen Task Spec digest is serialized. The server-owned
    /// canonical document is reconstructed after authentication and catalog
    /// resolution, immediately before the injected [`GatewayService`].
    ///
    /// # Errors
    ///
    /// Returns a bounded transport, authentication, codec, or typed-reply
    /// failure.
    pub fn send_submit(
        &self,
        request: &OpenClawSubmitRequest,
        nonce: TransportNonce,
    ) -> Result<OpenClawSubmitReply, GatewayTransportError> {
        let request_frame = encode_openclaw_submit_request(request)?;
        let mut stream = TcpStream::connect_timeout(&self.endpoint, self.timeout)
            .map_err(|error| map_connect_error(&error))?;
        configure_stream(&stream, self.timeout)?;
        let connection_deadline = connection_deadline(self.timeout)?;
        let (session_epoch, session_key) = read_transport_session_greeting(
            &mut stream,
            &self.authentication_key,
            connection_deadline,
        )?;
        let nonce_bytes = nonce.bytes();
        write_authenticated_packet(
            &mut stream,
            REQUEST_MAGIC,
            session_epoch,
            nonce_bytes,
            &request_frame,
            &session_key,
            connection_deadline,
        )?;
        let (reply_nonce, reply_frame) = read_authenticated_packet(
            &mut stream,
            RESPONSE_MAGIC,
            session_epoch,
            &session_key,
            connection_deadline,
        )?;
        if reply_nonce != nonce_bytes {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Authentication,
            ));
        }
        wire::decode_openclaw_submit_reply(request, &reply_frame)
    }
}

fn prepare_authenticated_connection(
    mut stream: TcpStream,
    remote_address: SocketAddr,
    timeout: Duration,
    session_epoch: TransportSessionEpoch,
    key: &AuthenticationKey,
    official_launch_record: Option<&OpenClawOfficialLaunchRecord>,
) -> ConnectionResult {
    if !remote_address.ip().is_loopback() {
        return Err(GatewayTransportError::new(
            GatewayTransportErrorKind::NonLocal,
        ));
    }
    stream
        .set_nonblocking(false)
        .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Unavailable))?;
    configure_stream(&stream, timeout)?;
    let deadline = connection_deadline(timeout)?;
    write_transport_session_greeting(&mut stream, session_epoch, deadline)?;
    let hello_nonce = if let Some(record) = official_launch_record {
        let (nonce, hello_frame) =
            read_authenticated_packet(&mut stream, REQUEST_MAGIC, session_epoch, key, deadline)?;
        let hello = wire::decode_openclaw_client_hello(&hello_frame)
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Authentication))?;
        if hello != record.client_hello() {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Authentication,
            ));
        }
        Some(nonce)
    } else {
        None
    };
    let (nonce, frame) =
        read_authenticated_packet(&mut stream, REQUEST_MAGIC, session_epoch, key, deadline)?;
    Ok(PreparedConnection {
        stream,
        hello_nonce,
        nonce,
        frame,
        deadline,
    })
}

fn configure_stream(stream: &TcpStream, timeout: Duration) -> Result<(), GatewayTransportError> {
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| stream.set_write_timeout(Some(timeout)))
        .and_then(|()| stream.set_nodelay(true))
        .map_err(|error| map_io_error(&error))
}

fn new_transport_session_epoch() -> Result<TransportSessionEpoch, GatewayTransportError> {
    let mut epoch = [0_u8; SESSION_EPOCH_BYTES];
    getrandom::fill(&mut epoch)
        .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Configuration))?;
    if epoch.iter().all(|byte| *byte == 0) {
        return Err(GatewayTransportError::new(
            GatewayTransportErrorKind::Configuration,
        ));
    }
    Ok(TransportSessionEpoch(epoch))
}

fn derive_transport_session_key(
    root_key: &AuthenticationKey,
    epoch: TransportSessionEpoch,
) -> Result<AuthenticationKey, GatewayTransportError> {
    let mut hasher = Sha256::new();
    hasher.update(b"lattice-openclaw-session-key-v1\0");
    hasher.update(root_key.bytes());
    hasher.update(epoch.0);
    let digest = hasher.finalize();
    let mut key = [0_u8; TAG_BYTES];
    key.copy_from_slice(&digest);
    AuthenticationKey::new(key)
}

fn write_transport_session_greeting(
    stream: &mut TcpStream,
    epoch: TransportSessionEpoch,
    deadline: Instant,
) -> Result<(), GatewayTransportError> {
    let greeting = transport_session_greeting(epoch);
    write_all_until(stream, &greeting, deadline)
}

fn transport_session_greeting(epoch: TransportSessionEpoch) -> [u8; SESSION_GREETING_BYTES] {
    let mut greeting = [0_u8; SESSION_GREETING_BYTES];
    greeting[..SESSION_MAGIC.len()].copy_from_slice(&SESSION_MAGIC);
    greeting[SESSION_MAGIC.len()..].copy_from_slice(&epoch.0);
    greeting
}

fn read_transport_session_greeting(
    stream: &mut TcpStream,
    root_key: &AuthenticationKey,
    deadline: Instant,
) -> Result<(TransportSessionEpoch, AuthenticationKey), GatewayTransportError> {
    let mut greeting = [0_u8; SESSION_GREETING_BYTES];
    read_exact_until(stream, &mut greeting, deadline)?;
    if greeting[..SESSION_MAGIC.len()] != SESSION_MAGIC {
        return Err(GatewayTransportError::new(
            GatewayTransportErrorKind::Authentication,
        ));
    }
    let mut epoch = [0_u8; SESSION_EPOCH_BYTES];
    epoch.copy_from_slice(&greeting[SESSION_MAGIC.len()..]);
    if epoch.iter().all(|byte| *byte == 0) {
        return Err(GatewayTransportError::new(
            GatewayTransportErrorKind::Authentication,
        ));
    }
    let epoch = TransportSessionEpoch(epoch);
    let session_key = derive_transport_session_key(root_key, epoch)?;
    Ok((epoch, session_key))
}

fn connection_deadline(timeout: Duration) -> Result<Instant, GatewayTransportError> {
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| GatewayTransportError::new(GatewayTransportErrorKind::Configuration))
}

fn remaining_connection_time(deadline: Instant) -> Result<Duration, GatewayTransportError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| GatewayTransportError::new(GatewayTransportErrorKind::Timeout))
}

fn read_exact_until(
    stream: &mut TcpStream,
    mut buffer: &mut [u8],
    deadline: Instant,
) -> Result<(), GatewayTransportError> {
    while !buffer.is_empty() {
        stream
            .set_read_timeout(Some(remaining_connection_time(deadline)?))
            .map_err(|error| map_io_error(&error))?;
        match stream.read(buffer) {
            Ok(0) => {
                return Err(GatewayTransportError::new(
                    GatewayTransportErrorKind::Malformed,
                ));
            }
            Ok(read) => buffer = &mut buffer[read..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(map_io_error(&error)),
        }
    }
    Ok(())
}

fn write_all_until(
    stream: &mut TcpStream,
    mut buffer: &[u8],
    deadline: Instant,
) -> Result<(), GatewayTransportError> {
    while !buffer.is_empty() {
        stream
            .set_write_timeout(Some(remaining_connection_time(deadline)?))
            .map_err(|error| map_io_error(&error))?;
        match stream.write(buffer) {
            Ok(0) => {
                return Err(GatewayTransportError::new(
                    GatewayTransportErrorKind::Unavailable,
                ));
            }
            Ok(written) => buffer = &buffer[written..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(map_io_error(&error)),
        }
    }
    Ok(())
}

fn map_connect_error(error: &io::Error) -> GatewayTransportError {
    if matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        GatewayTransportError::new(GatewayTransportErrorKind::Timeout)
    } else {
        GatewayTransportError::new(GatewayTransportErrorKind::Unavailable)
    }
}

fn map_io_error(error: &io::Error) -> GatewayTransportError {
    let kind = match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => GatewayTransportErrorKind::Timeout,
        io::ErrorKind::UnexpectedEof => GatewayTransportErrorKind::Malformed,
        _ => GatewayTransportErrorKind::Unavailable,
    };
    GatewayTransportError::new(kind)
}

fn map_idempotency_error(error: OpenClawIdempotencyError) -> GatewayTransportError {
    let kind = match error {
        OpenClawIdempotencyError::Capacity => GatewayTransportErrorKind::Capacity,
        OpenClawIdempotencyError::Unavailable | OpenClawIdempotencyError::Malformed => {
            GatewayTransportErrorKind::Service
        }
    };
    GatewayTransportError::new(kind)
}

fn map_service_result(
    request: &GatewayRequest,
    result: lattice_ports::GatewayServiceResult<GatewayReply>,
) -> Result<GatewayReply, GatewayTransportError> {
    match result {
        Ok(reply) => Ok(reply),
        Err(error) => match error.kind() {
            PortErrorKind::Denied => build_reply(
                request,
                GatewayReplyBody::Denied(GatewayDenialCode::DownstreamDenied),
            )
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Reply)),
            PortErrorKind::Ambiguous => build_reply(
                request,
                GatewayReplyBody::UnknownOutcome(GatewayUnknownCode::DownstreamAmbiguous),
            )
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Reply)),
            PortErrorKind::Timeout => Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Timeout,
            )),
            PortErrorKind::Unavailable
            | PortErrorKind::Cancelled
            | PortErrorKind::VersionMismatch
            | PortErrorKind::CapabilityMismatch
            | PortErrorKind::Malformed => Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Service,
            )),
        },
    }
}

fn read_authenticated_packet(
    stream: &mut TcpStream,
    expected_magic: [u8; REQUEST_MAGIC.len()],
    expected_epoch: TransportSessionEpoch,
    key: &AuthenticationKey,
    deadline: Instant,
) -> Result<([u8; NONCE_BYTES], Vec<u8>), GatewayTransportError> {
    let mut header = [0_u8; HEADER_BYTES];
    read_exact_until(stream, &mut header, deadline)?;
    if header[..REQUEST_MAGIC.len()] != expected_magic {
        return Err(GatewayTransportError::new(
            GatewayTransportErrorKind::Malformed,
        ));
    }
    let epoch_start = REQUEST_MAGIC.len();
    let epoch_end = epoch_start + SESSION_EPOCH_BYTES;
    if header[epoch_start..epoch_end] != expected_epoch.0 {
        return Err(GatewayTransportError::new(
            GatewayTransportErrorKind::Authentication,
        ));
    }
    let mut nonce = [0_u8; NONCE_BYTES];
    let nonce_start = epoch_end;
    let nonce_end = nonce_start + NONCE_BYTES;
    nonce.copy_from_slice(&header[nonce_start..nonce_end]);
    if nonce.iter().all(|byte| *byte == 0) {
        return Err(GatewayTransportError::new(
            GatewayTransportErrorKind::Malformed,
        ));
    }
    let length_start = nonce_end;
    let length_end = length_start + LENGTH_BYTES;
    let length_bytes: [u8; LENGTH_BYTES] = header[length_start..length_end]
        .try_into()
        .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Malformed))?;
    let length = usize::try_from(u32::from_be_bytes(length_bytes))
        .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Malformed))?;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(GatewayTransportError::new(
            GatewayTransportErrorKind::Malformed,
        ));
    }
    let mut claimed_tag = [0_u8; TAG_BYTES];
    claimed_tag.copy_from_slice(&header[length_end..HEADER_BYTES]);
    let mut payload = vec![0_u8; length];
    read_exact_until(stream, &mut payload, deadline)?;
    let actual_tag = authenticate(
        key,
        expected_magic,
        expected_epoch,
        &nonce,
        length_bytes,
        &payload,
    );
    if !constant_time_eq(&claimed_tag, &actual_tag) {
        return Err(GatewayTransportError::new(
            GatewayTransportErrorKind::Authentication,
        ));
    }
    Ok((nonce, payload))
}

fn write_authenticated_packet(
    stream: &mut TcpStream,
    magic: [u8; REQUEST_MAGIC.len()],
    epoch: TransportSessionEpoch,
    nonce: [u8; NONCE_BYTES],
    payload: &[u8],
    key: &AuthenticationKey,
    deadline: Instant,
) -> Result<(), GatewayTransportError> {
    let packet = authenticated_packet(magic, epoch, nonce, payload, key)?;
    write_all_until(stream, &packet, deadline)
}

fn authenticated_packet(
    magic: [u8; REQUEST_MAGIC.len()],
    epoch: TransportSessionEpoch,
    nonce: [u8; NONCE_BYTES],
    payload: &[u8],
    key: &AuthenticationKey,
) -> Result<Vec<u8>, GatewayTransportError> {
    if payload.is_empty() || payload.len() > MAX_FRAME_BYTES {
        return Err(GatewayTransportError::new(
            GatewayTransportErrorKind::Malformed,
        ));
    }
    let payload_length = u32::try_from(payload.len())
        .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Malformed))?;
    let length_bytes = payload_length.to_be_bytes();
    let tag = authenticate(key, magic, epoch, &nonce, length_bytes, payload);
    let mut packet = Vec::with_capacity(HEADER_BYTES + payload.len());
    packet.extend_from_slice(&magic);
    packet.extend_from_slice(&epoch.0);
    packet.extend_from_slice(&nonce);
    packet.extend_from_slice(&length_bytes);
    packet.extend_from_slice(&tag);
    packet.extend_from_slice(payload);
    Ok(packet)
}

fn authenticate(
    key: &AuthenticationKey,
    magic: [u8; REQUEST_MAGIC.len()],
    epoch: TransportSessionEpoch,
    nonce: &[u8; NONCE_BYTES],
    length: [u8; LENGTH_BYTES],
    payload: &[u8],
) -> [u8; TAG_BYTES] {
    let mut inner_pad = [0x36_u8; HMAC_BLOCK_BYTES];
    let mut outer_pad = [0x5c_u8; HMAC_BLOCK_BYTES];
    for (index, byte) in key.bytes().iter().copied().enumerate() {
        inner_pad[index] ^= byte;
        outer_pad[index] ^= byte;
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(magic);
    inner.update(epoch.0);
    inner.update(nonce);
    inner.update(length);
    inner.update(payload);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    let digest = outer.finalize();
    let mut tag = [0_u8; TAG_BYTES];
    tag.copy_from_slice(&digest);
    tag
}

fn constant_time_eq(left: &[u8; TAG_BYTES], right: &[u8; TAG_BYTES]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::net::{IpAddr, Ipv4Addr, Shutdown};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;

    use lattice_contracts::{
        ContentDigest, GatewayActorId, GatewayActorKind, GatewayAdapterId, GatewayChannelId,
        GatewayCommandId, GatewayCorrelationId, GatewayInstanceId, GatewayProjectStatusTarget,
        GatewayRequestBody, GatewaySessionId, GatewayStatusTarget, ProjectSnapshotId,
        SubjectBinding, TaskId,
    };
    use lattice_gateway_ipc::encode_request;
    use lattice_ports::{GatewayServiceError, GatewayServiceResult};

    use super::*;

    fn digest(fill: char) -> ContentDigest {
        ContentDigest::from_sha256(fill.to_string().repeat(64)).expect("digest")
    }

    fn transport_peer() -> GatewayPeerContext {
        GatewayPeerContext::new_fake(
            GatewayClientKind::OpenClaw,
            GatewayInstanceId::new("gateway-transport-unit").expect("gateway"),
            GatewayAdapterId::new("openclaw-adapter").expect("adapter"),
            "1.0.0",
            digest('a'),
            digest('b'),
            GatewayActorId::new("responsible-user-unit").expect("actor"),
            GatewayActorKind::ResponsibleUser,
            GatewayChannelId::new("openclaw-local-unit").expect("channel"),
            GatewaySessionId::new("session-transport-unit").expect("session"),
            1,
            digest('c'),
            digest('c'),
        )
        .expect("transport peer")
    }

    fn status_request() -> GatewayRequest {
        status_request_with_command("command-unit")
    }

    fn status_request_with_command(command_id: &str) -> GatewayRequest {
        lattice_gateway_ipc::build_request(
            GatewayCommandId::new(command_id).expect("command"),
            GatewayCorrelationId::new("correlation-unit").expect("correlation"),
            GatewayRequestBody::Status(GatewayStatusTarget::Project(
                GatewayProjectStatusTarget::new(
                    ProjectId::new("project-a").expect("project"),
                    10,
                    None,
                )
                .expect("target"),
            )),
        )
        .expect("request")
    }

    fn full_submit_request() -> GatewayRequest {
        let document = br#"{"project_id":"project-a","project_snapshot_id":"snapshot-a","revision":"1","schema_version":"2.1","task_id":"task-a"}"#.to_vec();
        let document_digest =
            lattice_gateway_ipc::task_spec_document_digest(&document).expect("document digest");
        let binding = SubjectBinding::new(
            ProjectId::new("project-a").expect("project"),
            ProjectSnapshotId::new("snapshot-a").expect("snapshot"),
            TaskId::new("task-a").expect("task"),
            "1",
            document_digest.clone(),
        )
        .expect("binding");
        build_request(
            GatewayCommandId::new("command-full-submit").expect("command"),
            GatewayCorrelationId::new("correlation-full-submit").expect("correlation"),
            GatewayRequestBody::Submit(
                TaskSpecSubmission::new(binding, document, document_digest).expect("submission"),
            ),
        )
        .expect("request")
    }

    fn lowercase_hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;

        let mut output = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
        }
        output
    }

    #[test]
    fn exported_parity_goldens_match_production_greeting_request_and_response_bytes() {
        assert_eq!(OPENCLAW_WIRE_REQUEST_MAGIC, *b"LATGW001");
        assert_eq!(OPENCLAW_WIRE_RESPONSE_MAGIC, *b"LATGR001");
        assert_eq!(OPENCLAW_WIRE_SESSION_MAGIC, *b"LATSN001");

        let root_key = AuthenticationKey::new([0x11; TAG_BYTES]).expect("root key");
        let session_epoch = TransportSessionEpoch([
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ]);
        let nonce = [
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
            0x1e, 0x1f,
        ];
        assert_eq!(
            lowercase_hex(root_key.bytes()),
            OPENCLAW_PARITY_ROOT_KEY_HEX
        );
        assert_eq!(
            lowercase_hex(&session_epoch.0),
            OPENCLAW_PARITY_SESSION_EPOCH_HEX
        );
        assert_eq!(lowercase_hex(&nonce), OPENCLAW_PARITY_NONCE_HEX);
        assert_eq!(
            lowercase_hex(&transport_session_greeting(session_epoch)),
            OPENCLAW_PARITY_SESSION_GREETING_HEX
        );

        let request = OpenClawStatusRequest::command(
            GatewayCommandId::new("command-status-a").expect("command"),
            GatewayCorrelationId::new("correlation-status-a").expect("correlation"),
            ProjectId::new("project-a").expect("project"),
            GatewayCommandId::new("target-command-a").expect("target command"),
        );
        let request_payload = encode_openclaw_status_request(&request).expect("request payload");
        assert_eq!(
            std::str::from_utf8(&request_payload).expect("request utf8"),
            OPENCLAW_PARITY_STATUS_COMMAND_REQUEST_JSON
        );
        let session_key =
            derive_transport_session_key(&root_key, session_epoch).expect("session key");
        let request_packet = authenticated_packet(
            OPENCLAW_WIRE_REQUEST_MAGIC,
            session_epoch,
            nonce,
            &request_payload,
            &session_key,
        )
        .expect("request packet");
        assert_eq!(
            lowercase_hex(&request_packet),
            OPENCLAW_PARITY_REQUEST_PACKET_HEX
        );

        let gateway_request = request.gateway_request().expect("gateway request");
        let reply = build_reply(
            &gateway_request,
            GatewayReplyBody::Denied(GatewayDenialCode::DownstreamDenied),
        )
        .expect("reply");
        let response_payload = encode_reply(&reply).expect("response payload");
        assert_eq!(
            std::str::from_utf8(&response_payload).expect("response utf8"),
            OPENCLAW_PARITY_STATUS_COMMAND_RESPONSE_JSON
        );
        let response_packet = authenticated_packet(
            OPENCLAW_WIRE_RESPONSE_MAGIC,
            session_epoch,
            nonce,
            &response_payload,
            &session_key,
        )
        .expect("response packet");
        assert_eq!(
            lowercase_hex(&response_packet),
            OPENCLAW_PARITY_RESPONSE_PACKET_HEX
        );

        let fixture: serde_json::Value =
            serde_json::from_str(OPENCLAW_WIRE_PARITY_GOLDEN_JSON).expect("fixture JSON");
        assert_eq!(
            fixture["session_greeting"]["bytes_hex"],
            OPENCLAW_PARITY_SESSION_GREETING_HEX
        );
        assert_eq!(
            fixture["request_packet"]["bytes_hex"],
            OPENCLAW_PARITY_REQUEST_PACKET_HEX
        );
        assert_eq!(
            fixture["response_packet"]["bytes_hex"],
            OPENCLAW_PARITY_RESPONSE_PACKET_HEX
        );
    }

    #[test]
    fn official_client_hello_contains_only_lattice_owned_record_id_and_start_nonce() {
        let record = official_launch_record_with_id("launch-record-a");
        assert_eq!(record.package_name(), OPENCLAW_OFFICIAL_PACKAGE_NAME);
        assert_eq!(record.package_version(), OPENCLAW_OFFICIAL_PACKAGE_VERSION);
        assert_eq!(
            record.package_integrity(),
            OPENCLAW_OFFICIAL_PACKAGE_INTEGRITY
        );
        assert_eq!(record.entrypoint(), OPENCLAW_OFFICIAL_ENTRYPOINT);

        let hello = record.client_hello();
        let frame = encode_openclaw_client_hello(&hello).expect("hello frame");
        let text = std::str::from_utf8(&frame).expect("utf8");
        assert!(text.contains("\"launch_record_id\":\"launch-record-a\""));
        assert!(text.contains("\"process_start_nonce\":\"21212121212121212121212121212121\""));
        for forbidden in [
            "package_name",
            "package_version",
            "package_integrity",
            "entrypoint",
            "path",
            "credential",
            "runtime",
        ] {
            assert!(!text.contains(forbidden));
        }
        assert_eq!(
            wire::decode_openclaw_client_hello(&frame).expect("decode hello"),
            hello
        );
        let self_reported = br#"{"launch_record_id":"launch-record-a","package_version":"2026.7.1-2","process_start_nonce":"21212121212121212121212121212121","protocol":"lattice-openclaw-client-hello","version":"1"}"#;
        assert_eq!(
            wire::decode_openclaw_client_hello(self_reported)
                .expect_err("plugin package self-report must fail")
                .kind(),
            GatewayTransportErrorKind::Codec
        );
    }

    fn official_launch_record() -> OpenClawOfficialLaunchRecord {
        official_launch_record_with_id("launch-record-unit")
    }

    fn official_launch_record_with_id(launch_record_id: &str) -> OpenClawOfficialLaunchRecord {
        let process_start_nonce =
            OpenClawProcessStartNonce::new([0x21; 16]).expect("process nonce");
        let package_tarball_digest = digest('d');
        let entrypoint_digest = digest('e');
        let isolated_profile_digest = digest('f');
        let attestation_key =
            OpenClawLaunchAttestationKey::new([0x41; 32]).expect("attestation key");
        let evidence = OpenClawOfficialLaunchEvidence::new(
            launch_record_id,
            4242,
            process_start_nonce,
            package_tarball_digest,
            entrypoint_digest,
            isolated_profile_digest,
        )
        .expect("launch evidence");
        let attestation_tag = OpenClawLaunchAttestationTag::new(
            launch_attestation_tag(&attestation_key, &evidence).expect("attestation bytes"),
        )
        .expect("attestation tag");
        OpenClawOfficialLaunchRecord::verify_lattice_attestation(
            evidence,
            &attestation_key,
            attestation_tag,
        )
        .expect("launch record")
    }

    #[test]
    fn official_launch_record_requires_exact_lattice_attestation() {
        let launch_record_id = "launch-record-attested";
        let process_start_nonce =
            OpenClawProcessStartNonce::new([0x42; 16]).expect("process nonce");
        let package_tarball_digest = digest('a');
        let entrypoint_digest = digest('b');
        let isolated_profile_digest = digest('c');
        let attestation_key =
            OpenClawLaunchAttestationKey::new([0x43; 32]).expect("attestation key");
        let exact_evidence = OpenClawOfficialLaunchEvidence::new(
            launch_record_id,
            9001,
            process_start_nonce,
            package_tarball_digest.clone(),
            entrypoint_digest.clone(),
            isolated_profile_digest.clone(),
        )
        .expect("exact evidence");
        let attestation_tag = OpenClawLaunchAttestationTag::new(
            launch_attestation_tag(&attestation_key, &exact_evidence).expect("attestation bytes"),
        )
        .expect("attestation tag");

        let record = OpenClawOfficialLaunchRecord::verify_lattice_attestation(
            exact_evidence.clone(),
            &attestation_key,
            attestation_tag,
        )
        .expect("exact attestation");
        assert_eq!(record.process_id(), 9001);

        let wrong_tag = OpenClawLaunchAttestationTag::new([0x44; 32]).expect("wrong tag");
        assert_eq!(
            OpenClawOfficialLaunchRecord::verify_lattice_attestation(
                exact_evidence,
                &attestation_key,
                wrong_tag,
            )
            .expect_err("wrong tag must fail")
            .kind(),
            GatewayTransportErrorKind::Authentication
        );

        let substituted_evidence = OpenClawOfficialLaunchEvidence::new(
            launch_record_id,
            9002,
            process_start_nonce,
            package_tarball_digest,
            entrypoint_digest,
            isolated_profile_digest,
        )
        .expect("substituted evidence shape");
        assert_eq!(
            OpenClawOfficialLaunchRecord::verify_lattice_attestation(
                substituted_evidence,
                &attestation_key,
                attestation_tag,
            )
            .expect_err("PID substitution must fail")
            .kind(),
            GatewayTransportErrorKind::Authentication
        );
    }

    #[test]
    fn official_launch_gate_requires_authenticated_hello_before_command_dispatch() {
        let calls = Arc::new(AtomicUsize::new(0));
        let root_key = AuthenticationKey::new([0x31; TAG_BYTES]).expect("root key");
        let config = OpenClawGatewayConfig::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Duration::from_secs(2),
            ProjectId::new("project-a").expect("project"),
            transport_peer(),
            root_key.clone(),
        )
        .expect("config");
        let record = official_launch_record();
        let server = OpenClawGatewayServer::bind_official_launch(
            config,
            CountingRejectService(calls.clone()),
            record.clone(),
        )
        .expect("official-gated server");
        let endpoint = server.local_addr().expect("endpoint");
        let server_thread = thread::spawn(move || {
            let mut server = server;
            server.serve_once().expect("serve official command");
            server
        });

        let mut stream = TcpStream::connect(endpoint).expect("connect");
        configure_stream(&stream, Duration::from_secs(2)).expect("configure");
        let deadline = connection_deadline(Duration::from_secs(2)).expect("deadline");
        let (epoch, session_key) =
            read_transport_session_greeting(&mut stream, &root_key, deadline).expect("greeting");
        let hello = encode_openclaw_client_hello(&record.client_hello()).expect("hello");
        write_authenticated_packet(
            &mut stream,
            REQUEST_MAGIC,
            epoch,
            [0x32; NONCE_BYTES],
            &hello,
            &session_key,
            deadline,
        )
        .expect("write hello");

        let gateway_request = status_request();
        let GatewayRequestBody::Status(target) = gateway_request.body() else {
            panic!("status request");
        };
        let request = OpenClawStatusRequest::from_target(
            gateway_request.command_id().clone(),
            gateway_request.correlation_id().clone(),
            target.clone(),
        );
        let frame = encode_openclaw_status_request(&request).expect("status frame");
        write_authenticated_packet(
            &mut stream,
            REQUEST_MAGIC,
            epoch,
            [0x33; NONCE_BYTES],
            &frame,
            &session_key,
            deadline,
        )
        .expect("write command");
        let (reply_nonce, reply_frame) =
            read_authenticated_packet(&mut stream, RESPONSE_MAGIC, epoch, &session_key, deadline)
                .expect("read reply");
        assert_eq!(reply_nonce, [0x33; NONCE_BYTES]);
        decode_reply(&gateway_request, &reply_frame).expect("typed reply");

        let server = server_thread.join().expect("server thread");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            server.service().expect("service").0.load(Ordering::SeqCst),
            1
        );
    }

    #[test]
    fn official_launch_gate_rejects_a_command_frame_used_as_client_hello() {
        let calls = Arc::new(AtomicUsize::new(0));
        let root_key = AuthenticationKey::new([0x34; TAG_BYTES]).expect("root key");
        let config = OpenClawGatewayConfig::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Duration::from_secs(2),
            ProjectId::new("project-a").expect("project"),
            transport_peer(),
            root_key.clone(),
        )
        .expect("config");
        let server = OpenClawGatewayServer::bind_official_launch(
            config,
            CountingRejectService(calls.clone()),
            official_launch_record(),
        )
        .expect("official-gated server");
        let endpoint = server.local_addr().expect("endpoint");
        let server_thread = thread::spawn(move || {
            let mut server = server;
            server
                .serve_once()
                .expect_err("missing hello must fail")
                .kind()
        });

        let mut stream = TcpStream::connect(endpoint).expect("connect");
        configure_stream(&stream, Duration::from_secs(2)).expect("configure");
        let deadline = connection_deadline(Duration::from_secs(2)).expect("deadline");
        let (epoch, session_key) =
            read_transport_session_greeting(&mut stream, &root_key, deadline).expect("greeting");
        let gateway_request = status_request();
        let GatewayRequestBody::Status(target) = gateway_request.body() else {
            panic!("status request");
        };
        let request = OpenClawStatusRequest::from_target(
            gateway_request.command_id().clone(),
            gateway_request.correlation_id().clone(),
            target.clone(),
        );
        let frame = encode_openclaw_status_request(&request).expect("status frame");
        write_authenticated_packet(
            &mut stream,
            REQUEST_MAGIC,
            epoch,
            [0x35; NONCE_BYTES],
            &frame,
            &session_key,
            deadline,
        )
        .expect("write command in hello slot");

        assert_eq!(
            server_thread.join().expect("server thread"),
            GatewayTransportErrorKind::Authentication
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    struct CountingRejectService(Arc<AtomicUsize>);

    #[derive(Clone)]
    enum TestDurableIdempotencyEntry {
        Claimed(ContentDigest),
        Terminal(Box<OpenClawTerminalCommandRecord>),
    }

    #[derive(Clone, Default)]
    struct TestDurableIdempotencyStore {
        records: Arc<Mutex<BTreeMap<OpenClawCommandScope, TestDurableIdempotencyEntry>>>,
    }

    impl OpenClawIdempotencyStore for TestDurableIdempotencyStore {
        fn durability(&self) -> OpenClawIdempotencyDurability {
            OpenClawIdempotencyDurability::DurableTerminalReceipts
        }

        fn reconcile_and_claim(
            &mut self,
            scope: &OpenClawCommandScope,
            request: &GatewayRequest,
        ) -> Result<OpenClawIdempotencyDecision, OpenClawIdempotencyError> {
            if request.project_id() != scope.project_id()
                || request.command_id() != scope.command_id()
            {
                return Err(OpenClawIdempotencyError::Malformed);
            }
            let request_digest = request.request_digest();
            let mut records = self
                .records
                .lock()
                .map_err(|_| OpenClawIdempotencyError::Unavailable)?;
            if let Some(entry) = records.get(scope) {
                return Ok(match entry {
                    TestDurableIdempotencyEntry::Claimed(digest) if digest == request_digest => {
                        OpenClawIdempotencyDecision::InFlight
                    }
                    TestDurableIdempotencyEntry::Terminal(record)
                        if record.request_digest() == request_digest =>
                    {
                        OpenClawIdempotencyDecision::Exact(Box::new(record.reply().clone()))
                    }
                    TestDurableIdempotencyEntry::Claimed(_)
                    | TestDurableIdempotencyEntry::Terminal(_) => {
                        OpenClawIdempotencyDecision::CommandSubstitution
                    }
                });
            }
            records.insert(
                scope.clone(),
                TestDurableIdempotencyEntry::Claimed(request_digest.clone()),
            );
            Ok(OpenClawIdempotencyDecision::Claimed)
        }

        fn finalize_terminal(
            &mut self,
            record: OpenClawTerminalCommandRecord,
        ) -> Result<(), OpenClawIdempotencyError> {
            let mut records = self
                .records
                .lock()
                .map_err(|_| OpenClawIdempotencyError::Unavailable)?;
            match records.get(record.scope()) {
                Some(TestDurableIdempotencyEntry::Claimed(digest))
                    if digest == record.request_digest() =>
                {
                    records.insert(
                        record.scope().clone(),
                        TestDurableIdempotencyEntry::Terminal(Box::new(record)),
                    );
                    Ok(())
                }
                Some(TestDurableIdempotencyEntry::Terminal(existing))
                    if existing.as_ref() == &record =>
                {
                    Ok(())
                }
                Some(
                    TestDurableIdempotencyEntry::Claimed(_)
                    | TestDurableIdempotencyEntry::Terminal(_),
                )
                | None => Err(OpenClawIdempotencyError::Malformed),
            }
        }
    }

    impl GatewayService for CountingRejectService {
        fn handle(
            &mut self,
            _peer: GatewayPeerContext,
            _request: GatewayRequest,
        ) -> GatewayServiceResult<GatewayReply> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(GatewayServiceError::new(
                PortErrorKind::Denied,
                "TEST_ONLY_REJECT",
            ))
        }
    }

    fn assert_authenticated_raw_rejection(
        frame: &[u8],
        nonce: [u8; NONCE_BYTES],
        expected: GatewayTransportErrorKind,
    ) {
        let calls = Arc::new(AtomicUsize::new(0));
        let key = AuthenticationKey::new([0x51; TAG_BYTES]).expect("key");
        let config = OpenClawGatewayConfig::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Duration::from_millis(250),
            ProjectId::new("project-a").expect("project"),
            transport_peer(),
            key.clone(),
        )
        .expect("config");
        let server = OpenClawGatewayServer::bind(config, CountingRejectService(calls.clone()))
            .expect("server");
        let endpoint = server.local_addr().expect("endpoint");
        let server_thread = thread::spawn(move || {
            let mut server = server;
            server.serve_once().expect_err("codec must reject").kind()
        });
        let mut stream = TcpStream::connect(endpoint).expect("connect");
        configure_stream(&stream, Duration::from_secs(2)).expect("configure");
        let deadline = connection_deadline(Duration::from_secs(2)).expect("deadline");
        let (session_epoch, session_key) =
            read_transport_session_greeting(&mut stream, &key, deadline).expect("greeting");
        write_authenticated_packet(
            &mut stream,
            REQUEST_MAGIC,
            session_epoch,
            nonce,
            frame,
            &session_key,
            deadline,
        )
        .expect("write authenticated raw frame");

        assert_eq!(server_thread.join().expect("server thread"), expected);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn authenticated_unknown_schema_and_arbitrary_task_text_fail_before_service() {
        let encoded = encode_request(&status_request()).expect("encoded request");
        let text = String::from_utf8(encoded).expect("utf8");
        let unknown_schema = text
            .replace(
                "\"protocol\":\"lattice-gateway-ipc\"",
                "\"protocol\":\"unknown-schema\"",
            )
            .into_bytes();
        assert_authenticated_raw_rejection(
            &unknown_schema,
            [0x61; NONCE_BYTES],
            GatewayTransportErrorKind::Codec,
        );

        for (index, field) in [
            "credential",
            "memory",
            "path",
            "provider",
            "shell",
            "sql",
            "task_text",
        ]
        .into_iter()
        .enumerate()
        {
            let injected = text
                .replace(
                    ",\"version\":\"1\"}",
                    &format!(",\"{field}\":\"forbidden\",\"version\":\"1\"}}"),
                )
                .into_bytes();
            let mut nonce = [0x62; NONCE_BYTES];
            nonce[NONCE_BYTES - 1] = u8::try_from(index + 1).expect("bounded test index");
            assert_authenticated_raw_rejection(&injected, nonce, GatewayTransportErrorKind::Codec);
        }
    }

    #[test]
    fn authenticated_raw_full_submit_is_rejected_before_service() {
        let frame = encode_request(&full_submit_request()).expect("encoded full Submit");
        assert_authenticated_raw_rejection(
            &frame,
            [0x69; NONCE_BYTES],
            GatewayTransportErrorKind::ForbiddenPayload,
        );
    }

    #[test]
    fn stalled_partial_frame_hits_bounded_read_timeout_before_service() {
        let calls = Arc::new(AtomicUsize::new(0));
        let key = AuthenticationKey::new([0x52; TAG_BYTES]).expect("key");
        let config = OpenClawGatewayConfig::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Duration::from_millis(50),
            ProjectId::new("project-a").expect("project"),
            transport_peer(),
            key,
        )
        .expect("config");
        let server = OpenClawGatewayServer::bind(config, CountingRejectService(calls.clone()))
            .expect("server");
        let endpoint = server.local_addr().expect("endpoint");
        let server_thread = thread::spawn(move || {
            let mut server = server;
            server.serve_once().expect_err("read must time out").kind()
        });
        let mut stream = TcpStream::connect(endpoint).expect("connect");
        stream
            .write_all(&REQUEST_MAGIC[..4])
            .expect("partial header");

        assert_eq!(
            server_thread.join().expect("server thread"),
            GatewayTransportErrorKind::Timeout
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn slow_drip_frame_hits_one_absolute_connection_deadline() {
        let calls = Arc::new(AtomicUsize::new(0));
        let key = AuthenticationKey::new([0x55; TAG_BYTES]).expect("key");
        let config = OpenClawGatewayConfig::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Duration::from_millis(60),
            ProjectId::new("project-a").expect("project"),
            transport_peer(),
            key,
        )
        .expect("config");
        let server = OpenClawGatewayServer::bind(config, CountingRejectService(calls.clone()))
            .expect("server");
        let endpoint = server.local_addr().expect("endpoint");
        let server_thread = thread::spawn(move || {
            let mut server = server;
            let started = Instant::now();
            let kind = server
                .serve_once()
                .expect_err("slow drip must time out")
                .kind();
            (kind, started.elapsed())
        });
        let mut stream = TcpStream::connect(endpoint).expect("connect");
        for byte in REQUEST_MAGIC.iter().copied().cycle().take(16) {
            if stream.write_all(&[byte]).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        let (kind, elapsed) = server_thread.join().expect("server thread");
        assert_eq!(kind, GatewayTransportErrorKind::Timeout);
        assert!(elapsed < Duration::from_millis(150));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn stalled_unauthenticated_connection_cannot_monopolize_bounded_admission() {
        let calls = Arc::new(AtomicUsize::new(0));
        let key = AuthenticationKey::new([0x56; TAG_BYTES]).expect("key");
        let config = OpenClawGatewayConfig::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Duration::from_secs(2),
            ProjectId::new("project-a").expect("project"),
            transport_peer(),
            key.clone(),
        )
        .expect("config");
        let server = OpenClawGatewayServer::bind(config, CountingRejectService(calls.clone()))
            .expect("server");
        let endpoint = server.local_addr().expect("endpoint");
        let mut stalled = TcpStream::connect(endpoint).expect("stalled connect");
        stalled.write_all(&[REQUEST_MAGIC[0]]).expect("one byte");
        let client_thread = thread::spawn(move || {
            let client = OpenClawGatewayClient::new(endpoint, Duration::from_secs(1), key)
                .expect("valid client");
            client
                .send(
                    &status_request(),
                    TransportNonce::new([0x57; NONCE_BYTES]).expect("nonce"),
                )
                .expect("valid peer reply")
        });
        thread::sleep(Duration::from_millis(20));
        let server_thread = thread::spawn(move || {
            let mut server = server;
            server
                .serve_once()
                .expect("valid peer must pass stalled unauthenticated peer");
            server
        });
        let reply = client_thread.join().expect("client thread");
        assert!(matches!(reply.body(), GatewayReplyBody::Denied(_)));

        let server = server_thread.join().expect("server thread");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            server.service().expect("service").0.load(Ordering::SeqCst),
            1
        );
    }

    #[test]
    fn per_start_session_rotation_rejects_a_prior_session_frame() {
        let root_key = AuthenticationKey::new([0x58; TAG_BYTES]).expect("root key");
        let first_config = OpenClawGatewayConfig::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Duration::from_secs(2),
            ProjectId::new("project-a").expect("project"),
            transport_peer(),
            root_key.clone(),
        )
        .expect("first config");
        let first_server = OpenClawGatewayServer::bind(
            first_config,
            CountingRejectService(Arc::new(AtomicUsize::new(0))),
        )
        .expect("first server");
        let prior_epoch = first_server.session_epoch;
        let prior_key = first_server.authentication_key.clone();
        drop(first_server);

        let calls = Arc::new(AtomicUsize::new(0));
        let second_config = OpenClawGatewayConfig::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Duration::from_secs(2),
            ProjectId::new("project-a").expect("project"),
            transport_peer(),
            root_key.clone(),
        )
        .expect("second config");
        let second_server =
            OpenClawGatewayServer::bind(second_config, CountingRejectService(calls.clone()))
                .expect("second server");
        assert_ne!(prior_epoch, second_server.session_epoch);
        assert_ne!(prior_key, second_server.authentication_key);
        let endpoint = second_server.local_addr().expect("endpoint");
        let server_thread = thread::spawn(move || {
            let mut server = second_server;
            let error = server
                .serve_once()
                .expect_err("prior-session frame must fail");
            (server, error.kind())
        });
        let mut stream = TcpStream::connect(endpoint).expect("connect");
        configure_stream(&stream, Duration::from_secs(2)).expect("configure");
        let deadline = connection_deadline(Duration::from_secs(2)).expect("deadline");
        let _current_session =
            read_transport_session_greeting(&mut stream, &root_key, deadline).expect("greeting");
        let frame = encode_request(&status_request()).expect("frame");
        write_authenticated_packet(
            &mut stream,
            REQUEST_MAGIC,
            prior_epoch,
            [0x59; NONCE_BYTES],
            &frame,
            &prior_key,
            deadline,
        )
        .expect("write prior-session frame");

        let (server, error) = server_thread.join().expect("server thread");
        assert_eq!(error, GatewayTransportErrorKind::Authentication);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            server.service().expect("service").0.load(Ordering::SeqCst),
            0
        );
    }

    #[test]
    fn typed_durable_port_reconciles_terminal_reply_across_server_restarts() {
        let root_key = AuthenticationKey::new([0x5a; TAG_BYTES]).expect("root key");
        let request = status_request();
        let store = TestDurableIdempotencyStore::default();
        let first_calls = Arc::new(AtomicUsize::new(0));
        let first_config = OpenClawGatewayConfig::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Duration::from_secs(2),
            ProjectId::new("project-a").expect("project"),
            transport_peer(),
            root_key.clone(),
        )
        .expect("first config");
        let first_server = OpenClawGatewayServer::bind_with_durable_idempotency(
            first_config,
            CountingRejectService(first_calls.clone()),
            store.clone(),
        )
        .expect("first server");
        let first_endpoint = first_server.local_addr().expect("first endpoint");
        let first_thread = thread::spawn(move || {
            let mut server = first_server;
            server.serve_once().expect("first dispatch");
            server
        });
        let first_client =
            OpenClawGatewayClient::new(first_endpoint, Duration::from_secs(2), root_key.clone())
                .expect("first client");
        let first_reply = first_client
            .send(
                &request,
                TransportNonce::new([0x5b; NONCE_BYTES]).expect("first nonce"),
            )
            .expect("first reply");
        assert!(matches!(first_reply.body(), GatewayReplyBody::Denied(_)));
        drop(first_thread.join().expect("first server thread"));
        assert_eq!(first_calls.load(Ordering::SeqCst), 1);

        let second_calls = Arc::new(AtomicUsize::new(0));
        let second_config = OpenClawGatewayConfig::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Duration::from_secs(2),
            ProjectId::new("project-a").expect("project"),
            transport_peer(),
            root_key.clone(),
        )
        .expect("second config");
        let second_server = OpenClawGatewayServer::bind_with_durable_idempotency(
            second_config,
            CountingRejectService(second_calls.clone()),
            store,
        )
        .expect("second server");
        let second_endpoint = second_server.local_addr().expect("second endpoint");
        let second_thread = thread::spawn(move || {
            let mut server = second_server;
            server.serve_once().expect("durable reconciliation");
            server
        });
        let second_client =
            OpenClawGatewayClient::new(second_endpoint, Duration::from_secs(2), root_key)
                .expect("second client");
        let second_reply = second_client
            .send(
                &request,
                TransportNonce::new([0x5c; NONCE_BYTES]).expect("second nonce"),
            )
            .expect("reconciled reply");

        assert_eq!(first_reply, second_reply);
        let second_server = second_thread.join().expect("second server thread");
        assert_eq!(second_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            second_server
                .service()
                .expect("service")
                .0
                .load(Ordering::SeqCst),
            0
        );
    }

    #[test]
    fn durable_inflight_claim_after_restart_never_redispatches() {
        let request = status_request();
        let peer = transport_peer();
        let scope = OpenClawCommandScope::new(
            request.project_id().clone(),
            peer.actor_id().clone(),
            peer.session_epoch(),
            request.command_id().clone(),
        )
        .expect("scope");
        let store = TestDurableIdempotencyStore::default();
        assert_eq!(
            store
                .clone()
                .reconcile_and_claim(&scope, &request)
                .expect("persist prior-process claim"),
            OpenClawIdempotencyDecision::Claimed
        );

        let calls = Arc::new(AtomicUsize::new(0));
        let root_key = AuthenticationKey::new([0x5f; TAG_BYTES]).expect("root key");
        let config = OpenClawGatewayConfig::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Duration::from_secs(2),
            ProjectId::new("project-a").expect("project"),
            peer,
            root_key.clone(),
        )
        .expect("config");
        let server = OpenClawGatewayServer::bind_with_durable_idempotency(
            config,
            CountingRejectService(calls.clone()),
            store,
        )
        .expect("server");
        let endpoint = server.local_addr().expect("endpoint");
        let server_thread = thread::spawn(move || {
            let mut server = server;
            let error = server
                .serve_once()
                .expect_err("in-flight durable claim must remain ambiguous");
            (server, error.kind())
        });
        let client =
            OpenClawGatewayClient::new(endpoint, Duration::from_secs(2), root_key).expect("client");
        let _client_error = client
            .send(
                &request,
                TransportNonce::new([0x60; NONCE_BYTES]).expect("nonce"),
            )
            .expect_err("in-flight claim closes without a reply");

        let (server, error) = server_thread.join().expect("server thread");
        assert_eq!(error, GatewayTransportErrorKind::Ambiguous);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            server.service().expect("service").0.load(Ordering::SeqCst),
            0
        );
    }

    #[test]
    fn process_store_capacity_is_reserved_before_service_dispatch() {
        let mut store = ProcessMemoryOpenClawIdempotencyStore::default();
        let project = ProjectId::new("project-a").expect("project");
        let peer = transport_peer();
        let actor = peer.actor_id().clone();
        for index in 0..MAX_COMMAND_REPLAY_ENTRIES {
            let request = status_request_with_command(&format!("capacity-command-{index}"));
            let scope = OpenClawCommandScope::new(
                project.clone(),
                actor.clone(),
                peer.session_epoch(),
                request.command_id().clone(),
            )
            .expect("scope");
            assert_eq!(
                store
                    .reconcile_and_claim(&scope, &request)
                    .expect("bounded claim"),
                OpenClawIdempotencyDecision::Claimed
            );
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let root_key = AuthenticationKey::new([0x5d; TAG_BYTES]).expect("root key");
        let config = OpenClawGatewayConfig::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Duration::from_secs(2),
            project,
            transport_peer(),
            root_key.clone(),
        )
        .expect("config");
        let server = OpenClawGatewayServer::bind_with_store(
            config,
            CountingRejectService(calls.clone()),
            Box::new(store),
            None,
        )
        .expect("server");
        let endpoint = server.local_addr().expect("endpoint");
        let server_thread = thread::spawn(move || {
            let mut server = server;
            let error = server
                .serve_once()
                .expect_err("capacity must reject before service");
            (server, error.kind())
        });
        let client =
            OpenClawGatewayClient::new(endpoint, Duration::from_secs(2), root_key).expect("client");
        let _client_error = client
            .send(
                &status_request(),
                TransportNonce::new([0x5e; NONCE_BYTES]).expect("nonce"),
            )
            .expect_err("capacity rejection closes without a reply");

        let (server, error) = server_thread.join().expect("server thread");
        assert_eq!(error, GatewayTransportErrorKind::Capacity);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            server.service().expect("service").0.load(Ordering::SeqCst),
            0
        );
    }

    #[test]
    fn truncated_header_is_malformed_before_service() {
        let calls = Arc::new(AtomicUsize::new(0));
        let key = AuthenticationKey::new([0x54; TAG_BYTES]).expect("key");
        let config = OpenClawGatewayConfig::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Duration::from_secs(2),
            ProjectId::new("project-a").expect("project"),
            transport_peer(),
            key,
        )
        .expect("config");
        let server = OpenClawGatewayServer::bind(config, CountingRejectService(calls.clone()))
            .expect("server");
        let endpoint = server.local_addr().expect("endpoint");
        let server_thread = thread::spawn(move || {
            let mut server = server;
            server
                .serve_once()
                .expect_err("truncated header must fail")
                .kind()
        });
        let mut stream = TcpStream::connect(endpoint).expect("connect");
        stream
            .write_all(&REQUEST_MAGIC[..4])
            .expect("partial header");
        stream.shutdown(Shutdown::Write).expect("shutdown writer");

        assert_eq!(
            server_thread.join().expect("server thread"),
            GatewayTransportErrorKind::Malformed
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn public_bind_address_is_rejected_before_listener_creation() {
        let calls = Arc::new(AtomicUsize::new(0));
        let error = OpenClawGatewayConfig::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            Duration::from_secs(2),
            ProjectId::new("project-a").expect("project"),
            transport_peer(),
            AuthenticationKey::new([0x53; TAG_BYTES]).expect("key"),
        )
        .expect_err("public bind must fail");

        assert_eq!(error.kind(), GatewayTransportErrorKind::NonLocal);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
