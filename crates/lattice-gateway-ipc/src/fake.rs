use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::fmt;

use lattice_contracts::{
    GatewayClientKind, GatewayDenialCode, GatewayPeerContext, GatewayReply, GatewayReplyBody,
    GatewayRequest, GatewayRequestBody, GatewayUnknownCode,
};
use lattice_ports::{GatewayService, PortErrorKind};

use crate::{CodecError, build_reply, decode_reply, decode_request, encode_reply, encode_request};

/// Maximum terminal command records retained by one fake server.
pub const MAX_REPLAY_ENTRIES: usize = 1_024;

/// Deterministic fault injected by the pure loopback fake.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FakeFault {
    /// Endpoint is unavailable before dispatch.
    Unavailable,
    /// Request times out before dispatch.
    Timeout,
    /// Request is cancelled before dispatch.
    Cancelled,
    /// Service produces and caches a terminal reply, but the client loses it.
    AmbiguousAfterDispatch,
}

/// Stable fake transport/error classification kept separate from typed replies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoopbackErrorKind {
    /// Raw frame or reply codec rejection.
    Codec,
    /// Service or endpoint unavailable.
    Unavailable,
    /// Version disagreement.
    VersionMismatch,
    /// Malformed downstream behavior.
    Malformed,
    /// The bounded fake cannot retain another terminal command.
    Capacity,
    /// Bounded timeout.
    Timeout,
    /// Explicit cancellation.
    Cancelled,
    /// Dispatch may have completed; reconciliation or exact retry is required.
    Ambiguous,
}

/// Bounded, payload-free fake transport failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoopbackError {
    kind: LoopbackErrorKind,
    code: &'static str,
}

impl LoopbackError {
    const fn new(kind: LoopbackErrorKind, code: &'static str) -> Self {
        Self { kind, code }
    }

    /// Returns the stable failure class.
    #[must_use]
    pub const fn kind(self) -> LoopbackErrorKind {
        self.kind
    }

    /// Returns a stable payload-free code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        self.code
    }
}

impl fmt::Display for LoopbackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl Error for LoopbackError {}

impl From<CodecError> for LoopbackError {
    fn from(error: CodecError) -> Self {
        let kind = if error.kind() == crate::CodecErrorKind::UnsupportedVersion {
            LoopbackErrorKind::VersionMismatch
        } else {
            LoopbackErrorKind::Codec
        };
        Self::new(kind, error.code())
    }
}

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

/// Pure in-memory fake server around one injected [`GatewayService`].
///
/// It owns no listener or durable state. Peer context is an explicit argument
/// and never decoded from attacker-controlled frame bytes.
pub struct FakeGatewayServer<S> {
    service: S,
    replay: BTreeMap<ReplayKey, ReplayRecord>,
    faults: VecDeque<FakeFault>,
}

impl<S> FakeGatewayServer<S>
where
    S: GatewayService,
{
    /// Creates an empty fake server around an injected service.
    #[must_use]
    pub fn new(service: S) -> Self {
        Self {
            service,
            replay: BTreeMap::new(),
            faults: VecDeque::new(),
        }
    }

    /// Returns the injected service for test-only observations.
    #[must_use]
    pub const fn service(&self) -> &S {
        &self.service
    }

    /// Returns the injected service mutably for deterministic test setup.
    #[must_use]
    pub const fn service_mut(&mut self) -> &mut S {
        &mut self.service
    }

    /// Appends one deterministic fault to the fake script.
    pub fn push_fault(&mut self, fault: FakeFault) {
        self.faults.push_back(fault);
    }

    /// Handles one raw frame under separately supplied fake peer context.
    ///
    /// # Errors
    ///
    /// Returns a bounded codec or simulated transport failure. Business denial
    /// and ambiguous downstream outcomes are encoded as typed replies.
    pub fn handle_frame(
        &mut self,
        peer: GatewayPeerContext,
        frame: &[u8],
    ) -> Result<Vec<u8>, LoopbackError> {
        let request = decode_request(frame)?;
        let key = ReplayKey {
            project: request.project_id().as_str().to_owned(),
            actor: peer.actor_id().as_str().to_owned(),
            command: request.command_id().as_str().to_owned(),
        };

        // Authorization is a security precondition, not a mutable observation.
        // Check it before replay so a cached normal reply cannot be disclosed
        // through a narrower recovery surface. Rebuilding this deterministic
        // denial also prevents the recovery surface from poisoning the key.
        if !role_allows(peer.client_kind(), request.body()) {
            return Self::local_reply(
                &request,
                GatewayReplyBody::Denied(GatewayDenialCode::RoleDenied),
            );
        }

        if let Some(record) = self.replay.get(&key) {
            if record.request_digest == request.request_digest().as_str() {
                return Ok(record.reply_frame.clone());
            }
            return Self::local_reply(
                &request,
                GatewayReplyBody::Denied(GatewayDenialCode::CommandSubstitution),
            );
        }
        if self.replay.len() >= MAX_REPLAY_ENTRIES {
            return Err(LoopbackError::new(
                LoopbackErrorKind::Capacity,
                "GATEWAY_FAKE_REPLAY_CAPACITY",
            ));
        }

        let drop_reply = self.consume_fault()?;
        let service_result = self.service.handle(peer, request.clone());
        let reply = match service_result {
            Ok(reply) => {
                verify_reply_binding(&request, &reply)?;
                reply
            }
            Err(error) => match error.kind() {
                PortErrorKind::Denied => build_reply(
                    &request,
                    GatewayReplyBody::Denied(GatewayDenialCode::DownstreamDenied),
                )?,
                PortErrorKind::Ambiguous => build_reply(
                    &request,
                    GatewayReplyBody::UnknownOutcome(GatewayUnknownCode::DownstreamAmbiguous),
                )?,
                PortErrorKind::Unavailable => {
                    return Err(LoopbackError::new(
                        LoopbackErrorKind::Unavailable,
                        "GATEWAY_SERVICE_UNAVAILABLE",
                    ));
                }
                PortErrorKind::Timeout => {
                    return Err(LoopbackError::new(
                        LoopbackErrorKind::Timeout,
                        "GATEWAY_SERVICE_TIMEOUT",
                    ));
                }
                PortErrorKind::Cancelled => {
                    return Err(LoopbackError::new(
                        LoopbackErrorKind::Cancelled,
                        "GATEWAY_SERVICE_CANCELLED",
                    ));
                }
                PortErrorKind::VersionMismatch => {
                    return Err(LoopbackError::new(
                        LoopbackErrorKind::VersionMismatch,
                        "GATEWAY_SERVICE_VERSION_MISMATCH",
                    ));
                }
                PortErrorKind::CapabilityMismatch | PortErrorKind::Malformed => {
                    return Err(LoopbackError::new(
                        LoopbackErrorKind::Malformed,
                        "GATEWAY_SERVICE_MALFORMED",
                    ));
                }
            },
        };
        let reply_frame = encode_reply(&reply)?;
        self.replay.insert(
            key,
            ReplayRecord {
                request_digest: request.request_digest().as_str().to_owned(),
                reply_frame: reply_frame.clone(),
            },
        );
        if drop_reply {
            Err(LoopbackError::new(
                LoopbackErrorKind::Ambiguous,
                "GATEWAY_FAKE_REPLY_LOST",
            ))
        } else {
            Ok(reply_frame)
        }
    }

    fn local_reply(
        request: &GatewayRequest,
        body: GatewayReplyBody,
    ) -> Result<Vec<u8>, LoopbackError> {
        let reply = build_reply(request, body)?;
        encode_reply(&reply).map_err(Into::into)
    }

    fn consume_fault(&mut self) -> Result<bool, LoopbackError> {
        match self.faults.pop_front() {
            Some(FakeFault::Unavailable) => Err(LoopbackError::new(
                LoopbackErrorKind::Unavailable,
                "GATEWAY_FAKE_UNAVAILABLE",
            )),
            Some(FakeFault::Timeout) => Err(LoopbackError::new(
                LoopbackErrorKind::Timeout,
                "GATEWAY_FAKE_TIMEOUT",
            )),
            Some(FakeFault::Cancelled) => Err(LoopbackError::new(
                LoopbackErrorKind::Cancelled,
                "GATEWAY_FAKE_CANCELLED",
            )),
            Some(FakeFault::AmbiguousAfterDispatch) => Ok(true),
            None => Ok(false),
        }
    }
}

/// Pure client that exercises the same encoded frame contract as a future
/// local transport while retaining no transport or listener handle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeGatewayClient {
    peer: GatewayPeerContext,
}

impl FakeGatewayClient {
    /// Constructs a fake client with explicitly out-of-band peer context.
    #[must_use]
    pub const fn new(peer: GatewayPeerContext) -> Self {
        Self { peer }
    }

    /// Canonically encodes, loopback-dispatches, and decodes one request.
    ///
    /// # Errors
    ///
    /// Returns a bounded codec or simulated transport failure.
    pub fn send<S>(
        &self,
        server: &mut FakeGatewayServer<S>,
        request: &GatewayRequest,
    ) -> Result<GatewayReply, LoopbackError>
    where
        S: GatewayService,
    {
        let frame = encode_request(request)?;
        let reply_frame = server.handle_frame(self.peer.clone(), &frame)?;
        decode_reply(request, &reply_frame).map_err(Into::into)
    }
}

fn role_allows(client: GatewayClientKind, body: &GatewayRequestBody) -> bool {
    !matches!(client, GatewayClientKind::RecoveryCli)
        || matches!(
            body,
            GatewayRequestBody::Status(_) | GatewayRequestBody::Stop(_)
        )
}

fn verify_reply_binding(
    request: &GatewayRequest,
    reply: &GatewayReply,
) -> Result<(), LoopbackError> {
    if reply.version() != request.version()
        || reply.command_id() != request.command_id()
        || reply.correlation_id() != request.correlation_id()
        || reply.action() != request.action()
        || reply.request_digest() != request.request_digest()
    {
        return Err(LoopbackError::new(
            LoopbackErrorKind::Malformed,
            "GATEWAY_SERVICE_REPLY_MISMATCH",
        ));
    }
    encode_reply(reply)?;
    Ok(())
}
