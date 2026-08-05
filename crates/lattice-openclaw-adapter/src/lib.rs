//! Authenticated, loopback-only `OpenClaw` transport for the frozen gateway IPC.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError, sync_channel};
use std::thread;
use std::time::Duration;

use lattice_contracts::{
    GatewayClientKind, GatewayDenialCode, GatewayPeerContext, GatewayReply, GatewayReplyBody,
    GatewayRequest, GatewayRequestBody, GatewayUnknownCode, ProjectId, RuntimeKind,
};
use lattice_gateway_ipc::{
    MAX_FRAME_BYTES, build_reply, decode_reply, decode_request, encode_reply, encode_request,
};
use lattice_ports::{GatewayService, PortErrorKind};
use sha2::{Digest, Sha256};

const REQUEST_MAGIC: [u8; 8] = *b"LATGW001";
const RESPONSE_MAGIC: [u8; 8] = *b"LATGR001";
const NONCE_BYTES: usize = 16;
const TAG_BYTES: usize = 32;
const LENGTH_BYTES: usize = 4;
const HEADER_BYTES: usize = REQUEST_MAGIC.len() + NONCE_BYTES + LENGTH_BYTES + TAG_BYTES;
const HMAC_BLOCK_BYTES: usize = 64;
const MAX_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum authenticated transport nonces retained for one server process.
pub const MAX_AUTH_REPLAY_ENTRIES: usize = 4_096;
/// Maximum terminal command replies retained by one live adapter process.
pub const MAX_COMMAND_REPLAY_ENTRIES: usize = 1_024;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ReplayKey {
    project: String,
    actor: String,
    command: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplayRecord {
    request_digest: String,
    reply_frame: Vec<u8>,
}

struct PendingDispatch<S> {
    receiver: Receiver<(S, lattice_ports::GatewayServiceResult<GatewayReply>)>,
    request: GatewayRequest,
    replay_key: ReplayKey,
}

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
}

impl OpenClawGatewayConfig {
    /// Validates the live peer, project, local endpoint, timeout, and key.
    ///
    /// # Errors
    ///
    /// Rejects a non-loopback endpoint, fake/recovery peer, or timeout outside
    /// the bounded production range.
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
        if peer.runtime() != RuntimeKind::Live || peer.client_kind() != GatewayClientKind::OpenClaw
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
        })
    }
}

/// Stateful production listener around one injected Rust [`GatewayService`].
pub struct OpenClawGatewayServer<S> {
    listener: TcpListener,
    timeout: Duration,
    project_id: ProjectId,
    peer: GatewayPeerContext,
    authentication_key: AuthenticationKey,
    seen_nonces: BTreeSet<[u8; NONCE_BYTES]>,
    terminal_replies: BTreeMap<ReplayKey, ReplayRecord>,
    service: Option<S>,
    pending: Option<PendingDispatch<S>>,
}

impl<S> OpenClawGatewayServer<S>
where
    S: GatewayService + Send + 'static,
{
    /// Binds the configured loopback endpoint without opening a public listener.
    ///
    /// # Errors
    ///
    /// Returns a static unavailable or non-local failure when binding is unsafe.
    pub fn bind(config: OpenClawGatewayConfig, service: S) -> Result<Self, GatewayTransportError> {
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
        Ok(Self {
            listener,
            timeout: config.timeout,
            project_id: config.project_id,
            peer: config.peer,
            authentication_key: config.authentication_key,
            seen_nonces: BTreeSet::new(),
            terminal_replies: BTreeMap::new(),
            service: Some(service),
            pending: None,
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
        let (mut stream, remote_address) = self
            .listener
            .accept()
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Unavailable))?;
        if !remote_address.ip().is_loopback() {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::NonLocal,
            ));
        }
        configure_stream(&stream, self.timeout)?;
        let (nonce, frame) =
            read_authenticated_packet(&mut stream, REQUEST_MAGIC, &self.authentication_key)?;
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

        let request = decode_request(&frame)
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Codec))?;
        if request.project_id() != &self.project_id {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::CrossProject,
            ));
        }
        if matches!(request.body(), GatewayRequestBody::Submit(_)) {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::ForbiddenPayload,
            ));
        }
        let replay_key = ReplayKey {
            project: request.project_id().as_str().to_owned(),
            actor: self.peer.actor_id().as_str().to_owned(),
            command: request.command_id().as_str().to_owned(),
        };
        self.reconcile_pending()?;
        if let Some(record) = self.terminal_replies.get(&replay_key) {
            if record.request_digest == request.request_digest().as_str() {
                return write_authenticated_packet(
                    &mut stream,
                    RESPONSE_MAGIC,
                    nonce,
                    &record.reply_frame,
                    &self.authentication_key,
                );
            }
            let denial = build_reply(
                &request,
                GatewayReplyBody::Denied(GatewayDenialCode::CommandSubstitution),
            )
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Reply))?;
            let denial_frame = encode_reply(&denial)
                .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Reply))?;
            return write_authenticated_packet(
                &mut stream,
                RESPONSE_MAGIC,
                nonce,
                &denial_frame,
                &self.authentication_key,
            );
        }
        if self.terminal_replies.len() >= MAX_COMMAND_REPLAY_ENTRIES {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Capacity,
            ));
        }
        let reply_frame = self.dispatch_with_deadline(request, replay_key)?;
        write_authenticated_packet(
            &mut stream,
            RESPONSE_MAGIC,
            nonce,
            &reply_frame,
            &self.authentication_key,
        )
    }

    fn dispatch_with_deadline(
        &mut self,
        request: GatewayRequest,
        replay_key: ReplayKey,
    ) -> Result<Vec<u8>, GatewayTransportError> {
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

        match receiver.recv_timeout(self.timeout) {
            Ok((service, result)) => {
                self.service = Some(service);
                self.prepare_terminal_reply(&request, replay_key, result)
            }
            Err(RecvTimeoutError::Timeout) => {
                self.pending = Some(PendingDispatch {
                    receiver,
                    request,
                    replay_key,
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
        let Some(pending) = self.pending.take() else {
            return Ok(());
        };
        match pending.receiver.try_recv() {
            Ok((service, result)) => {
                self.service = Some(service);
                self.prepare_terminal_reply(&pending.request, pending.replay_key, result)?;
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

    fn prepare_terminal_reply(
        &mut self,
        request: &GatewayRequest,
        replay_key: ReplayKey,
        result: lattice_ports::GatewayServiceResult<GatewayReply>,
    ) -> Result<Vec<u8>, GatewayTransportError> {
        let reply = map_service_result(request, result)?;
        let reply_frame = encode_reply(&reply)
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Reply))?;
        decode_reply(request, &reply_frame)
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Reply))?;
        self.terminal_replies.insert(
            replay_key,
            ReplayRecord {
                request_digest: request.request_digest().as_str().to_owned(),
                reply_frame: reply_frame.clone(),
            },
        );
        Ok(reply_frame)
    }
}

/// Authenticated loopback client used by the `OpenClaw` edge and live preflight.
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
        let request_frame = encode_request(request)
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Codec))?;
        let mut stream = TcpStream::connect_timeout(&self.endpoint, self.timeout)
            .map_err(|error| map_connect_error(&error))?;
        configure_stream(&stream, self.timeout)?;
        let nonce_bytes = nonce.bytes();
        write_authenticated_packet(
            &mut stream,
            REQUEST_MAGIC,
            nonce_bytes,
            &request_frame,
            &self.authentication_key,
        )?;
        let (reply_nonce, reply_frame) =
            read_authenticated_packet(&mut stream, RESPONSE_MAGIC, &self.authentication_key)?;
        if reply_nonce != nonce_bytes {
            return Err(GatewayTransportError::new(
                GatewayTransportErrorKind::Authentication,
            ));
        }
        decode_reply(request, &reply_frame)
            .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Reply))
    }
}

fn configure_stream(stream: &TcpStream, timeout: Duration) -> Result<(), GatewayTransportError> {
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| stream.set_write_timeout(Some(timeout)))
        .and_then(|()| stream.set_nodelay(true))
        .map_err(|error| map_io_error(&error))
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
    key: &AuthenticationKey,
) -> Result<([u8; NONCE_BYTES], Vec<u8>), GatewayTransportError> {
    let mut header = [0_u8; HEADER_BYTES];
    stream
        .read_exact(&mut header)
        .map_err(|error| map_io_error(&error))?;
    if header[..REQUEST_MAGIC.len()] != expected_magic {
        return Err(GatewayTransportError::new(
            GatewayTransportErrorKind::Malformed,
        ));
    }
    let mut nonce = [0_u8; NONCE_BYTES];
    let nonce_start = REQUEST_MAGIC.len();
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
    stream
        .read_exact(&mut payload)
        .map_err(|error| map_io_error(&error))?;
    let actual_tag = authenticate(key, expected_magic, &nonce, length_bytes, &payload);
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
    nonce: [u8; NONCE_BYTES],
    payload: &[u8],
    key: &AuthenticationKey,
) -> Result<(), GatewayTransportError> {
    if payload.is_empty() || payload.len() > MAX_FRAME_BYTES {
        return Err(GatewayTransportError::new(
            GatewayTransportErrorKind::Malformed,
        ));
    }
    let payload_length = u32::try_from(payload.len())
        .map_err(|_| GatewayTransportError::new(GatewayTransportErrorKind::Malformed))?;
    let length_bytes = payload_length.to_be_bytes();
    let tag = authenticate(key, magic, &nonce, length_bytes, payload);
    let mut packet = Vec::with_capacity(HEADER_BYTES + payload.len());
    packet.extend_from_slice(&magic);
    packet.extend_from_slice(&nonce);
    packet.extend_from_slice(&length_bytes);
    packet.extend_from_slice(&tag);
    packet.extend_from_slice(payload);
    stream
        .write_all(&packet)
        .map_err(|error| map_io_error(&error))
}

fn authenticate(
    key: &AuthenticationKey,
    magic: [u8; REQUEST_MAGIC.len()],
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
    use std::net::{IpAddr, Ipv4Addr, Shutdown};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    use lattice_contracts::{
        ContentDigest, GatewayActorId, GatewayAdapterId, GatewayChannelId, GatewayCommandId,
        GatewayCorrelationId, GatewayInstanceId, GatewayProjectStatusTarget, GatewayRequestBody,
        GatewaySessionId, GatewayStatusTarget,
    };
    use lattice_ports::{GatewayServiceError, GatewayServiceResult};

    use super::*;

    fn digest(fill: char) -> ContentDigest {
        ContentDigest::from_sha256(fill.to_string().repeat(64)).expect("digest")
    }

    fn live_peer() -> GatewayPeerContext {
        GatewayPeerContext::new_authenticated_openclaw(
            GatewayInstanceId::new("gateway-live-unit").expect("gateway"),
            GatewayAdapterId::new("openclaw-adapter").expect("adapter"),
            "1.0.0",
            digest('a'),
            digest('b'),
            GatewayActorId::new("responsible-user-unit").expect("actor"),
            GatewayChannelId::new("openclaw-local-unit").expect("channel"),
            GatewaySessionId::new("session-live-unit").expect("session"),
            1,
            digest('c'),
            digest('c'),
        )
        .expect("live peer")
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

    struct CountingRejectService(Arc<AtomicUsize>);

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

    fn assert_authenticated_raw_codec_rejection(frame: &[u8], nonce: [u8; NONCE_BYTES]) {
        let calls = Arc::new(AtomicUsize::new(0));
        let key = AuthenticationKey::new([0x51; TAG_BYTES]).expect("key");
        let config = OpenClawGatewayConfig::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Duration::from_secs(2),
            ProjectId::new("project-a").expect("project"),
            live_peer(),
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
        write_authenticated_packet(&mut stream, REQUEST_MAGIC, nonce, frame, &key)
            .expect("write authenticated raw frame");

        assert_eq!(
            server_thread.join().expect("server thread"),
            GatewayTransportErrorKind::Codec
        );
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
        assert_authenticated_raw_codec_rejection(&unknown_schema, [0x61; NONCE_BYTES]);

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
            assert_authenticated_raw_codec_rejection(&injected, nonce);
        }
    }

    #[test]
    fn stalled_partial_frame_hits_bounded_read_timeout_before_service() {
        let calls = Arc::new(AtomicUsize::new(0));
        let key = AuthenticationKey::new([0x52; TAG_BYTES]).expect("key");
        let config = OpenClawGatewayConfig::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Duration::from_millis(50),
            ProjectId::new("project-a").expect("project"),
            live_peer(),
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
    fn truncated_header_is_malformed_before_service() {
        let calls = Arc::new(AtomicUsize::new(0));
        let key = AuthenticationKey::new([0x54; TAG_BYTES]).expect("key");
        let config = OpenClawGatewayConfig::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Duration::from_secs(2),
            ProjectId::new("project-a").expect("project"),
            live_peer(),
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
            live_peer(),
            AuthenticationKey::new([0x53; TAG_BYTES]).expect("key"),
        )
        .expect_err("public bind must fail");

        assert_eq!(error.kind(), GatewayTransportErrorKind::NonLocal);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
