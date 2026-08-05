//! Unique owned Windows -> WSL2 -> bubblewrap Hermes construction chain.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lattice_contracts::{ContentDigest, HermesEvidence, HermesResearchRequest, RequestId};
use lattice_ports::{HermesPort, PortError, PortResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::broker::CodexBrokerReceipt;
use crate::codex_proxy::{
    ProductionCodexProxyControl, ProductionCodexProxyDuplex, ProductionCodexProxyProvider,
};
use crate::containment::{
    HermesContainmentFrameLimits, HermesWslContainmentConfig, OUTER_RUNNER_SOURCE,
    PRIVATE_RUNNER_SOURCE, WSL_DISTRO, minimal_wsl_environment, parse_containment_frame,
};
use crate::runtime::HermesOfflineRuntimeManifest;
use crate::{
    CanonicalReflection, ContainmentOwnerState, HermesAdapterConfig, HermesAdapterError,
    HermesAdapterErrorKind, HermesAdapterResult, HermesContainmentReceipt, HermesReflectionAdapter,
    HermesReflectionEvidence, HermesReflectionJob, cross_binding, encode_sha256, error, malformed,
    map_port_error, sha256_text,
};

const STARTUP_MAGIC: &[u8] = b"LATTICE_HERMES_PRODUCTION_START_V1\n";
const STARTUP_SCHEMA: &str = "lattice.hermes.production-start.v1";
const ATTESTATION_SCHEMA: &str = "lattice.hermes.containment-attestation.v2";
const CONFIG_SCHEMA: &str = "lattice.hermes.production-config.v1";
const BWRAP_SHA256: &str = "8e19e40e7d5f7a7e8b488c7926feb040eab6ed10c58fa360e266d2f70670e92b";
const MAX_STARTUP_BYTES: usize = 128 * 1024;
const MAX_RUNNER_TIMEOUT: Duration = Duration::from_mins(5);
const CODEX_PROXY_MAGIC: &[u8] = b"LATTICE_HERMES_CODEX_PROXY_V1\n";
const CODEX_PROXY_BINDING_DOMAIN: &[u8] = b"LATTICE_HERMES_CODEX_PROXY_V1";
const CODEX_PROXY_STREAM_ID: u32 = 1;
const CODEX_PROXY_HEADER_BYTES: usize = 41;
const MAX_CODEX_PROXY_DATA_BYTES: usize = 65_536;
const MAX_CODEX_PROXY_BODY_BYTES: usize = CODEX_PROXY_HEADER_BYTES + MAX_CODEX_PROXY_DATA_BYTES;
const MAX_CODEX_PROXY_WIRE_BYTES: usize = CODEX_PROXY_MAGIC.len() + 4 + MAX_CODEX_PROXY_BODY_BYTES;
const MAX_CODEX_PROXY_BUFFER_BYTES: usize = MAX_CODEX_PROXY_WIRE_BYTES + 8192;
const CODEX_PROXY_TEARDOWN_TIMEOUT: Duration = Duration::from_secs(3);
static RUNNER_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CodexProxyHostEvent {
    Open,
    Data(Vec<u8>),
    Close,
    Error(u16),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexProxyFailureEvidence {
    byte_count: u64,
    sha256: String,
}

impl CodexProxyFailureEvidence {
    #[must_use]
    pub const fn byte_count(&self) -> u64 {
        self.byte_count
    }

    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

pub(crate) struct CodexProxyHostSession {
    binding: [u8; 32],
    deadline: Instant,
    expected_sequence: u32,
    outbound_sequence: u32,
    inbound: CodexProxyInboundState,
    outbound: CodexProxyOutboundState,
    failure_evidence: Option<CodexProxyFailureEvidence>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CodexProxyInboundState {
    AwaitOpen,
    Open,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CodexProxyOutboundState {
    AwaitOpenAck,
    Open,
    Closed,
}

impl CodexProxyHostSession {
    pub(crate) fn new(
        nonce: &str,
        broker_receipt: &ContentDigest,
        deadline: Instant,
    ) -> HermesAdapterResult<Self> {
        if deadline <= Instant::now() {
            return Err(error(
                HermesAdapterErrorKind::Timeout,
                "HERMES_CODEX_PROXY_DEADLINE_EXCEEDED",
            ));
        }
        let nonce = decode_hex(nonce)
            .map_err(|_| malformed("HERMES_CODEX_PROXY_BINDING_INPUT_REJECTED"))?;
        let broker = decode_hex(broker_receipt.as_str())
            .map_err(|_| malformed("HERMES_CODEX_PROXY_BINDING_INPUT_REJECTED"))?;
        let digest = digest_join(&[&nonce, &broker, CODEX_PROXY_BINDING_DOMAIN]);
        let binding: [u8; 32] = decode_hex(&digest)
            .map_err(|_| malformed("HERMES_CODEX_PROXY_BINDING_INPUT_REJECTED"))?
            .try_into()
            .map_err(|_| malformed("HERMES_CODEX_PROXY_BINDING_INPUT_REJECTED"))?;
        Ok(Self {
            binding,
            deadline,
            expected_sequence: 0,
            outbound_sequence: 0,
            inbound: CodexProxyInboundState::AwaitOpen,
            outbound: CodexProxyOutboundState::AwaitOpenAck,
            failure_evidence: None,
        })
    }

    #[cfg(test)]
    pub(crate) const fn binding(&self) -> [u8; 32] {
        self.binding
    }

    pub(crate) fn failure_evidence(&self) -> Option<&CodexProxyFailureEvidence> {
        self.failure_evidence.as_ref()
    }

    const fn inbound_closed(&self) -> bool {
        matches!(self.inbound, CodexProxyInboundState::Closed)
    }

    const fn outbound_closed(&self) -> bool {
        matches!(self.outbound, CodexProxyOutboundState::Closed)
    }

    pub(crate) fn encode_open_ack(&mut self) -> HermesAdapterResult<Vec<u8>> {
        if self.inbound != CodexProxyInboundState::Open
            || self.outbound != CodexProxyOutboundState::AwaitOpenAck
        {
            return Err(malformed("HERMES_CODEX_PROXY_STATE_REJECTED"));
        }
        let frame = self.encode_outbound(1, &[])?;
        self.outbound = CodexProxyOutboundState::Open;
        Ok(frame)
    }

    pub(crate) fn encode_data(&mut self, payload: &[u8]) -> HermesAdapterResult<Vec<u8>> {
        if self.outbound != CodexProxyOutboundState::Open
            || payload.is_empty()
            || payload.len() > MAX_CODEX_PROXY_DATA_BYTES
        {
            return Err(malformed("HERMES_CODEX_PROXY_STATE_REJECTED"));
        }
        self.encode_outbound(2, payload)
    }

    pub(crate) fn encode_close(&mut self) -> HermesAdapterResult<Vec<u8>> {
        if self.outbound != CodexProxyOutboundState::Open {
            return Err(malformed("HERMES_CODEX_PROXY_STATE_REJECTED"));
        }
        let frame = self.encode_outbound(3, &[])?;
        self.outbound = CodexProxyOutboundState::Closed;
        Ok(frame)
    }

    pub(crate) fn accept(&mut self, frame: &[u8]) -> HermesAdapterResult<CodexProxyHostEvent> {
        let result = self.accept_inner(frame);
        if result.is_err() {
            self.record_failure(frame);
        }
        result
    }

    fn record_failure(&mut self, bytes: &[u8]) {
        self.failure_evidence = Some(CodexProxyFailureEvidence {
            byte_count: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            sha256: encode_sha256(&Sha256::digest(bytes)),
        });
    }

    fn accept_inner(&mut self, frame: &[u8]) -> HermesAdapterResult<CodexProxyHostEvent> {
        if Instant::now() >= self.deadline {
            return Err(error(
                HermesAdapterErrorKind::Timeout,
                "HERMES_CODEX_PROXY_DEADLINE_EXCEEDED",
            ));
        }
        let Some(encoded) = frame.strip_prefix(CODEX_PROXY_MAGIC) else {
            return Err(malformed("HERMES_CODEX_PROXY_MAGIC_REJECTED"));
        };
        let length_bytes: [u8; 4] = encoded
            .get(..4)
            .and_then(|value| value.try_into().ok())
            .ok_or_else(|| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?;
        let body_length = usize::try_from(u32::from_be_bytes(length_bytes))
            .map_err(|_| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?;
        let body = encoded
            .get(4..)
            .filter(|body| body.len() == body_length)
            .ok_or_else(|| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?;
        if !(CODEX_PROXY_HEADER_BYTES..=MAX_CODEX_PROXY_BODY_BYTES).contains(&body.len()) {
            return Err(malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"));
        }
        let kind = body[0];
        let stream_id = u32::from_be_bytes(
            body[1..5]
                .try_into()
                .map_err(|_| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?,
        );
        let sequence = u32::from_be_bytes(
            body[5..9]
                .try_into()
                .map_err(|_| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?,
        );
        if stream_id != CODEX_PROXY_STREAM_ID {
            return Err(malformed("HERMES_CODEX_PROXY_STREAM_REJECTED"));
        }
        if !constant_time_equal(&body[9..41], &self.binding) {
            return Err(cross_binding("HERMES_CODEX_PROXY_BINDING_REJECTED"));
        }
        if sequence != self.expected_sequence {
            return Err(cross_binding("HERMES_CODEX_PROXY_SEQUENCE_REJECTED"));
        }
        let payload = &body[CODEX_PROXY_HEADER_BYTES..];
        let event = match kind {
            1 if payload.is_empty()
                && self.inbound == CodexProxyInboundState::AwaitOpen
                && self.outbound == CodexProxyOutboundState::AwaitOpenAck =>
            {
                self.inbound = CodexProxyInboundState::Open;
                CodexProxyHostEvent::Open
            }
            2 if !payload.is_empty()
                && payload.len() <= MAX_CODEX_PROXY_DATA_BYTES
                && self.inbound == CodexProxyInboundState::Open =>
            {
                CodexProxyHostEvent::Data(payload.to_vec())
            }
            3 if payload.is_empty() && self.inbound == CodexProxyInboundState::Open => {
                self.inbound = CodexProxyInboundState::Closed;
                CodexProxyHostEvent::Close
            }
            4 if payload.len() == 2 && self.inbound == CodexProxyInboundState::Open => {
                self.inbound = CodexProxyInboundState::Closed;
                CodexProxyHostEvent::Error(u16::from_be_bytes([payload[0], payload[1]]))
            }
            1..=4 => return Err(malformed("HERMES_CODEX_PROXY_STATE_REJECTED")),
            _ => return Err(malformed("HERMES_CODEX_PROXY_KIND_REJECTED")),
        };
        self.expected_sequence = self
            .expected_sequence
            .checked_add(1)
            .ok_or_else(|| malformed("HERMES_CODEX_PROXY_SEQUENCE_REJECTED"))?;
        Ok(event)
    }

    fn encode_outbound(&mut self, kind: u8, payload: &[u8]) -> HermesAdapterResult<Vec<u8>> {
        if Instant::now() >= self.deadline {
            return Err(error(
                HermesAdapterErrorKind::Timeout,
                "HERMES_CODEX_PROXY_DEADLINE_EXCEEDED",
            ));
        }
        let frame = encode_codex_proxy_frame(kind, self.outbound_sequence, self.binding, payload)?;
        self.outbound_sequence = self
            .outbound_sequence
            .checked_add(1)
            .ok_or_else(|| malformed("HERMES_CODEX_PROXY_SEQUENCE_REJECTED"))?;
        Ok(frame)
    }
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
pub(crate) fn encode_codex_proxy_test_frame(
    kind: u8,
    sequence: u32,
    binding: [u8; 32],
    payload: &[u8],
) -> Vec<u8> {
    encode_codex_proxy_frame(kind, sequence, binding, payload).unwrap_or_default()
}

fn encode_codex_proxy_frame(
    kind: u8,
    sequence: u32,
    binding: [u8; 32],
    payload: &[u8],
) -> HermesAdapterResult<Vec<u8>> {
    let body_length = CODEX_PROXY_HEADER_BYTES
        .checked_add(payload.len())
        .filter(|length| *length <= MAX_CODEX_PROXY_BODY_BYTES)
        .ok_or_else(|| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?;
    let mut frame = Vec::with_capacity(CODEX_PROXY_MAGIC.len() + 4 + body_length);
    frame.extend_from_slice(CODEX_PROXY_MAGIC);
    frame.extend_from_slice(
        &u32::try_from(body_length)
            .map_err(|_| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?
            .to_be_bytes(),
    );
    frame.push(kind);
    frame.extend_from_slice(&CODEX_PROXY_STREAM_ID.to_be_bytes());
    frame.extend_from_slice(&sequence.to_be_bytes());
    frame.extend_from_slice(&binding);
    frame.extend_from_slice(payload);
    Ok(frame)
}

enum OuterStreamEvent {
    Data(Vec<u8>),
    Eof,
    Failed,
}

enum ProviderStreamEvent {
    Data(Vec<u8>),
    Eof,
    Failed,
}

#[derive(Default)]
struct CodexProxyHostStatus {
    failure: Option<HermesAdapterError>,
    failure_evidence: Option<CodexProxyFailureEvidence>,
    authenticated_open: bool,
    clean_terminal: bool,
}

struct ProductionCodexProxyHost {
    status: Arc<Mutex<CodexProxyHostStatus>>,
    stop: Arc<AtomicBool>,
    control: Arc<dyn ProductionCodexProxyControl>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ProductionCodexProxyHost {
    #[allow(clippy::too_many_arguments)]
    fn start(
        provider: Box<dyn ProductionCodexProxyProvider>,
        nonce: &str,
        broker_receipt: &ContentDigest,
        absolute_deadline: Instant,
        outer_input: std::fs::File,
        outer_stream: Receiver<OuterStreamEvent>,
        initial_bytes: Vec<u8>,
        owner: Arc<ContainmentOwnerState>,
    ) -> HermesAdapterResult<Self> {
        let mut session = CodexProxyHostSession::new(nonce, broker_receipt, absolute_deadline)?;
        let control = provider.control();
        let status = Arc::new(Mutex::new(CodexProxyHostStatus::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_status = Arc::clone(&status);
        let worker_stop = Arc::clone(&stop);
        let worker_control = Arc::clone(&control);
        let worker = thread::Builder::new()
            .name("lattice-hermes-codex-proxy".to_owned())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_codex_proxy_host(
                        provider,
                        absolute_deadline,
                        outer_input,
                        &outer_stream,
                        initial_bytes,
                        &worker_stop,
                        &mut session,
                        &worker_control,
                        &worker_status,
                    )
                }))
                .unwrap_or_else(|_| {
                    Err(error(
                        HermesAdapterErrorKind::Ambiguous,
                        "HERMES_CODEX_PROXY_HOST_PANICKED",
                    ))
                });
                if let Err(failure) = result {
                    owner.invalidate();
                    let failure = worker_control.terminate().err().unwrap_or(failure);
                    if let Ok(mut observed) = worker_status.lock() {
                        observed.failure = Some(failure);
                        observed.failure_evidence = session.failure_evidence().cloned();
                    }
                }
            })
            .map_err(|_| {
                error(
                    HermesAdapterErrorKind::Spawn,
                    "HERMES_CODEX_PROXY_HOST_THREAD_FAILED",
                )
            })?;
        Ok(Self {
            status,
            stop,
            control,
            worker: Some(worker),
        })
    }

    fn ensure_live(&self) -> HermesAdapterResult<()> {
        let observed = self.status.lock().map_err(|_| {
            error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_HOST_STATE_UNKNOWN",
            )
        })?;
        if let Some(failure) = observed.failure.clone() {
            return Err(failure);
        }
        if self
            .worker
            .as_ref()
            .is_some_and(thread::JoinHandle::is_finished)
        {
            return Err(error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_HOST_EXITED",
            ));
        }
        Ok(())
    }

    fn failure_evidence(&self) -> Option<CodexProxyFailureEvidence> {
        self.status
            .lock()
            .ok()
            .and_then(|observed| observed.failure_evidence.clone())
    }

    fn wait_for_clean_terminal(&self, deadline: Instant) -> HermesAdapterResult<()> {
        loop {
            {
                let observed = self.status.lock().map_err(|_| {
                    error(
                        HermesAdapterErrorKind::Ambiguous,
                        "HERMES_CODEX_PROXY_HOST_STATE_UNKNOWN",
                    )
                })?;
                if let Some(failure) = observed.failure.clone() {
                    return Err(failure);
                }
                if observed.authenticated_open && observed.clean_terminal {
                    return Ok(());
                }
            }
            if self
                .worker
                .as_ref()
                .is_some_and(thread::JoinHandle::is_finished)
            {
                return Err(error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_CODEX_PROXY_HOST_EXITED",
                ));
            }
            if Instant::now() >= deadline {
                return Err(error(
                    HermesAdapterErrorKind::Timeout,
                    "HERMES_CODEX_PROXY_COMPLETION_TIMEOUT",
                ));
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn stop_and_reap(&mut self) -> HermesAdapterResult<()> {
        self.stop.store(true, Ordering::Release);
        let control_result = self.control.terminate();
        let join_result = self.join_worker_bounded();
        control_result.and(join_result)
    }

    fn terminate(&mut self) -> HermesAdapterResult<()> {
        self.stop_and_reap()?;
        let observed = self.status.lock().map_err(|_| {
            error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_HOST_STATE_UNKNOWN",
            )
        })?;
        observed.failure.clone().map_or(Ok(()), Err)
    }

    fn join_worker_bounded(&mut self) -> HermesAdapterResult<()> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        let deadline = Instant::now()
            .checked_add(CODEX_PROXY_TEARDOWN_TIMEOUT)
            .ok_or_else(|| {
                error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_CODEX_PROXY_HOST_TEARDOWN_AMBIGUOUS",
                )
            })?;
        while !worker.is_finished() {
            if Instant::now() >= deadline {
                drop(worker);
                return Err(error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_CODEX_PROXY_HOST_TEARDOWN_AMBIGUOUS",
                ));
            }
            thread::sleep(Duration::from_millis(1));
        }
        worker.join().map_err(|_| {
            error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_HOST_JOIN_FAILED",
            )
        })
    }
}

impl Drop for ProductionCodexProxyHost {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn run_codex_proxy_host(
    provider: Box<dyn ProductionCodexProxyProvider>,
    absolute_deadline: Instant,
    mut outer_input: std::fs::File,
    outer_stream: &Receiver<OuterStreamEvent>,
    initial_bytes: Vec<u8>,
    stop: &AtomicBool,
    session: &mut CodexProxyHostSession,
    control: &Arc<dyn ProductionCodexProxyControl>,
    status: &Arc<Mutex<CodexProxyHostStatus>>,
) -> HermesAdapterResult<()> {
    let mut provider = Some(provider);
    let mut duplex: Option<ProductionCodexProxyDuplex> = None;
    let mut provider_stream: Option<Receiver<ProviderStreamEvent>> = None;
    let mut buffer = initial_bytes;

    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        if Instant::now() >= absolute_deadline {
            return Err(error(
                HermesAdapterErrorKind::Timeout,
                "HERMES_CODEX_PROXY_DEADLINE_EXCEEDED",
            ));
        }

        while let Some(frame) = take_codex_proxy_frame(&mut buffer, session)? {
            match session.accept(&frame)? {
                CodexProxyHostEvent::Open => {
                    let sealed = provider
                        .take()
                        .ok_or_else(|| malformed("HERMES_CODEX_PROXY_PROVIDER_REPLAY_REJECTED"))?;
                    let mut opened = sealed.open(absolute_deadline)?;
                    control.ensure_running()?;
                    let reader = opened.take_reader()?;
                    provider_stream = Some(start_provider_reader(reader)?);
                    write_proxy_frame(&mut outer_input, &session.encode_open_ack()?)?;
                    status
                        .lock()
                        .map_err(|_| {
                            error(
                                HermesAdapterErrorKind::Ambiguous,
                                "HERMES_CODEX_PROXY_HOST_STATE_UNKNOWN",
                            )
                        })?
                        .authenticated_open = true;
                    duplex = Some(opened);
                }
                CodexProxyHostEvent::Data(payload) => {
                    let opened = duplex
                        .as_mut()
                        .ok_or_else(|| malformed("HERMES_CODEX_PROXY_PROVIDER_STATE_REJECTED"))?;
                    control.ensure_running()?;
                    opened.write_all(&payload)?;
                }
                CodexProxyHostEvent::Close => {
                    let opened = duplex
                        .as_mut()
                        .ok_or_else(|| malformed("HERMES_CODEX_PROXY_PROVIDER_STATE_REJECTED"))?;
                    opened.close_input();
                }
                CodexProxyHostEvent::Error(_) => {
                    return Err(error(
                        HermesAdapterErrorKind::Failed,
                        "HERMES_CODEX_PROXY_CHILD_ERROR",
                    ));
                }
            }
        }

        let provider_event = provider_stream
            .as_ref()
            .and_then(|events| match events.try_recv() {
                Ok(event) => Some(Ok(event)),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_CODEX_PROXY_PROVIDER_STREAM_LOST",
                ))),
            });
        if let Some(provider_event) = provider_event {
            match provider_event? {
                ProviderStreamEvent::Data(payload) => {
                    write_proxy_frame(&mut outer_input, &session.encode_data(&payload)?)?;
                }
                ProviderStreamEvent::Eof => {
                    if !session.inbound_closed() {
                        control.ensure_running()?;
                        return Err(error(
                            HermesAdapterErrorKind::Failed,
                            "HERMES_CODEX_PROXY_PROVIDER_EOF_BEFORE_CLOSE",
                        ));
                    }
                    write_proxy_frame(&mut outer_input, &session.encode_close()?)?;
                    provider_stream = None;
                    debug_assert!(session.outbound_closed());
                    status
                        .lock()
                        .map_err(|_| {
                            error(
                                HermesAdapterErrorKind::Ambiguous,
                                "HERMES_CODEX_PROXY_HOST_STATE_UNKNOWN",
                            )
                        })?
                        .clean_terminal = true;
                }
                ProviderStreamEvent::Failed => {
                    return Err(error(
                        HermesAdapterErrorKind::Transport,
                        "HERMES_CODEX_PROXY_PROVIDER_READ_FAILED",
                    ));
                }
            }
        }

        match outer_stream.recv_timeout(Duration::from_millis(2)) {
            Ok(OuterStreamEvent::Data(payload)) => {
                buffer.extend_from_slice(&payload);
                if buffer.len() > MAX_CODEX_PROXY_BUFFER_BYTES {
                    session.record_failure(&buffer);
                    return Err(malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"));
                }
            }
            Ok(OuterStreamEvent::Eof) => {
                return Err(error(
                    HermesAdapterErrorKind::Failed,
                    "HERMES_CODEX_PROXY_OUTER_EOF",
                ));
            }
            Ok(OuterStreamEvent::Failed) => {
                return Err(error(
                    HermesAdapterErrorKind::Transport,
                    "HERMES_CODEX_PROXY_OUTER_READ_FAILED",
                ));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_CODEX_PROXY_OUTER_STREAM_LOST",
                ));
            }
        }
    }
}

fn take_codex_proxy_frame(
    buffer: &mut Vec<u8>,
    session: &mut CodexProxyHostSession,
) -> HermesAdapterResult<Option<Vec<u8>>> {
    let comparable = buffer.len().min(CODEX_PROXY_MAGIC.len());
    if buffer[..comparable] != CODEX_PROXY_MAGIC[..comparable] {
        session.record_failure(buffer);
        return Err(malformed("HERMES_CODEX_PROXY_MAGIC_REJECTED"));
    }
    let prefix_bytes = CODEX_PROXY_MAGIC.len() + 4;
    if buffer.len() < prefix_bytes {
        return Ok(None);
    }
    let body_length = usize::try_from(u32::from_be_bytes(
        buffer[CODEX_PROXY_MAGIC.len()..prefix_bytes]
            .try_into()
            .expect("fixed four-byte proxy length"),
    ))
    .map_err(|_| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?;
    if !(CODEX_PROXY_HEADER_BYTES..=MAX_CODEX_PROXY_BODY_BYTES).contains(&body_length) {
        session.record_failure(&buffer[..prefix_bytes]);
        return Err(malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"));
    }
    let total = prefix_bytes
        .checked_add(body_length)
        .ok_or_else(|| malformed("HERMES_CODEX_PROXY_SIZE_REJECTED"))?;
    if buffer.len() < total {
        return Ok(None);
    }
    Ok(Some(buffer.drain(..total).collect()))
}

fn write_proxy_frame(writer: &mut std::fs::File, frame: &[u8]) -> HermesAdapterResult<()> {
    writer
        .write_all(frame)
        .and_then(|()| writer.flush())
        .map_err(|_| {
            error(
                HermesAdapterErrorKind::Transport,
                "HERMES_CODEX_PROXY_OUTER_WRITE_FAILED",
            )
        })
}

fn start_outer_reader(mut reader: std::fs::File) -> Receiver<OuterStreamEvent> {
    let (sender, receiver) = mpsc::sync_channel(8);
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    let _ = sender.send(OuterStreamEvent::Eof);
                    return;
                }
                Ok(read) => {
                    if sender
                        .send(OuterStreamEvent::Data(buffer[..read].to_vec()))
                        .is_err()
                    {
                        return;
                    }
                }
                Err(_) => {
                    let _ = sender.send(OuterStreamEvent::Failed);
                    return;
                }
            }
        }
    });
    receiver
}

fn start_provider_reader(
    mut reader: Box<dyn Read + Send>,
) -> HermesAdapterResult<Receiver<ProviderStreamEvent>> {
    let (sender, receiver) = mpsc::sync_channel(8);
    thread::Builder::new()
        .name("lattice-hermes-codex-output".to_owned())
        .spawn(move || {
            let mut buffer = vec![0_u8; MAX_CODEX_PROXY_DATA_BYTES].into_boxed_slice();
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        let _ = sender.send(ProviderStreamEvent::Eof);
                        return;
                    }
                    Ok(read) => {
                        if sender
                            .send(ProviderStreamEvent::Data(buffer[..read].to_vec()))
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(_) => {
                        let _ = sender.send(ProviderStreamEvent::Failed);
                        return;
                    }
                }
            }
        })
        .map_err(|_| {
            error(
                HermesAdapterErrorKind::Spawn,
                "HERMES_CODEX_PROXY_READER_THREAD_FAILED",
            )
        })?;
    Ok(receiver)
}

#[cfg(test)]
#[derive(Default)]
struct ScriptedCodexProxyProvider {
    control: Arc<ScriptedCodexProxyControl>,
}

#[cfg(test)]
impl ProductionCodexProxyProvider for ScriptedCodexProxyProvider {
    fn control(&self) -> Arc<dyn ProductionCodexProxyControl> {
        self.control.clone()
    }

    fn open(
        self: Box<Self>,
        absolute_deadline: Instant,
    ) -> HermesAdapterResult<ProductionCodexProxyDuplex> {
        if absolute_deadline <= Instant::now() {
            return Err(error(
                HermesAdapterErrorKind::Timeout,
                "HERMES_CODEX_PROXY_DEADLINE_EXCEEDED",
            ));
        }
        let response = concat!(
            "{\"id\":0,\"result\":{\"ok\":true}}\n",
            "{\"id\":1,\"result\":{\"ok\":true}}\n",
            "{\"id\":2,\"result\":{\"ok\":true}}\n"
        )
        .as_bytes()
        .to_vec();
        Ok(ProductionCodexProxyDuplex::new(
            Box::new(std::io::Cursor::new(response)),
            Box::new(Vec::<u8>::new()),
        ))
    }
}

#[cfg(test)]
#[derive(Default)]
struct ScriptedCodexProxyControl;

#[cfg(test)]
impl ProductionCodexProxyControl for ScriptedCodexProxyControl {
    fn ensure_running(&self) -> HermesAdapterResult<()> {
        Ok(())
    }

    fn terminate(&self) -> HermesAdapterResult<()> {
        Ok(())
    }
}

#[derive(Clone)]
enum RunnerMode {
    Official,
    #[cfg(test)]
    ScriptedFixture(String),
}

impl RunnerMode {
    const fn as_str(&self) -> &'static str {
        match self {
            Self::Official => "official",
            #[cfg(test)]
            Self::ScriptedFixture(_) => "scripted_fixture",
        }
    }

    fn fixture_reflection(&self) -> Option<&str> {
        match self {
            Self::Official => None,
            #[cfg(test)]
            Self::ScriptedFixture(reflection) => Some(reflection),
        }
    }
}

/// Validated inputs for the only production Hermes construction chain.
///
/// This value cannot install a receipt into an arbitrary adapter. [`Self::launch`]
/// owns WSL, bubblewrap, the namespace PID, endpoint, and adapter together.
pub struct HermesProductionRunnerConfig {
    containment: HermesWslContainmentConfig,
    expected_request: HermesResearchRequest,
    runtime_manifest_sha256: String,
    broker_receipt_sha256: String,
    api_key: String,
    model: String,
    startup_timeout: Duration,
    operation_timeout: Duration,
    poll_interval: Duration,
    mode: RunnerMode,
    codex_provider: Option<Box<dyn ProductionCodexProxyProvider>>,
}

impl HermesProductionRunnerConfig {
    /// Creates an exact official-runtime runner configuration.
    ///
    /// # Errors
    ///
    /// Rejects an invalid broker receipt, runtime manifest serialization,
    /// bearer/model value, or relative timeout before any process is started.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        containment: HermesWslContainmentConfig,
        runtime_manifest: &HermesOfflineRuntimeManifest,
        broker_receipt: &CodexBrokerReceipt,
        expected_request: HermesResearchRequest,
        api_key: impl Into<String>,
        model: impl Into<String>,
        startup_timeout: Duration,
        operation_timeout: Duration,
        poll_interval: Duration,
    ) -> HermesAdapterResult<Self> {
        broker_receipt.validate_for_containment()?;
        Self::validated(
            containment,
            runtime_manifest,
            broker_receipt.receipt_digest().as_str().to_owned(),
            expected_request,
            api_key.into(),
            model.into(),
            startup_timeout,
            operation_timeout,
            poll_interval,
            RunnerMode::Official,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn scripted_fixture(
        containment: HermesWslContainmentConfig,
        runtime_manifest: &HermesOfflineRuntimeManifest,
        broker_receipt_digest: &ContentDigest,
        expected_request: HermesResearchRequest,
        api_key: impl Into<String>,
        model: impl Into<String>,
        startup_timeout: Duration,
        operation_timeout: Duration,
        poll_interval: Duration,
        reflection: impl Into<String>,
    ) -> HermesAdapterResult<Self> {
        let reflection = reflection.into();
        if reflection.is_empty() || reflection.len() > 64 * 1024 {
            return Err(malformed("HERMES_PRODUCTION_FIXTURE_REJECTED"));
        }
        serde_json::from_str::<serde_json::Value>(&reflection)
            .map_err(|_| malformed("HERMES_PRODUCTION_FIXTURE_REJECTED"))?;
        Self::validated(
            containment,
            runtime_manifest,
            broker_receipt_digest.as_str().to_owned(),
            expected_request,
            api_key.into(),
            model.into(),
            startup_timeout,
            operation_timeout,
            poll_interval,
            RunnerMode::ScriptedFixture(reflection),
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn official_with_broker_digest(
        containment: HermesWslContainmentConfig,
        runtime_manifest: &HermesOfflineRuntimeManifest,
        broker_receipt_digest: &ContentDigest,
        expected_request: HermesResearchRequest,
        api_key: impl Into<String>,
        model: impl Into<String>,
        startup_timeout: Duration,
        operation_timeout: Duration,
        poll_interval: Duration,
    ) -> HermesAdapterResult<Self> {
        Self::validated(
            containment,
            runtime_manifest,
            broker_receipt_digest.as_str().to_owned(),
            expected_request,
            api_key.into(),
            model.into(),
            startup_timeout,
            operation_timeout,
            poll_interval,
            RunnerMode::Official,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn validated(
        containment: HermesWslContainmentConfig,
        runtime_manifest: &HermesOfflineRuntimeManifest,
        broker_receipt_sha256: String,
        expected_request: HermesResearchRequest,
        api_key: String,
        model: String,
        startup_timeout: Duration,
        operation_timeout: Duration,
        poll_interval: Duration,
        mode: RunnerMode,
    ) -> HermesAdapterResult<Self> {
        if startup_timeout.is_zero() || startup_timeout > MAX_RUNNER_TIMEOUT {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_PRODUCTION_STARTUP_TIMEOUT_REJECTED",
            ));
        }
        HermesAdapterConfig::new(
            "127.0.0.1:1".parse().expect("fixed loopback endpoint"),
            api_key.clone(),
            operation_timeout,
            poll_interval,
        )?;
        let manifest_bytes = serde_json::to_vec(runtime_manifest)
            .map_err(|_| malformed("HERMES_RUNTIME_MANIFEST_CANONICALIZATION_FAILED"))?;
        let runtime_manifest_sha256 = encode_sha256(&Sha256::digest(&manifest_bytes));
        #[cfg(test)]
        let codex_provider: Option<Box<dyn ProductionCodexProxyProvider>> = match &mode {
            RunnerMode::Official => None,
            RunnerMode::ScriptedFixture(_) => Some(Box::new(ScriptedCodexProxyProvider::default())),
        };
        #[cfg(not(test))]
        let codex_provider = None;
        Ok(Self {
            containment,
            expected_request,
            runtime_manifest_sha256,
            broker_receipt_sha256,
            api_key,
            model,
            startup_timeout,
            operation_timeout,
            poll_interval,
            mode,
            codex_provider,
        })
    }

    /// Starts the pinned WSL/bubblewrap child and privately installs the
    /// resulting receipt into the adapter owned by the returned port.
    ///
    /// # Errors
    ///
    /// Fails closed on deadline, path, launcher, runtime, broker, socketpair,
    /// endpoint, PID, containment-frame, child-liveness, or startup ambiguity.
    pub fn launch(self, absolute_deadline: Instant) -> HermesAdapterResult<ProductionHermesRunner> {
        self.launch_inner(absolute_deadline)
    }

    #[allow(clippy::too_many_lines)]
    fn launch_inner(
        self,
        absolute_deadline: Instant,
    ) -> HermesAdapterResult<ProductionHermesRunner> {
        if absolute_deadline <= Instant::now() {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_PRODUCTION_LAUNCH_BINDING_REJECTED",
            ));
        }
        if self.containment.isolation_root().exists()
            || self
                .containment
                .isolation_root()
                .starts_with(self.containment.product_root())
            || self
                .containment
                .product_root()
                .starts_with(self.containment.isolation_root())
        {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_PRODUCTION_ROOT_REJECTED",
            ));
        }
        fs::create_dir(self.containment.isolation_root()).map_err(|_| {
            error(
                HermesAdapterErrorKind::Spawn,
                "HERMES_PRODUCTION_ROOT_CREATE_FAILED",
            )
        })?;
        let capture_root = self.containment.isolation_root().join("capture");
        fs::create_dir(&capture_root).map_err(|_| {
            error(
                HermesAdapterErrorKind::Spawn,
                "HERMES_PRODUCTION_CAPTURE_CREATE_FAILED",
            )
        })?;
        let nonce = production_nonce(self.containment.isolation_root())?;
        let remaining = absolute_deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                error(
                    HermesAdapterErrorKind::Timeout,
                    "HERMES_PRODUCTION_DEADLINE_EXCEEDED",
                )
            })?;
        let deadline_millis = u64::try_from(remaining.as_millis())
            .unwrap_or(u64::MAX)
            .min(300_000);
        if deadline_millis == 0 {
            return Err(error(
                HermesAdapterErrorKind::Timeout,
                "HERMES_PRODUCTION_DEADLINE_EXCEEDED",
            ));
        }
        let secret_path = self.containment.isolation_root().join("launch-secret.json");
        let runner_path = self.containment.isolation_root().join("inner-runner.py");
        let secret = LaunchSecret {
            api_key: &self.api_key,
            broker_receipt_sha256: &self.broker_receipt_sha256,
            config_sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            deadline_millis,
            endpoint: "127.0.0.1:0",
            fixture_reflection: self.mode.fixture_reflection(),
            mode: self.mode.as_str(),
            model: &self.model,
            nonce: &nonce,
            runtime_manifest_sha256: &self.runtime_manifest_sha256,
        };
        let secret_bytes = serde_json::to_vec(&secret)
            .map_err(|_| malformed("HERMES_PRODUCTION_SECRET_REJECTED"))?;
        write_new_secret(&secret_path, &secret_bytes)?;
        if let Err(failure) = write_new_runner(&runner_path, PRIVATE_RUNNER_SOURCE.as_bytes()) {
            remove_ingress(&secret_path);
            return Err(failure);
        }
        let secret_guest_path = windows_path_to_wsl(&secret_path)?;
        let runner_guest_path = windows_path_to_wsl(&runner_path)?;
        let runner_sha256 = sha256_text(PRIVATE_RUNNER_SOURCE);
        let interpreter = format!(
            "{}/python/bin/python3.12",
            self.containment.runtime_guest_root()
        );
        let arguments = [
            OsString::from("-d"),
            OsString::from(WSL_DISTRO),
            OsString::from("--exec"),
            OsString::from(interpreter),
            OsString::from("-I"),
            OsString::from("-S"),
            OsString::from("-B"),
            OsString::from("-c"),
            OsString::from(OUTER_RUNNER_SOURCE),
            OsString::from("production"),
            OsString::from(self.containment.runtime_guest_root()),
            OsString::from(&nonce),
            OsString::from(secret_guest_path),
            OsString::from(runner_guest_path),
            OsString::from(runner_sha256),
        ]
        .into_iter()
        .collect::<Vec<_>>();
        let plan = crate::windows_job::WindowsJobCommandPlan {
            executable: self.containment.wsl_executable().to_path_buf(),
            arguments,
            current_dir: self.containment.isolation_root().to_path_buf(),
            environment: minimal_wsl_environment(self.containment.wsl_executable())?,
            run_root: self.containment.isolation_root().to_path_buf(),
            stdout_path: capture_root.join("production.stdout"),
            stderr_path: capture_root.join("production.stderr"),
            stdout_limit: MAX_STARTUP_BYTES as u64,
            stderr_limit: 4096,
            deadline: absolute_deadline,
            teardown_timeout: Duration::from_secs(3),
        };
        let mut process = match crate::windows_job::spawn_with_parent_stdio(&plan) {
            Ok(process) => process,
            Err(failure) => {
                remove_ingress(&secret_path);
                remove_ingress(&runner_path);
                return Err(failure);
            }
        };
        let outer_input = match process.take_stdin_writer() {
            Ok(writer) => writer,
            Err(failure) => {
                let _ = process.terminate();
                remove_ingress(&secret_path);
                remove_ingress(&runner_path);
                return Err(failure);
            }
        };
        let outer_output = match process.take_stdout_reader() {
            Ok(reader) => reader,
            Err(failure) => {
                let _ = process.terminate();
                remove_ingress(&secret_path);
                remove_ingress(&runner_path);
                return Err(failure);
            }
        };
        let outer_stream = start_outer_reader(outer_output);
        let startup_deadline = Instant::now()
            .checked_add(self.startup_timeout)
            .map_or(absolute_deadline, |candidate| {
                candidate.min(absolute_deadline)
            });
        let mut startup = match wait_for_startup(&mut process, &outer_stream, startup_deadline) {
            Ok(startup) => startup,
            Err(failure) => {
                let mapped = map_outer_failure(&process, failure);
                let _ = process.terminate();
                remove_ingress(&secret_path);
                remove_ingress(&runner_path);
                return Err(mapped);
            }
        };
        if secret_path.exists() || runner_path.exists() {
            let _ = process.terminate();
            remove_ingress(&secret_path);
            remove_ingress(&runner_path);
            return Err(error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_PRODUCTION_INGRESS_NOT_CONSUMED",
            ));
        }
        let attestation = verify_startup(
            &startup,
            &self.runtime_manifest_sha256,
            &self.broker_receipt_sha256,
            &self.api_key,
            &self.model,
            &nonce,
            self.mode.as_str(),
        )?;
        process.ensure_running()?;
        let runner_nonce_sha256 = attestation.runner_nonce_sha256.clone();
        let owner = Arc::new(ContainmentOwnerState::new(runner_nonce_sha256.clone()));
        let receipt = mint_receipt(
            &attestation,
            process.process_id(),
            runner_nonce_sha256,
            Arc::downgrade(&owner),
        )?;
        let outer_initial_bytes = std::mem::take(&mut startup.trailing);
        Ok(ProductionHermesRunner {
            endpoint: attestation.endpoint,
            api_key: self.api_key,
            model: self.model,
            expected_request: self.expected_request,
            nonce,
            broker_receipt_sha256: self.broker_receipt_sha256,
            codex_provider: self.codex_provider,
            outer_input: Some(outer_input),
            outer_stream: Some(outer_stream),
            outer_initial_bytes,
            receipt,
            process,
            owner,
            absolute_deadline,
            operation_timeout: self.operation_timeout,
            poll_interval: self.poll_interval,
            windows_launcher_pid: startup.windows_launcher_pid,
            outer_pid: startup.wire.outer_pid,
            bwrap_pid: startup.wire.bwrap_pid,
        })
    }
}

/// Live contained Hermes process that exists before any Codex effect and can
/// be bound exactly once to the resulting immutable reflection job.
pub struct ProductionHermesRunner {
    endpoint: SocketAddr,
    api_key: String,
    model: String,
    expected_request: HermesResearchRequest,
    nonce: String,
    broker_receipt_sha256: String,
    codex_provider: Option<Box<dyn ProductionCodexProxyProvider>>,
    outer_input: Option<std::fs::File>,
    outer_stream: Option<Receiver<OuterStreamEvent>>,
    outer_initial_bytes: Vec<u8>,
    receipt: HermesContainmentReceipt,
    process: crate::windows_job::WindowsJobChild,
    owner: Arc<ContainmentOwnerState>,
    absolute_deadline: Instant,
    operation_timeout: Duration,
    poll_interval: Duration,
    windows_launcher_pid: u32,
    outer_pid: u32,
    bwrap_pid: u32,
}

impl ProductionHermesRunner {
    /// Consumes the sole runner owner and binds it once to the completed job.
    ///
    /// # Errors
    ///
    /// Rejects a mismatched model, expired/dead child, or receipt binding before
    /// adapter construction. Any error drops and reaps the owned process tree.
    pub fn bind(mut self, job: HermesReflectionJob) -> HermesAdapterResult<ProductionHermesPort> {
        if job.model() != self.model || job.request() != &self.expected_request {
            return Err(error(
                HermesAdapterErrorKind::CrossBinding,
                "HERMES_PRODUCTION_JOB_BINDING_REJECTED",
            ));
        }
        self.process.ensure_running()?;
        self.receipt.verify_binding(self.endpoint, &self.api_key)?;
        let broker_receipt = ContentDigest::from_sha256(self.broker_receipt_sha256.clone())
            .map_err(|_| malformed("HERMES_CODEX_PROXY_BINDING_INPUT_REJECTED"))?;
        let provider = self.codex_provider.take().ok_or_else(|| {
            error(
                HermesAdapterErrorKind::CapabilityMismatch,
                "HERMES_CODEX_PROXY_PROVIDER_NOT_STAGED",
            )
        })?;
        let outer_input = self.outer_input.take().ok_or_else(|| {
            error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_OUTER_INPUT_UNAVAILABLE",
            )
        })?;
        let outer_stream = self.outer_stream.take().ok_or_else(|| {
            error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_OUTER_STREAM_UNAVAILABLE",
            )
        })?;
        let codex_proxy = ProductionCodexProxyHost::start(
            provider,
            &self.nonce,
            &broker_receipt,
            self.absolute_deadline,
            outer_input,
            outer_stream,
            std::mem::take(&mut self.outer_initial_bytes),
            Arc::clone(&self.owner),
        )?;
        let remaining = self
            .absolute_deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                error(
                    HermesAdapterErrorKind::Timeout,
                    "HERMES_PRODUCTION_DEADLINE_EXCEEDED",
                )
            })?;
        let timeout = self.operation_timeout.min(remaining);
        let mut adapter_config = HermesAdapterConfig::new(
            self.endpoint,
            self.api_key,
            timeout,
            self.poll_interval.min(timeout),
        )?;
        adapter_config.install_containment_receipt(self.receipt.clone())?;
        let adapter = HermesReflectionAdapter::connect(adapter_config, job)?;
        Ok(ProductionHermesPort {
            adapter,
            codex_proxy,
            receipt: self.receipt,
            process: self.process,
            owner: self.owner,
            absolute_deadline: self.absolute_deadline,
            operation_timeout: self.operation_timeout,
            poll_interval: self.poll_interval,
            windows_launcher_pid: self.windows_launcher_pid,
            outer_pid: self.outer_pid,
            bwrap_pid: self.bwrap_pid,
        })
    }

    /// Returns the sealed attestation needed by the full-chain orchestrator.
    #[must_use]
    pub const fn containment_receipt(&self) -> &HermesContainmentReceipt {
        &self.receipt
    }

    /// Proves that the same owned Job process tree remains alive.
    ///
    /// # Errors
    ///
    /// Fails closed on deadline, process exit, Job ambiguity, or receipt replay.
    pub fn verify_live(&mut self) -> HermesAdapterResult<()> {
        self.process.ensure_running()?;
        self.receipt.verify_binding(self.endpoint, &self.api_key)
    }

    /// Explicitly invalidates the receipt and reaps the owned WSL tree.
    ///
    /// # Errors
    ///
    /// Reports teardown ambiguity if the Job cannot prove all descendants exit.
    pub fn terminate(mut self) -> HermesAdapterResult<()> {
        self.owner.invalidate();
        self.process.terminate()
    }

    #[must_use]
    pub const fn windows_launcher_pid(&self) -> u32 {
        self.windows_launcher_pid
    }

    #[must_use]
    pub const fn outer_pid(&self) -> u32 {
        self.outer_pid
    }

    #[must_use]
    pub const fn bwrap_pid(&self) -> u32 {
        self.bwrap_pid
    }
}

/// Production-only Hermes port whose adapter and contained child share one
/// unforgeable owner capability.
pub struct ProductionHermesPort {
    adapter: HermesReflectionAdapter,
    codex_proxy: ProductionCodexProxyHost,
    receipt: HermesContainmentReceipt,
    process: crate::windows_job::WindowsJobChild,
    owner: Arc<ContainmentOwnerState>,
    absolute_deadline: Instant,
    operation_timeout: Duration,
    poll_interval: Duration,
    windows_launcher_pid: u32,
    outer_pid: u32,
    bwrap_pid: u32,
}

impl ProductionHermesPort {
    /// Runs one reflection while the exact contained namespace process remains
    /// alive and returns canonical payload plus normalized evidence.
    ///
    /// # Errors
    ///
    /// Fails before endpoint I/O on child death, deadline, binding, or replay,
    /// and discards any result if the owned child dies before return.
    pub fn run_reflection_evidence(
        &mut self,
        request: &HermesResearchRequest,
    ) -> PortResult<HermesReflectionEvidence> {
        self.prepare_operation()?;
        let result = self.adapter.run_reflection_evidence(request);
        if result.is_ok() {
            self.require_clean_proxy_terminal()?;
        }
        self.ensure_live()?;
        result
    }

    /// Reconciles one already-submitted run without changing the owner or
    /// recomputing normalized evidence.
    ///
    /// # Errors
    ///
    /// Preserves owner, deadline, recovery, and adapter failures.
    pub fn reconcile_reflection(
        &mut self,
        request: &HermesResearchRequest,
        receipt: &crate::HermesRunRecoveryReceipt,
    ) -> PortResult<CanonicalReflection> {
        self.prepare_operation()?;
        let result = self
            .adapter
            .reconcile_reflection(request, receipt)
            .map_err(|failure| map_port_error(&failure));
        if result.is_ok() {
            self.require_clean_proxy_terminal()?;
        }
        self.ensure_live()?;
        result
    }

    #[must_use]
    pub fn containment_receipt(&self) -> &HermesContainmentReceipt {
        &self.receipt
    }

    #[must_use]
    pub const fn windows_launcher_pid(&self) -> u32 {
        self.windows_launcher_pid
    }

    #[must_use]
    pub const fn outer_pid(&self) -> u32 {
        self.outer_pid
    }

    #[must_use]
    pub const fn bwrap_pid(&self) -> u32 {
        self.bwrap_pid
    }

    /// Explicitly invalidates the receipt and reaps the owned WSL tree.
    ///
    /// # Errors
    ///
    /// Reports teardown ambiguity if the Job cannot prove all descendants
    /// exited. Drop retains kill-on-close as a final backstop.
    pub fn terminate(mut self) -> HermesAdapterResult<()> {
        self.owner.invalidate();
        let process_result = self.process.terminate();
        let proxy_result = self.codex_proxy.terminate();
        match (process_result, proxy_result) {
            (Err(process_failure), _) => Err(process_failure),
            (Ok(()), result) => result,
        }
    }

    fn prepare_operation(&mut self) -> PortResult<()> {
        self.ensure_live()?;
        let remaining = self
            .absolute_deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                PortError::new(
                    lattice_contracts::Component::Hermes,
                    lattice_ports::PortErrorKind::Timeout,
                    "HERMES_PRODUCTION_DEADLINE_EXCEEDED",
                )
            })?;
        self.adapter.config.timeout = self.operation_timeout.min(remaining);
        self.adapter.config.poll_interval = self.poll_interval.min(self.adapter.config.timeout);
        Ok(())
    }

    fn ensure_live(&mut self) -> PortResult<()> {
        if let Err(failure) = self.process.ensure_running() {
            return Err(self.fail_closed(&failure));
        }
        if let Err(failure) = self.codex_proxy.ensure_live() {
            return Err(self.fail_closed(&failure));
        }
        self.adapter
            .require_containment_receipt()
            .map(|_| ())
            .map_err(|failure| map_port_error(&failure))
    }

    fn require_clean_proxy_terminal(&mut self) -> PortResult<()> {
        match self
            .codex_proxy
            .wait_for_clean_terminal(self.absolute_deadline)
        {
            Ok(()) => Ok(()),
            Err(failure) => Err(self.fail_closed(&failure)),
        }
    }

    fn fail_closed(&mut self, failure: &HermesAdapterError) -> PortError {
        self.owner.invalidate();
        let process_result = self.process.terminate();
        let proxy_result = self.codex_proxy.stop_and_reap();
        if let Err(teardown) = process_result {
            return map_port_error(&teardown);
        }
        if let Err(teardown) = proxy_result {
            return map_port_error(&teardown);
        }
        map_port_error(&failure)
    }

    #[cfg(test)]
    pub(crate) fn terminate_child_for_test(&mut self) -> HermesAdapterResult<()> {
        self.process.terminate()
    }

    /// Returns bounded count-and-digest evidence for rejected proxy bytes.
    #[must_use]
    pub fn codex_proxy_failure_evidence(&self) -> Option<CodexProxyFailureEvidence> {
        self.codex_proxy.failure_evidence()
    }
}

impl HermesPort for ProductionHermesPort {
    fn research(&mut self, request: HermesResearchRequest) -> PortResult<HermesEvidence> {
        self.run_reflection_evidence(&request)
            .map(HermesReflectionEvidence::into_evidence)
    }

    fn interrupt(&mut self, request_id: &RequestId) -> PortResult<()> {
        self.prepare_operation()?;
        let result = HermesPort::interrupt(&mut self.adapter, request_id);
        self.ensure_live()?;
        result
    }
}

impl Drop for ProductionHermesPort {
    fn drop(&mut self) {
        self.owner.invalidate();
        let _ = self.process.terminate();
        let _ = self.codex_proxy.stop_and_reap();
    }
}

#[derive(Serialize)]
struct LaunchSecret<'a> {
    api_key: &'a str,
    broker_receipt_sha256: &'a str,
    config_sha256: &'a str,
    deadline_millis: u64,
    endpoint: &'a str,
    fixture_reflection: Option<&'a str>,
    mode: &'a str,
    model: &'a str,
    nonce: &'a str,
    runtime_manifest_sha256: &'a str,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StartupWire {
    bwrap_pid: u32,
    containment_frame_hex: String,
    containment_frame_sha256: String,
    outer_pid: u32,
    schema: String,
}

struct StartupObservation {
    wire: StartupWire,
    frame: Vec<u8>,
    windows_launcher_pid: u32,
    trailing: Vec<u8>,
}

struct VerifiedAttestation {
    endpoint: SocketAddr,
    namespace_pid: u32,
    runtime_manifest_sha256: String,
    broker_receipt_sha256: String,
    api_key_sha256: String,
    runner_nonce_sha256: String,
    bwrap_sha256: String,
    socketpair_binding_sha256: String,
    containment_frame_sha256: String,
    outer_pid: u32,
    bwrap_pid: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ContainmentAttestationWire {
    api_key_sha256: String,
    endpoint: String,
    mode: String,
    namespace_pid: u32,
    net_namespace: String,
    nonce_sha256: String,
    schema: String,
}

fn wait_for_startup(
    process: &mut crate::windows_job::WindowsJobChild,
    stream: &Receiver<OuterStreamEvent>,
    deadline: Instant,
) -> HermesAdapterResult<StartupObservation> {
    let mut bytes = Vec::new();
    loop {
        if let Some((wire, frame, consumed)) = parse_startup(&bytes)? {
            let trailing = bytes.split_off(consumed);
            return Ok(StartupObservation {
                wire,
                frame,
                windows_launcher_pid: process.process_id(),
                trailing,
            });
        }
        process.ensure_running()?;
        if Instant::now() >= deadline {
            return Err(error(
                HermesAdapterErrorKind::Timeout,
                "HERMES_PRODUCTION_STARTUP_TIMEOUT",
            ));
        }
        let wait = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_default()
            .min(Duration::from_millis(2));
        match stream.recv_timeout(wait) {
            Ok(OuterStreamEvent::Data(payload)) => {
                bytes.extend_from_slice(&payload);
                if bytes.len() > MAX_STARTUP_BYTES + MAX_CODEX_PROXY_WIRE_BYTES {
                    return Err(malformed("HERMES_PRODUCTION_STARTUP_LENGTH_REJECTED"));
                }
            }
            Ok(OuterStreamEvent::Eof) => {
                return Err(error(
                    HermesAdapterErrorKind::Failed,
                    "HERMES_PRODUCTION_CHILD_EXITED",
                ));
            }
            Ok(OuterStreamEvent::Failed) => {
                return Err(error(
                    HermesAdapterErrorKind::Transport,
                    "HERMES_PRODUCTION_STARTUP_READ_FAILED",
                ));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_PRODUCTION_STARTUP_STREAM_LOST",
                ));
            }
        }
    }
}

fn parse_startup(bytes: &[u8]) -> HermesAdapterResult<Option<(StartupWire, Vec<u8>, usize)>> {
    if bytes.len() < STARTUP_MAGIC.len() {
        if STARTUP_MAGIC.starts_with(bytes) {
            return Ok(None);
        }
        return Err(malformed("HERMES_PRODUCTION_STARTUP_MAGIC_REJECTED"));
    }
    if !bytes.starts_with(STARTUP_MAGIC) {
        return Err(malformed("HERMES_PRODUCTION_STARTUP_MAGIC_REJECTED"));
    }
    if bytes.len() < STARTUP_MAGIC.len() + 8 {
        return Ok(None);
    }
    let offset = STARTUP_MAGIC.len();
    let length = u64::from_be_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("fixed eight-byte slice"),
    );
    let length = usize::try_from(length)
        .map_err(|_| malformed("HERMES_PRODUCTION_STARTUP_LENGTH_REJECTED"))?;
    if length == 0 || length > MAX_STARTUP_BYTES {
        return Err(malformed("HERMES_PRODUCTION_STARTUP_LENGTH_REJECTED"));
    }
    let total = STARTUP_MAGIC
        .len()
        .checked_add(8)
        .and_then(|value| value.checked_add(length))
        .ok_or_else(|| malformed("HERMES_PRODUCTION_STARTUP_LENGTH_REJECTED"))?;
    if bytes.len() < total {
        return Ok(None);
    }
    let encoded = &bytes[offset + 8..total];
    let wire: StartupWire = serde_json::from_slice(encoded)
        .map_err(|_| malformed("HERMES_PRODUCTION_STARTUP_MALFORMED"))?;
    if serde_json::to_vec(&wire).map_err(|_| malformed("HERMES_PRODUCTION_STARTUP_MALFORMED"))?
        != encoded
        || wire.schema != STARTUP_SCHEMA
        || wire.outer_pid == 0
        || wire.bwrap_pid == 0
    {
        return Err(malformed("HERMES_PRODUCTION_STARTUP_MALFORMED"));
    }
    let frame = decode_hex(&wire.containment_frame_hex)?;
    if encode_sha256(&Sha256::digest(&frame)) != wire.containment_frame_sha256 {
        return Err(cross_binding(
            "HERMES_PRODUCTION_CONTAINMENT_FRAME_DIGEST_REJECTED",
        ));
    }
    Ok(Some((wire, frame, total)))
}

#[allow(clippy::too_many_arguments)]
fn verify_startup(
    startup: &StartupObservation,
    runtime_manifest_sha256: &str,
    broker_receipt_sha256: &str,
    api_key: &str,
    model: &str,
    nonce: &str,
    mode: &str,
) -> HermesAdapterResult<VerifiedAttestation> {
    let frame =
        parse_containment_frame(&startup.frame, HermesContainmentFrameLimits::new(16 * 1024))?;
    let api_key_sha256 = sha256_text(api_key);
    let nonce_bytes = decode_hex(nonce)?;
    let runner_nonce_sha256 = encode_sha256(&Sha256::digest(&nonce_bytes));
    let socketpair_binding_sha256 =
        digest_join(&[&nonce_bytes, b"LATTICE_HERMES_PRODUCTION_SOCKETPAIR_V1"]);
    let request_sha256 = digest_join(&[&nonce_bytes, b"LATTICE_HERMES_PRODUCTION_REQUEST_V1"]);
    let transcript_sha256 = digest_join(&[&nonce_bytes, b"LATTICE_HERMES_PRODUCTION_READY_V1"]);
    let config_sha256 = production_config_sha256(frame.endpoint(), &api_key_sha256, model, nonce)?;
    if frame.runtime_manifest_sha256() != runtime_manifest_sha256.as_bytes()
        || frame.config_sha256() != config_sha256.as_bytes()
        || frame.request_sha256() != request_sha256.as_bytes()
        || frame.broker_receipt_sha256() != broker_receipt_sha256.as_bytes()
        || frame.bwrap_sha256() != BWRAP_SHA256.as_bytes()
        || frame.socketpair_binding_sha256() != socketpair_binding_sha256.as_bytes()
        || frame.api_key_sha256() != api_key_sha256.as_bytes()
        || frame.nonce_sha256() != runner_nonce_sha256.as_bytes()
        || frame.transcript_sha256() != transcript_sha256.as_bytes()
        || frame.mode() != mode
    {
        return Err(cross_binding("HERMES_PRODUCTION_FRAME_BINDING_REJECTED"));
    }
    let metadata: ContainmentAttestationWire = serde_json::from_slice(frame.reflection())
        .map_err(|_| malformed("HERMES_PRODUCTION_ATTESTATION_REJECTED"))?;
    if serde_json::to_vec(&metadata)
        .map_err(|_| malformed("HERMES_PRODUCTION_ATTESTATION_REJECTED"))?
        != frame.reflection()
        || metadata.schema != ATTESTATION_SCHEMA
        || metadata.endpoint != frame.endpoint().to_string()
        || metadata.mode != mode
        || metadata.namespace_pid != frame.namespace_pid()
        || metadata.api_key_sha256 != api_key_sha256
        || metadata.nonce_sha256 != runner_nonce_sha256
        || !metadata.net_namespace.starts_with("net:[")
        || !metadata.net_namespace.ends_with(']')
    {
        return Err(cross_binding("HERMES_PRODUCTION_ATTESTATION_REJECTED"));
    }
    Ok(VerifiedAttestation {
        endpoint: frame.endpoint(),
        namespace_pid: frame.namespace_pid(),
        runtime_manifest_sha256: runtime_manifest_sha256.to_owned(),
        broker_receipt_sha256: broker_receipt_sha256.to_owned(),
        api_key_sha256,
        runner_nonce_sha256,
        bwrap_sha256: BWRAP_SHA256.to_owned(),
        socketpair_binding_sha256,
        containment_frame_sha256: startup.wire.containment_frame_sha256.clone(),
        outer_pid: startup.wire.outer_pid,
        bwrap_pid: startup.wire.bwrap_pid,
    })
}

fn mint_receipt(
    attestation: &VerifiedAttestation,
    windows_launcher_pid: u32,
    runner_nonce_sha256: String,
    owner: std::sync::Weak<ContainmentOwnerState>,
) -> HermesAdapterResult<HermesContainmentReceipt> {
    if windows_launcher_pid == 0
        || attestation.outer_pid == 0
        || attestation.bwrap_pid == 0
        || attestation.namespace_pid == 0
        || runner_nonce_sha256 != attestation.runner_nonce_sha256
    {
        return Err(cross_binding("HERMES_PRODUCTION_PID_BINDING_REJECTED"));
    }
    let mut digest = Sha256::new();
    digest.update(b"lattice.hermes.production-containment-receipt.v1\0");
    for field in [
        windows_launcher_pid.to_string(),
        attestation.outer_pid.to_string(),
        attestation.bwrap_pid.to_string(),
        attestation.namespace_pid.to_string(),
        attestation.endpoint.to_string(),
        attestation.api_key_sha256.clone(),
        runner_nonce_sha256.clone(),
        attestation.runtime_manifest_sha256.clone(),
        attestation.bwrap_sha256.clone(),
        attestation.socketpair_binding_sha256.clone(),
        attestation.broker_receipt_sha256.clone(),
        attestation.containment_frame_sha256.clone(),
    ] {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field.as_bytes());
    }
    let receipt_digest = ContentDigest::from_sha256(encode_sha256(&digest.finalize()))
        .map_err(|_| malformed("HERMES_PRODUCTION_RECEIPT_REJECTED"))?;
    Ok(HermesContainmentReceipt {
        endpoint: attestation.endpoint,
        api_key_sha256: attestation.api_key_sha256.clone(),
        runner_nonce_sha256,
        contained_pid: attestation.namespace_pid,
        runtime_manifest_sha256: attestation.runtime_manifest_sha256.clone(),
        bwrap_sha256: attestation.bwrap_sha256.clone(),
        socketpair_binding_sha256: attestation.socketpair_binding_sha256.clone(),
        broker_receipt_sha256: attestation.broker_receipt_sha256.clone(),
        containment_frame_sha256: attestation.containment_frame_sha256.clone(),
        receipt_digest,
        owner: Some(owner),
    })
}

fn production_config_sha256(
    endpoint: SocketAddr,
    api_key_sha256: &str,
    model: &str,
    nonce: &str,
) -> HermesAdapterResult<String> {
    #[derive(Serialize)]
    struct ConfigWire<'a> {
        api_key_sha256: &'a str,
        endpoint: String,
        model: &'a str,
        nonce: &'a str,
        schema: &'a str,
    }
    let bytes = serde_json::to_vec(&ConfigWire {
        api_key_sha256,
        endpoint: endpoint.to_string(),
        model,
        nonce,
        schema: CONFIG_SCHEMA,
    })
    .map_err(|_| malformed("HERMES_PRODUCTION_CONFIG_DIGEST_REJECTED"))?;
    Ok(encode_sha256(&Sha256::digest(bytes)))
}

fn map_outer_failure(
    process: &crate::windows_job::WindowsJobChild,
    fallback: HermesAdapterError,
) -> HermesAdapterError {
    let Ok(stderr) = process.read_stderr(4096) else {
        return fallback;
    };
    let Ok(stderr) = std::str::from_utf8(&stderr) else {
        return fallback;
    };
    let Some(code) = stderr
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("HERMES_OUTER_FAIL:"))
        .and_then(|value| value.parse::<u32>().ok())
    else {
        return fallback;
    };
    match code {
        64 => error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PRODUCTION_ARGUMENT_REJECTED",
        ),
        65 | 66 => error(
            HermesAdapterErrorKind::Identity,
            "HERMES_PRODUCTION_RUNTIME_IDENTITY_REJECTED",
        ),
        67..=73 | 76..=78 => error(
            HermesAdapterErrorKind::CrossBinding,
            "HERMES_PRODUCTION_CONTAINMENT_PROTOCOL_REJECTED",
        ),
        74 => error(
            HermesAdapterErrorKind::CapabilityMismatch,
            "HERMES_OFFICIAL_SERVER_NOT_STAGED",
        ),
        75 => error(
            HermesAdapterErrorKind::CapabilityMismatch,
            "HERMES_OFFICIAL_SERVER_STARTUP_BLOCKED",
        ),
        79 => error(
            HermesAdapterErrorKind::Timeout,
            "HERMES_PRODUCTION_DEADLINE_EXCEEDED",
        ),
        _ => fallback,
    }
}

fn production_nonce(isolation_root: &Path) -> HermesAdapterResult<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| malformed("HERMES_PRODUCTION_NONCE_CLOCK_REJECTED"))?;
    let sequence = RUNNER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut digest = Sha256::new();
    digest.update(b"lattice.hermes.production.nonce.v1\0");
    digest.update(std::process::id().to_be_bytes());
    digest.update(sequence.to_be_bytes());
    digest.update(now.as_nanos().to_be_bytes());
    digest.update(isolation_root.as_os_str().to_string_lossy().as_bytes());
    Ok(encode_sha256(&digest.finalize()))
}

fn write_new_secret(path: &Path, bytes: &[u8]) -> HermesAdapterResult<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| {
            error(
                HermesAdapterErrorKind::Spawn,
                "HERMES_PRODUCTION_SECRET_CREATE_FAILED",
            )
        })?;
    file.write_all(bytes).map_err(|_| {
        error(
            HermesAdapterErrorKind::Spawn,
            "HERMES_PRODUCTION_SECRET_WRITE_FAILED",
        )
    })?;
    file.sync_all().map_err(|_| {
        error(
            HermesAdapterErrorKind::Spawn,
            "HERMES_PRODUCTION_SECRET_WRITE_FAILED",
        )
    })
}

fn write_new_runner(path: &Path, bytes: &[u8]) -> HermesAdapterResult<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| {
            error(
                HermesAdapterErrorKind::Spawn,
                "HERMES_PRODUCTION_RUNNER_CREATE_FAILED",
            )
        })?;
    file.write_all(bytes).map_err(|_| {
        error(
            HermesAdapterErrorKind::Spawn,
            "HERMES_PRODUCTION_RUNNER_WRITE_FAILED",
        )
    })?;
    file.sync_all().map_err(|_| {
        error(
            HermesAdapterErrorKind::Spawn,
            "HERMES_PRODUCTION_RUNNER_WRITE_FAILED",
        )
    })
}

fn remove_ingress(path: &Path) {
    drop(fs::remove_file(path));
}

fn windows_path_to_wsl(path: &Path) -> HermesAdapterResult<String> {
    let text = path.as_os_str().to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let bytes = text.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !matches!(bytes[2], b'\\' | b'/')
        || text.starts_with(r"\\")
    {
        return Err(error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PRODUCTION_WSL_PATH_REJECTED",
        ));
    }
    let drive = char::from(bytes[0].to_ascii_lowercase());
    let suffix = text[3..].replace('\\', "/");
    if suffix.split('/').any(|part| part == "..") {
        return Err(error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PRODUCTION_WSL_PATH_REJECTED",
        ));
    }
    Ok(format!("/mnt/{drive}/{suffix}"))
}

fn decode_hex(value: &str) -> HermesAdapterResult<Vec<u8>> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return Err(malformed("HERMES_PRODUCTION_HEX_REJECTED"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair)
                .map_err(|_| malformed("HERMES_PRODUCTION_HEX_REJECTED"))?;
            u8::from_str_radix(pair, 16).map_err(|_| malformed("HERMES_PRODUCTION_HEX_REJECTED"))
        })
        .collect()
}

fn digest_join(parts: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part);
    }
    encode_sha256(&digest.finalize())
}

#[cfg(test)]
mod proxy_host_tests {
    use std::io;
    use std::sync::Condvar;

    use super::*;

    #[derive(Default)]
    struct BlockingProxyState {
        cancelled: Mutex<bool>,
        wake: Condvar,
        open_entered: AtomicBool,
        read_exited: AtomicBool,
        write_entered: AtomicBool,
    }

    struct BlockingProxyControl(Arc<BlockingProxyState>);

    impl ProductionCodexProxyControl for BlockingProxyControl {
        fn ensure_running(&self) -> HermesAdapterResult<()> {
            if *self.0.cancelled.lock().expect("blocking control lock") {
                Err(error(
                    HermesAdapterErrorKind::Failed,
                    "HERMES_CODEX_PROXY_TEST_CHILD_EXITED",
                ))
            } else {
                Ok(())
            }
        }

        fn terminate(&self) -> HermesAdapterResult<()> {
            *self.0.cancelled.lock().expect("blocking control lock") = true;
            self.0.wake.notify_all();
            Ok(())
        }
    }

    struct BlockingOpenProvider {
        state: Arc<BlockingProxyState>,
        control: Arc<BlockingProxyControl>,
    }

    impl BlockingOpenProvider {
        fn new(state: Arc<BlockingProxyState>) -> Self {
            Self {
                control: Arc::new(BlockingProxyControl(Arc::clone(&state))),
                state,
            }
        }
    }

    impl ProductionCodexProxyProvider for BlockingOpenProvider {
        fn control(&self) -> Arc<dyn ProductionCodexProxyControl> {
            self.control.clone()
        }

        fn open(
            self: Box<Self>,
            _absolute_deadline: Instant,
        ) -> HermesAdapterResult<ProductionCodexProxyDuplex> {
            self.state.open_entered.store(true, Ordering::Release);
            let mut cancelled = self.state.cancelled.lock().expect("blocking open lock");
            while !*cancelled {
                cancelled = self.state.wake.wait(cancelled).expect("blocking open wait");
            }
            Err(error(
                HermesAdapterErrorKind::Cancelled,
                "HERMES_CODEX_PROXY_OPEN_CANCELLED",
            ))
        }
    }

    struct BlockingReader(Arc<BlockingProxyState>);

    impl Read for BlockingReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            let mut cancelled = self.0.cancelled.lock().expect("blocking read lock");
            while !*cancelled {
                cancelled = self.0.wake.wait(cancelled).expect("blocking read wait");
            }
            self.0.read_exited.store(true, Ordering::Release);
            Ok(0)
        }
    }

    struct BlockingWriter(Arc<BlockingProxyState>);

    impl Write for BlockingWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            self.0.write_entered.store(true, Ordering::Release);
            let mut cancelled = self.0.cancelled.lock().expect("blocking write lock");
            while !*cancelled {
                cancelled = self.0.wake.wait(cancelled).expect("blocking write wait");
            }
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "cancelled test provider",
            ))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct BlockingIoProvider {
        state: Arc<BlockingProxyState>,
        control: Arc<BlockingProxyControl>,
    }

    impl BlockingIoProvider {
        fn new(state: Arc<BlockingProxyState>) -> Self {
            Self {
                control: Arc::new(BlockingProxyControl(Arc::clone(&state))),
                state,
            }
        }
    }

    impl ProductionCodexProxyProvider for BlockingIoProvider {
        fn control(&self) -> Arc<dyn ProductionCodexProxyControl> {
            self.control.clone()
        }

        fn open(
            self: Box<Self>,
            _absolute_deadline: Instant,
        ) -> HermesAdapterResult<ProductionCodexProxyDuplex> {
            Ok(ProductionCodexProxyDuplex::new(
                Box::new(BlockingReader(Arc::clone(&self.state))),
                Box::new(BlockingWriter(Arc::clone(&self.state))),
            ))
        }
    }

    fn test_binding(nonce: &str, receipt: &ContentDigest) -> [u8; 32] {
        CodexProxyHostSession::new(nonce, receipt, Instant::now() + Duration::from_secs(2))
            .expect("test session")
            .binding()
    }

    fn test_sink(name: &str) -> (std::fs::File, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "lattice-hermes-{name}-{}-{}",
            std::process::id(),
            RUNNER_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let sink = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("create exact test sink");
        (sink, path)
    }

    fn wait_until(predicate: impl Fn() -> bool, message: &str) {
        let deadline = Instant::now() + Duration::from_secs(1);
        while !predicate() {
            assert!(Instant::now() < deadline, "{message}");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn pre_magic_bytes_fail_closed_with_only_bounded_digest_evidence() {
        let path = std::env::temp_dir().join(format!(
            "lattice-hermes-proxy-evidence-{}-{}",
            std::process::id(),
            RUNNER_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let outer_input = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("create exact test sink");
        let (sender, receiver) = mpsc::sync_channel(1);
        let owner = Arc::new(ContainmentOwnerState::new("11".repeat(32)));
        let mut host = ProductionCodexProxyHost::start(
            Box::new(ScriptedCodexProxyProvider::default()),
            &"22".repeat(32),
            &ContentDigest::from_sha256("33".repeat(32)).expect("digest"),
            Instant::now() + Duration::from_secs(2),
            outer_input,
            receiver,
            Vec::new(),
            Arc::clone(&owner),
        )
        .expect("start proxy host");
        let diagnostic = b"bwrap: pre-exec diagnostic\n";
        sender
            .send(OuterStreamEvent::Data(diagnostic.to_vec()))
            .expect("send untrusted diagnostic");
        let deadline = Instant::now() + Duration::from_secs(1);
        let failure = loop {
            if let Err(failure) = host.ensure_live() {
                break failure;
            }
            assert!(
                Instant::now() < deadline,
                "proxy rejection reached deadline"
            );
            thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(failure.code(), "HERMES_CODEX_PROXY_MAGIC_REJECTED");
        let evidence = host.failure_evidence().expect("bounded failure evidence");
        assert_eq!(evidence.byte_count(), diagnostic.len() as u64);
        assert_eq!(
            evidence.sha256(),
            encode_sha256(&Sha256::digest(diagnostic))
        );
        assert!(!owner.active.load(Ordering::Acquire));
        let terminal = host.terminate().expect_err("terminal failure is retained");
        assert_eq!(terminal.code(), "HERMES_CODEX_PROXY_MAGIC_REJECTED");
        drop(host);
        fs::remove_file(path).expect("remove exact test sink");
    }

    #[test]
    fn reverse_half_close_keeps_inbound_data_and_close_valid() {
        let receipt = ContentDigest::from_sha256("33".repeat(32)).expect("digest");
        let nonce = "22".repeat(32);
        let mut session =
            CodexProxyHostSession::new(&nonce, &receipt, Instant::now() + Duration::from_secs(2))
                .expect("test session");
        let binding = session.binding();
        let open = encode_codex_proxy_test_frame(1, 0, binding, &[]);
        assert_eq!(
            session.accept(&open).expect("open"),
            CodexProxyHostEvent::Open
        );
        session.encode_open_ack().expect("open ack");
        session.encode_close().expect("provider output close");
        let data = encode_codex_proxy_test_frame(2, 1, binding, b"late input");
        assert_eq!(
            session.accept(&data).expect("independent inbound data"),
            CodexProxyHostEvent::Data(b"late input".to_vec())
        );
        let close = encode_codex_proxy_test_frame(3, 2, binding, &[]);
        assert_eq!(
            session.accept(&close).expect("independent inbound close"),
            CodexProxyHostEvent::Close
        );
    }

    #[test]
    fn provider_eof_before_authenticated_remote_close_fails_closed() {
        let (outer_input, path) = test_sink("proxy-early-eof");
        let (sender, receiver) = mpsc::sync_channel(8);
        let owner = Arc::new(ContainmentOwnerState::new("11".repeat(32)));
        let nonce = "22".repeat(32);
        let receipt = ContentDigest::from_sha256("33".repeat(32)).expect("digest");
        let binding = test_binding(&nonce, &receipt);
        let mut host = ProductionCodexProxyHost::start(
            Box::new(ScriptedCodexProxyProvider::default()),
            &nonce,
            &receipt,
            Instant::now() + Duration::from_secs(2),
            outer_input,
            receiver,
            Vec::new(),
            Arc::clone(&owner),
        )
        .expect("start proxy host");
        sender
            .send(OuterStreamEvent::Data(encode_codex_proxy_test_frame(
                1,
                0,
                binding,
                &[],
            )))
            .expect("send open");
        wait_until(|| host.ensure_live().is_err(), "early EOF was not rejected");
        let failure = host.ensure_live().expect_err("early EOF stays failed");
        assert_eq!(
            failure.code(),
            "HERMES_CODEX_PROXY_PROVIDER_EOF_BEFORE_CLOSE"
        );
        assert!(!owner.active.load(Ordering::Acquire));
        let _ = host.terminate();
        drop(host);
        fs::remove_file(path).expect("remove exact test sink");
    }

    #[test]
    fn blocked_provider_open_is_cancelled_before_bounded_join() {
        let (outer_input, path) = test_sink("proxy-blocked-open");
        let (sender, receiver) = mpsc::sync_channel(8);
        let state = Arc::new(BlockingProxyState::default());
        let nonce = "22".repeat(32);
        let receipt = ContentDigest::from_sha256("33".repeat(32)).expect("digest");
        let binding = test_binding(&nonce, &receipt);
        let host = ProductionCodexProxyHost::start(
            Box::new(BlockingOpenProvider::new(Arc::clone(&state))),
            &nonce,
            &receipt,
            Instant::now() + Duration::from_secs(2),
            outer_input,
            receiver,
            Vec::new(),
            Arc::new(ContainmentOwnerState::new("11".repeat(32))),
        )
        .expect("start proxy host");
        sender
            .send(OuterStreamEvent::Data(encode_codex_proxy_test_frame(
                1,
                0,
                binding,
                &[],
            )))
            .expect("send open");
        wait_until(
            || state.open_entered.load(Ordering::Acquire),
            "provider open did not block",
        );
        let (result_sender, result_receiver) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut host = host;
            let _ = result_sender.send(host.terminate());
        });
        let result = result_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("terminate must cancel open before bounded join");
        assert_eq!(
            result.expect_err("cancelled open is retained").code(),
            "HERMES_CODEX_PROXY_OPEN_CANCELLED"
        );
        fs::remove_file(path).expect("remove exact test sink");
    }

    #[test]
    fn blocked_provider_read_and_write_are_cancelled_before_join() {
        let (outer_input, path) = test_sink("proxy-blocked-io");
        let (sender, receiver) = mpsc::sync_channel(8);
        let state = Arc::new(BlockingProxyState::default());
        let nonce = "22".repeat(32);
        let receipt = ContentDigest::from_sha256("33".repeat(32)).expect("digest");
        let binding = test_binding(&nonce, &receipt);
        let mut host = ProductionCodexProxyHost::start(
            Box::new(BlockingIoProvider::new(Arc::clone(&state))),
            &nonce,
            &receipt,
            Instant::now() + Duration::from_secs(2),
            outer_input,
            receiver,
            Vec::new(),
            Arc::new(ContainmentOwnerState::new("11".repeat(32))),
        )
        .expect("start proxy host");
        sender
            .send(OuterStreamEvent::Data(encode_codex_proxy_test_frame(
                1,
                0,
                binding,
                &[],
            )))
            .expect("send open");
        wait_until(
            || host.status.lock().expect("host status").authenticated_open,
            "provider did not authenticate open",
        );
        sender
            .send(OuterStreamEvent::Data(encode_codex_proxy_test_frame(
                2,
                1,
                binding,
                b"blocked input",
            )))
            .expect("send provider input");
        wait_until(
            || state.write_entered.load(Ordering::Acquire),
            "provider write did not block",
        );
        let started = Instant::now();
        let failure = host
            .terminate()
            .expect_err("cancelled blocked write remains a failure");
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(failure.code(), "HERMES_CODEX_PROXY_PROVIDER_WRITE_FAILED");
        wait_until(
            || state.read_exited.load(Ordering::Acquire),
            "provider read did not exit after cancellation",
        );
        drop(host);
        fs::remove_file(path).expect("remove exact test sink");
    }

    #[test]
    fn clean_terminal_is_required_before_evidence_barrier() {
        let (outer_input, path) = test_sink("proxy-no-open");
        let (_sender, receiver) = mpsc::sync_channel(1);
        let mut host = ProductionCodexProxyHost::start(
            Box::new(ScriptedCodexProxyProvider::default()),
            &"22".repeat(32),
            &ContentDigest::from_sha256("33".repeat(32)).expect("digest"),
            Instant::now() + Duration::from_secs(2),
            outer_input,
            receiver,
            Vec::new(),
            Arc::new(ContainmentOwnerState::new("11".repeat(32))),
        )
        .expect("start proxy host");
        let failure = host
            .wait_for_clean_terminal(Instant::now() + Duration::from_millis(25))
            .expect_err("no-open host cannot cross completion barrier");
        assert_eq!(failure.code(), "HERMES_CODEX_PROXY_COMPLETION_TIMEOUT");
        host.terminate().expect("unused provider terminates");
        drop(host);
        fs::remove_file(path).expect("remove exact test sink");
    }
}
