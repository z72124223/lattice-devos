//! Authenticated, loopback-only `OpenClaw` transport for the frozen gateway IPC.

mod wire;

pub use wire::{
    OpenClawSubmitReply, OpenClawSubmitReplyBody, OpenClawSubmitRequest,
    encode_openclaw_submit_request,
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
    ContentDigest, GatewayActorId, GatewayClientKind, GatewayCommandId, GatewayDenialCode,
    GatewayPeerContext, GatewayReply, GatewayReplyBody, GatewayRequest, GatewayRequestBody,
    GatewayUnknownCode, ProjectId, RuntimeKind, TaskSpecSubmission,
};
use lattice_gateway_ipc::{
    MAX_FRAME_BYTES, build_reply, build_request, decode_reply, decode_request, encode_reply,
    encode_request, verify_task_spec_document,
};
use lattice_ports::{GatewayService, PortErrorKind};
use sha2::{Digest, Sha256};

const REQUEST_MAGIC: [u8; 8] = *b"LATGW001";
const RESPONSE_MAGIC: [u8; 8] = *b"LATGR001";
const SESSION_MAGIC: [u8; 8] = *b"LATSN001";
const SESSION_EPOCH_BYTES: usize = 16;
const NONCE_BYTES: usize = 16;
const TAG_BYTES: usize = 32;
const LENGTH_BYTES: usize = 4;
const HEADER_BYTES: usize =
    REQUEST_MAGIC.len() + SESSION_EPOCH_BYTES + NONCE_BYTES + LENGTH_BYTES + TAG_BYTES;
const SESSION_GREETING_BYTES: usize = SESSION_MAGIC.len() + SESSION_EPOCH_BYTES;
const HMAC_BLOCK_BYTES: usize = 64;
const MAX_TIMEOUT: Duration = Duration::from_secs(30);

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

/// Exact command scope used by the durable terminal-idempotency seam.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct OpenClawCommandScope {
    project: ProjectId,
    actor: GatewayActorId,
    command: GatewayCommandId,
}

impl OpenClawCommandScope {
    /// Returns the command's exact project.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project
    }

    /// Returns the server-derived actor identity.
    #[must_use]
    pub const fn actor_id(&self) -> &GatewayActorId {
        &self.actor
    }

    /// Returns the idempotent semantic command identity.
    #[must_use]
    pub const fn command_id(&self) -> &GatewayCommandId {
        &self.command
    }
}

/// One typed terminal command record suitable for durable receipt storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenClawTerminalCommandRecord {
    scope: OpenClawCommandScope,
    request_digest: ContentDigest,
    reply: GatewayReply,
}

impl OpenClawTerminalCommandRecord {
    /// Returns the exact project/actor/command scope.
    #[must_use]
    pub const fn scope(&self) -> &OpenClawCommandScope {
        &self.scope
    }

    /// Returns the exact reconstructed gateway request digest.
    #[must_use]
    pub const fn request_digest(&self) -> &ContentDigest {
        &self.request_digest
    }

    /// Returns the validated terminal gateway reply.
    #[must_use]
    pub const fn reply(&self) -> &GatewayReply {
        &self.reply
    }
}

/// Persistence claim exposed by a terminal-idempotency provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenClawIdempotencyDurability {
    /// Process-local checkpoint storage; never restart-safe.
    ProcessMemory,
    /// Durable terminal command receipts reconciled across process starts.
    DurableTerminalReceipts,
}

/// Typed result of reconciling one command before dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpenClawIdempotencyDecision {
    /// The exact request now owns a bounded pre-dispatch claim.
    Claimed,
    /// The exact request is already claimed but has no terminal reply yet.
    InFlight,
    /// The exact request already has a terminal reply.
    Exact(Box<GatewayReply>),
    /// The command scope exists under a different request digest.
    CommandSubstitution,
}

/// Closed idempotency-provider failure without backend details.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenClawIdempotencyError {
    /// The provider could not be reached.
    Unavailable,
    /// The provider's bounded capacity was exhausted.
    Capacity,
    /// The provider returned a malformed or contradictory record.
    Malformed,
}

/// Typed terminal-command idempotency port. A `PostgreSQL` implementation is an
/// integration-window responsibility and is not supplied by this transport crate.
pub trait OpenClawIdempotencyStore: Send {
    /// States whether records survive a process restart.
    fn durability(&self) -> OpenClawIdempotencyDurability;

    /// Atomically reconciles or claims one command before dispatch.
    ///
    /// A `Claimed` result guarantees that bounded capacity has already been
    /// reserved before the caller may invoke `GatewayService`. Providers must
    /// return `InFlight` for an existing non-terminal claim with the same
    /// digest and must never issue a second claim for that command scope.
    ///
    /// # Errors
    ///
    /// Returns a closed provider failure without backend detail.
    fn reconcile_and_claim(
        &mut self,
        scope: &OpenClawCommandScope,
        request_digest: &ContentDigest,
    ) -> Result<OpenClawIdempotencyDecision, OpenClawIdempotencyError>;

    /// Finalizes one existing claim with a validated terminal reply.
    ///
    /// Finalization must be idempotent for an identical record. It must fail
    /// closed rather than creating a terminal record without a matching claim.
    ///
    /// # Errors
    ///
    /// Returns a closed provider failure without backend detail.
    fn finalize_terminal(
        &mut self,
        record: OpenClawTerminalCommandRecord,
    ) -> Result<(), OpenClawIdempotencyError>;
}

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
        request_digest: &ContentDigest,
    ) -> Result<OpenClawIdempotencyDecision, OpenClawIdempotencyError> {
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
        Self::bind_with_store(config, service, Box::new(store))
    }

    fn bind_with_store(
        config: OpenClawGatewayConfig,
        service: S,
        idempotency_store: Box<dyn OpenClawIdempotencyStore>,
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
            nonce,
            frame,
            deadline: connection_deadline,
        } = self.accept_authenticated_connection()?;
        if self.seen_nonces.contains(&nonce) {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Replay,
            ));
        }
        if self.seen_nonces.len() >= MAX_AUTH_REPLAY_ENTRIES {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Capacity,
            ));
        }
        self.seen_nonces.insert(nonce);

        let (request, reply_encoding) = self.decode_ingress(&frame)?;
        if request.project_id() != &self.project_id {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::CrossProject,
            ));
        }
        let replay_key = OpenClawCommandScope {
            project: request.project_id().clone(),
            actor: self.peer.actor_id().clone(),
            command: request.command_id().clone(),
        };
        self.reconcile_pending()?;
        match self
            .idempotency_store
            .reconcile_and_claim(&replay_key, request.request_digest())
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
        if let Ok(request) = decode_request(frame) {
            if matches!(request.body(), GatewayRequestBody::Submit(_)) {
                return Err(GatewayTransportError::new(
                    GatewayTransportErrorKind::ForbiddenPayload,
                ));
            }
            return Ok((request, ReplyEncoding::Gateway));
        }
        let selector = wire::decode_openclaw_submit_request(frame)?;
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
        Ok((request, ReplyEncoding::OpenClawSubmit))
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
                thread::Builder::new()
                    .name("lattice-openclaw-gateway-auth".to_owned())
                    .spawn(move || {
                        let result = prepare_authenticated_connection(
                            stream,
                            remote_address,
                            timeout,
                            session_epoch,
                            &key,
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
        let record = OpenClawTerminalCommandRecord {
            scope: replay_key,
            request_digest: request.request_digest().clone(),
            reply,
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
        if matches!(request.body(), GatewayRequestBody::Submit(_)) {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::ForbiddenPayload,
            ));
        }
        let request_frame = encode_request(request)
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Codec))?;
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
    let (nonce, frame) =
        read_authenticated_packet(&mut stream, REQUEST_MAGIC, session_epoch, key, deadline)?;
    Ok(PreparedConnection {
        stream,
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
    let mut greeting = [0_u8; SESSION_GREETING_BYTES];
    greeting[..SESSION_MAGIC.len()].copy_from_slice(&SESSION_MAGIC);
    greeting[SESSION_MAGIC.len()..].copy_from_slice(&epoch.0);
    write_all_until(stream, &greeting, deadline)
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
    write_all_until(stream, &packet, deadline)
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
        lattice_gateway_ipc::build_request(
            GatewayCommandId::new("command-unit").expect("command"),
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
            request_digest: &ContentDigest,
        ) -> Result<OpenClawIdempotencyDecision, OpenClawIdempotencyError> {
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
        let scope = OpenClawCommandScope {
            project: request.project_id().clone(),
            actor: peer.actor_id().clone(),
            command: request.command_id().clone(),
        };
        let store = TestDurableIdempotencyStore::default();
        assert_eq!(
            store
                .clone()
                .reconcile_and_claim(&scope, request.request_digest())
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
        let actor = transport_peer().actor_id().clone();
        let request_digest = status_request().request_digest().clone();
        for index in 0..MAX_COMMAND_REPLAY_ENTRIES {
            let scope = OpenClawCommandScope {
                project: project.clone(),
                actor: actor.clone(),
                command: GatewayCommandId::new(format!("capacity-command-{index}"))
                    .expect("command"),
            };
            assert_eq!(
                store
                    .reconcile_and_claim(&scope, &request_digest)
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
