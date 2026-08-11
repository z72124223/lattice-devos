//! Bounded MCP stdio surface for the canonical `latticed` entry.

use std::env;
use std::error::Error;
use std::fmt;
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{ContentDigest, SubjectBinding};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

/// Legacy stateful MCP protocol version implemented by this server.
pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
/// Stateless MCP protocol version implemented by this server.
pub const MCP_STATELESS_PROTOCOL_VERSION: &str = "2026-07-28";
/// Sole delivery execution tool.
pub const DELIVERY_RUN_TOOL: &str = "lattice_delivery_run";
/// Sole delivery status tool.
pub const DELIVERY_STATUS_TOOL: &str = "lattice_delivery_status";
/// Bounded high-level task submission tool.
pub const TASK_SUBMIT_TOOL: &str = "lattice_task_submit";
/// Bounded durable task status tool.
pub const TASK_STATUS_TOOL: &str = "lattice_task_status";
/// Sole task intent accepted by the transport boundary.
pub const CONTROLLED_CODEX_CANARY_INTENT: &str = "CONTROLLED_CODEX_CANARY";

const LEGACY_DELIVERY_RUN_DISABLED: &str = "LATTICE_DELIVERY_RUN_REQUIRES_CANONICAL_LATTICED";

const MAX_CLIENT_REQUEST_ID_BYTES: usize = 64;
const TASK_PUBLIC_STATUS_SCHEMA_VERSION: &str = "lattice.task.status.v1";
const TASK_PUBLIC_STATUS_VALUES: [&str; 4] = [
    "NOT_SUBMITTED",
    "RECONCILIATION_REQUIRED",
    "FAILED",
    "COMPLETED",
];
const TASK_PUBLIC_STATE_VALUES: [&str; 15] = [
    "NOT_SUBMITTED",
    "DRAFT",
    "AWAITING_EXECUTION_APPROVAL",
    "PREPARING",
    "EXECUTING",
    "VERIFYING",
    "REVIEWING",
    "AWAITING_MERGE_APPROVAL",
    "MERGING",
    "COMPLETED",
    "REJECTED",
    "BLOCKED",
    "FAILED",
    "STOPPING",
    "CANCELLED",
];

/// Maximum encoded bytes accepted for one newline-delimited stdio message.
pub const MAX_STDIO_MESSAGE_BYTES: usize = 65_536;
/// Maximum valid tool invocations accepted during one MCP server session.
pub const MAX_TOOL_INVOCATIONS_PER_SESSION: usize = 64;

const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const META_LOG_LEVEL: &str = "io.modelcontextprotocol/logLevel";
const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

const ACCEPTANCE_EVIDENCE_PATH_ENV: &str = "LATTICE_MCP_ACCEPTANCE_EVIDENCE_PATH";
const ACCEPTANCE_SESSION_ID_ENV: &str = "LATTICE_MCP_ACCEPTANCE_SESSION_ID";
const ACCEPTANCE_SAFE_CONFIG_SHA256_ENV: &str = "LATTICE_MCP_ACCEPTANCE_SAFE_CONFIG_SHA256";
const ACCEPTANCE_EVIDENCE_SCHEMA: &str = "lattice.mcp.acceptance-dispatch.v1";
const ACCEPTANCE_EVIDENCE_HASH_DOMAIN: &str = "lattice.mcp.acceptance-dispatch-hash.v1";

struct AcceptanceEvidence {
    file: File,
    session_id: String,
    safe_config_sha256: String,
    ordinal: u64,
    dispatch_accepted_count: u64,
    previous_event_sha256: String,
}

impl AcceptanceEvidence {
    fn from_process_environment() -> io::Result<Option<Self>> {
        let path = env::var_os(ACCEPTANCE_EVIDENCE_PATH_ENV);
        let session_id = env::var_os(ACCEPTANCE_SESSION_ID_ENV);
        let safe_config_sha256 = env::var_os(ACCEPTANCE_SAFE_CONFIG_SHA256_ENV);
        if path.is_none() && session_id.is_none() && safe_config_sha256.is_none() {
            return Ok(None);
        }
        let path = path
            .and_then(|value| value.into_string().ok())
            .ok_or_else(|| acceptance_evidence_error("incomplete or non-UTF-8 evidence path"))?;
        let session_id = session_id
            .and_then(|value| value.into_string().ok())
            .ok_or_else(|| acceptance_evidence_error("incomplete or non-UTF-8 session id"))?;
        let safe_config_sha256 = safe_config_sha256
            .and_then(|value| value.into_string().ok())
            .ok_or_else(|| acceptance_evidence_error("incomplete or non-UTF-8 safe config"))?;
        Self::open(&PathBuf::from(path), session_id, safe_config_sha256).map(Some)
    }

    fn open(path: &Path, session_id: String, safe_config_sha256: String) -> io::Result<Self> {
        if !path.is_absolute()
            || path
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with(r"\\.\pipe\")
            || !valid_lower_hex(&session_id, 32)
            || !valid_lower_hex(&safe_config_sha256, 64)
        {
            return Err(acceptance_evidence_error("invalid evidence configuration"));
        }
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != 0 {
            return Err(acceptance_evidence_error(
                "evidence sink is not a fresh regular file",
            ));
        }
        let file = OpenOptions::new().append(true).open(path)?;
        if !file.metadata()?.is_file() || file.metadata()?.len() != 0 {
            return Err(acceptance_evidence_error(
                "evidence sink changed before open",
            ));
        }
        let mut evidence = Self {
            file,
            session_id,
            safe_config_sha256,
            ordinal: 0,
            dispatch_accepted_count: 0,
            previous_event_sha256: "0".repeat(64),
        };
        evidence.append("SESSION_OPEN", None, None)?;
        Ok(evidence)
    }

    fn record_dispatch(&mut self, tool_name: &str, request_id: &Value) -> io::Result<()> {
        self.dispatch_accepted_count = self
            .dispatch_accepted_count
            .checked_add(1)
            .ok_or_else(|| acceptance_evidence_error("dispatch counter overflow"))?;
        let request_id_bytes = serde_json::to_vec(request_id)
            .map_err(|error| acceptance_evidence_error(&error.to_string()))?;
        let request_id_sha256 = sha256_hex(&request_id_bytes);
        self.append(
            "DISPATCH_ACCEPTED",
            Some(tool_name),
            Some(&request_id_sha256),
        )
    }

    fn close(&mut self) -> io::Result<()> {
        self.append("SESSION_CLOSED", None, None)
    }

    fn append(
        &mut self,
        record_type: &str,
        tool_name: Option<&str>,
        request_id_sha256: Option<&str>,
    ) -> io::Result<()> {
        self.ordinal = self
            .ordinal
            .checked_add(1)
            .ok_or_else(|| acceptance_evidence_error("event ordinal overflow"))?;
        let observed_at_unix_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| acceptance_evidence_error(&error.to_string()))?
            .as_nanos()
            .to_string();
        let tool_name_hash_field = tool_name.unwrap_or("null");
        let request_id_hash_field = request_id_sha256.unwrap_or("null");
        let ordinal = self.ordinal.to_string();
        let process_id = std::process::id().to_string();
        let dispatch_accepted_count = self.dispatch_accepted_count.to_string();
        let hash_input = [
            ACCEPTANCE_EVIDENCE_HASH_DOMAIN,
            &self.previous_event_sha256,
            &self.session_id,
            &self.safe_config_sha256,
            record_type,
            &ordinal,
            &process_id,
            tool_name_hash_field,
            request_id_hash_field,
            &dispatch_accepted_count,
            &observed_at_unix_nanos,
        ]
        .join("\n");
        let event_sha256 = sha256_hex(hash_input.as_bytes());
        let record = json!({
            "schema": ACCEPTANCE_EVIDENCE_SCHEMA,
            "record_type": record_type,
            "session_id": self.session_id,
            "safe_config_sha256": self.safe_config_sha256,
            "process_id": std::process::id(),
            "ordinal": self.ordinal,
            "tool_name": tool_name,
            "request_id_sha256": request_id_sha256,
            "dispatch_accepted_count": self.dispatch_accepted_count,
            "observed_at_unix_nanos": observed_at_unix_nanos,
            "previous_event_sha256": self.previous_event_sha256,
            "event_sha256": event_sha256,
        });
        let mut bytes = serde_json::to_vec(&record)
            .map_err(|error| acceptance_evidence_error(&error.to_string()))?;
        bytes.push(b'\n');
        self.file.write_all(&bytes)?;
        self.file.sync_all()?;
        self.previous_event_sha256 = event_sha256;
        Ok(())
    }
}

fn acceptance_evidence_error(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("LATTICE_MCP_ACCEPTANCE_EVIDENCE_REJECTED:{message}"),
    )
}

fn valid_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

/// Returns the process-owned commitment to the authorization-relevant MCP
/// protocol and closed tool schemas. Descriptions are intentionally excluded;
/// the adapter binary digest independently commits the complete executable.
pub(crate) fn task_ingress_schema_digest() -> Option<ContentDigest> {
    let value = CanonicalValue::Object(vec![
        (
            "legacy_protocol".to_owned(),
            CanonicalValue::String(MCP_PROTOCOL_VERSION.to_owned()),
        ),
        (
            "stateless_protocol".to_owned(),
            CanonicalValue::String(MCP_STATELESS_PROTOCOL_VERSION.to_owned()),
        ),
        (
            "delivery_tools".to_owned(),
            CanonicalValue::Array(vec![
                CanonicalValue::String(DELIVERY_RUN_TOOL.to_owned()),
                CanonicalValue::String(DELIVERY_STATUS_TOOL.to_owned()),
            ]),
        ),
        (
            "delivery_schema".to_owned(),
            CanonicalValue::String("closed-empty-object".to_owned()),
        ),
        (
            "task_submit_tool".to_owned(),
            CanonicalValue::String(TASK_SUBMIT_TOOL.to_owned()),
        ),
        (
            "task_submit_schema".to_owned(),
            CanonicalValue::String(format!(
                "closed:client_request_id:ascii-control-id:1..={MAX_CLIENT_REQUEST_ID_BYTES};intent:{CONTROLLED_CODEX_CANARY_INTENT}"
            )),
        ),
        (
            "task_status_tool".to_owned(),
            CanonicalValue::String(TASK_STATUS_TOOL.to_owned()),
        ),
        (
            "task_status_schema".to_owned(),
            CanonicalValue::String("closed:task_ref:lower-sha256".to_owned()),
        ),
        (
            "task_output_schema".to_owned(),
            CanonicalValue::String(
                "closed:schema_version:lattice.task.status.v1;status:NOT_SUBMITTED|RECONCILIATION_REQUIRED|FAILED|COMPLETED;task_state:NOT_SUBMITTED|DRAFT|AWAITING_EXECUTION_APPROVAL|PREPARING|EXECUTING|VERIFYING|REVIEWING|AWAITING_MERGE_APPROVAL|MERGING|COMPLETED|REJECTED|BLOCKED|FAILED|STOPPING|CANCELLED;task_ref:lower-sha256;ledger_head_digest:lower-sha256;result_digest:lower-sha256|null"
                    .to_owned(),
            ),
        ),
    ]);
    let domain = HashDomain::new("lattice.mcp.task-ingress-schema", "1.0").ok()?;
    let digest = canonical_sha256(&domain, &value).ok()?;
    ContentDigest::from_sha256(digest.to_hex()).ok()
}

/// Bounded execution failure safe for an MCP tool result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolExecutionError {
    code: &'static str,
}

impl ToolExecutionError {
    /// Constructs one static, secret-free failure.
    #[must_use]
    pub const fn new(code: &'static str) -> Self {
        Self { code }
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        self.code
    }
}

impl fmt::Display for ToolExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl Error for ToolExecutionError {}

/// Exact fixed-task selector injected by composition for both MCP tools.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryToolArguments {
    binding: SubjectBinding,
}

impl DeliveryToolArguments {
    pub(crate) const fn new(binding: SubjectBinding) -> Self {
        Self { binding }
    }

    /// Returns the fully typed immutable task binding selected by composition.
    #[must_use]
    pub const fn binding(&self) -> &SubjectBinding {
        &self.binding
    }
}

/// Validated high-level task request accepted by the MCP transport boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskSubmitArguments {
    client_request_id: String,
    intent: String,
}

impl TaskSubmitArguments {
    fn from_value(value: Option<&Value>) -> Option<Self> {
        let arguments = value?.as_object()?;
        if arguments.len() != 2
            || !arguments.contains_key("client_request_id")
            || !arguments.contains_key("intent")
        {
            return None;
        }
        let client_request_id = arguments.get("client_request_id")?.as_str()?;
        let intent = arguments.get("intent")?.as_str()?;
        if !valid_client_request_id(client_request_id) || intent != CONTROLLED_CODEX_CANARY_INTENT {
            return None;
        }
        Some(Self {
            client_request_id: client_request_id.to_owned(),
            intent: intent.to_owned(),
        })
    }

    /// Returns the bounded idempotency key supplied by the MCP client.
    #[must_use]
    pub fn client_request_id(&self) -> &str {
        &self.client_request_id
    }

    /// Returns the one high-level task intent admitted by this transport slice.
    #[must_use]
    pub fn intent(&self) -> &str {
        &self.intent
    }
}

/// Validated durable task reference accepted by the MCP transport boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskStatusArguments {
    task_ref: String,
}

impl TaskStatusArguments {
    fn from_value(value: Option<&Value>) -> Option<Self> {
        let arguments = value?.as_object()?;
        if arguments.len() != 1 || !arguments.contains_key("task_ref") {
            return None;
        }
        let task_ref = arguments.get("task_ref")?.as_str()?;
        if !valid_task_ref(task_ref) {
            return None;
        }
        Some(Self {
            task_ref: task_ref.to_owned(),
        })
    }

    /// Returns the exact lowercase SHA-256 task reference.
    #[must_use]
    pub fn task_ref(&self) -> &str {
        &self.task_ref
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TaskPublicStatus {
    status: String,
    task_state: String,
    task_ref: String,
    ledger_head_digest: String,
    result_digest: Option<String>,
}

impl TaskPublicStatus {
    fn from_value(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        if object.len() != 6
            || ![
                "schema_version",
                "status",
                "task_state",
                "task_ref",
                "ledger_head_digest",
                "result_digest",
            ]
            .iter()
            .all(|field| object.contains_key(*field))
        {
            return None;
        }

        if object.get("schema_version")?.as_str()? != TASK_PUBLIC_STATUS_SCHEMA_VERSION {
            return None;
        }
        let status = object.get("status")?.as_str()?;
        if !TASK_PUBLIC_STATUS_VALUES.contains(&status) {
            return None;
        }
        let task_state = object.get("task_state")?.as_str()?;
        if !TASK_PUBLIC_STATE_VALUES.contains(&task_state) {
            return None;
        }
        let task_ref = object.get("task_ref")?.as_str()?;
        let ledger_head_digest = object.get("ledger_head_digest")?.as_str()?;
        if !valid_task_ref(task_ref) || !valid_task_ref(ledger_head_digest) {
            return None;
        }
        let result_digest = match object.get("result_digest")? {
            Value::Null => None,
            Value::String(value) if valid_task_ref(value) => Some(value.clone()),
            _ => return None,
        };

        Some(Self {
            status: status.to_owned(),
            task_state: task_state.to_owned(),
            task_ref: task_ref.to_owned(),
            ledger_head_digest: ledger_head_digest.to_owned(),
            result_digest,
        })
    }

    fn into_value(self) -> Value {
        json!({
            "schema_version": TASK_PUBLIC_STATUS_SCHEMA_VERSION,
            "status": self.status,
            "task_state": self.task_state,
            "task_ref": self.task_ref,
            "ledger_head_digest": self.ledger_head_digest,
            "result_digest": self.result_digest,
        })
    }
}

/// Composition-owned typed operations exposed by MCP.
pub trait DeliveryToolService {
    /// Executes the fixed delivery profile.
    ///
    /// # Errors
    ///
    /// Returns only a stable, secret-free failure code.
    fn run(&mut self, arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError>;

    /// Reads the fixed delivery profile's durable status.
    ///
    /// # Errors
    ///
    /// Returns only a stable, secret-free failure code.
    fn status(&mut self, arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError>;

    /// Submits one validated high-level task intent to the existing service.
    ///
    /// # Errors
    ///
    /// Returns only a stable, secret-free failure code.
    fn task_submit(&mut self, arguments: &TaskSubmitArguments)
    -> Result<Value, ToolExecutionError>;

    /// Reads one validated durable task reference from the existing service.
    ///
    /// # Errors
    ///
    /// Returns only a stable, secret-free failure code.
    fn task_status(&mut self, arguments: &TaskStatusArguments)
    -> Result<Value, ToolExecutionError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Lifecycle {
    AwaitingInitialize,
    AwaitingInitialized,
    Ready,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestProtocol {
    Legacy,
    Stateless,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolSurface {
    CanonicalTaskControl,
    LegacyDeliveryObserver,
}

impl ToolSurface {
    const fn allows_task_control(self) -> bool {
        matches!(self, Self::CanonicalTaskControl)
    }

    const fn allows_delivery_run(self) -> bool {
        matches!(self, Self::CanonicalTaskControl)
    }

    const fn instructions(self) -> &'static str {
        match self {
            Self::CanonicalTaskControl => {
                "Four bounded LATTICE tools. Authority, task binding, orchestration, and execution configuration remain server-owned."
            }
            Self::LegacyDeliveryObserver => {
                "Legacy LATTICE delivery observer. Delivery mutation and task control are available only through the canonical latticed entrypoint."
            }
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum RequestProtocolError {
    InvalidMetadata,
    Unsupported(String),
}

/// Stateful MCP lifecycle and request dispatcher.
pub struct McpServer<S> {
    service: S,
    arguments: DeliveryToolArguments,
    lifecycle: Lifecycle,
    tool_surface: ToolSurface,
    tool_invocations: usize,
    acceptance_evidence: Option<AcceptanceEvidence>,
    acceptance_evidence_error: Option<io::Error>,
}

impl<S: DeliveryToolService> McpServer<S> {
    /// Constructs an uninitialized server.
    #[must_use]
    pub const fn new(service: S, binding: SubjectBinding) -> Self {
        Self {
            service,
            arguments: DeliveryToolArguments::new(binding),
            lifecycle: Lifecycle::AwaitingInitialize,
            tool_surface: ToolSurface::CanonicalTaskControl,
            tool_invocations: 0,
            acceptance_evidence: None,
            acceptance_evidence_error: None,
        }
    }

    /// Constructs an uninitialized legacy observer with no mutation capability.
    #[must_use]
    pub const fn new_legacy_delivery_observer(service: S, binding: SubjectBinding) -> Self {
        Self {
            service,
            arguments: DeliveryToolArguments::new(binding),
            lifecycle: Lifecycle::AwaitingInitialize,
            tool_surface: ToolSurface::LegacyDeliveryObserver,
            tool_invocations: 0,
            acceptance_evidence: None,
            acceptance_evidence_error: None,
        }
    }

    fn enable_acceptance_evidence(&mut self) -> io::Result<()> {
        self.acceptance_evidence = AcceptanceEvidence::from_process_environment()?;
        Ok(())
    }

    fn take_acceptance_evidence_error(&mut self) -> Option<io::Error> {
        self.acceptance_evidence_error.take()
    }

    fn close_acceptance_evidence(&mut self) -> io::Result<()> {
        if let Some(evidence) = self.acceptance_evidence.as_mut() {
            evidence.close()?;
        }
        Ok(())
    }

    /// Handles one decoded JSON-RPC message. Notifications return no value.
    #[must_use]
    pub fn handle(&mut self, message: Value) -> Option<Value> {
        let Value::Object(mut object) = message else {
            return Some(protocol_error(Value::Null, -32600, "Invalid Request"));
        };
        let id = object.remove("id");
        if id.as_ref().is_some_and(|id| !valid_request_id(id)) {
            return Some(protocol_error(Value::Null, -32600, "Invalid Request"));
        }
        if object
            .remove("jsonrpc")
            .and_then(|value| value.as_str().map(str::to_owned))
            .as_deref()
            != Some("2.0")
        {
            return id.map(|id| protocol_error(id, -32600, "Invalid Request"));
        }
        let Some(method) = object
            .remove("method")
            .and_then(|value| value.as_str().map(str::to_owned))
        else {
            return id.map(|id| protocol_error(id, -32600, "Invalid Request"));
        };
        let params = object.remove("params");

        if id.is_none() {
            if method == "notifications/initialized"
                && self.lifecycle == Lifecycle::AwaitingInitialized
            {
                self.lifecycle = Lifecycle::Ready;
            }
            return None;
        }
        let id = id?;
        let protocol = match request_protocol(params.as_ref()) {
            Ok(protocol) => protocol,
            Err(RequestProtocolError::InvalidMetadata) => {
                return Some(protocol_error(id, -32602, "Invalid request metadata"));
            }
            Err(RequestProtocolError::Unsupported(requested)) => {
                return Some(unsupported_protocol_error(id, &requested));
            }
        };
        match method.as_str() {
            "server/discover" => Some(self.discover(id, params.as_ref(), protocol)),
            "initialize" if protocol == RequestProtocol::Stateless => {
                Some(protocol_error(id, -32601, "Method not found"))
            }
            "initialize" => Some(self.initialize(id, params.as_ref())),
            "ping" if protocol == RequestProtocol::Legacy => Some(success(id, json!({}))),
            "tools/list" => Some(self.list_tools(id, params.as_ref(), protocol)),
            "tools/call" => Some(self.call_tool(id, params.as_ref(), protocol)),
            _ => Some(protocol_error(id, -32601, "Method not found")),
        }
    }

    fn discover(&self, id: Value, params: Option<&Value>, protocol: RequestProtocol) -> Value {
        if protocol != RequestProtocol::Stateless || !metadata_only_object_or_absent(params) {
            return protocol_error(id, -32602, "Invalid server/discover params");
        }
        success(
            id,
            json!({
                "resultType": "complete",
                "supportedVersions": [MCP_STATELESS_PROTOCOL_VERSION],
                "capabilities": {"tools": {}},
                "instructions": self.tool_surface.instructions(),
                "ttlMs": 0,
                "cacheScope": "private",
                "_meta": server_result_meta()
            }),
        )
    }

    fn initialize(&mut self, id: Value, params: Option<&Value>) -> Value {
        if self.lifecycle != Lifecycle::AwaitingInitialize {
            return protocol_error(id, -32600, "Already initialized");
        }
        let Some(params) = params.and_then(Value::as_object) else {
            return protocol_error(id, -32602, "Invalid initialize params");
        };
        if params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
            || !params.get("capabilities").is_some_and(Value::is_object)
            || !params.get("clientInfo").is_some_and(Value::is_object)
        {
            return protocol_error(id, -32602, "Invalid initialize params");
        }
        self.lifecycle = Lifecycle::AwaitingInitialized;
        success(
            id,
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {
                    "name": "latticed",
                    "title": "LATTICE DevOS",
                    "version": "1.0.0"
                },
                "instructions": self.tool_surface.instructions()
            }),
        )
    }

    fn list_tools(&self, id: Value, params: Option<&Value>, protocol: RequestProtocol) -> Value {
        if protocol == RequestProtocol::Legacy && self.lifecycle != Lifecycle::Ready {
            return protocol_error(id, -32002, "Server not initialized");
        }
        if !metadata_only_object_or_absent(params) {
            return protocol_error(id, -32602, "Invalid tools/list params");
        }
        let mut result = json!({"tools": tool_catalog(protocol, self.tool_surface)});
        if protocol == RequestProtocol::Stateless {
            let result = result
                .as_object_mut()
                .expect("tool list result is an object");
            result.insert(
                "resultType".to_owned(),
                Value::String("complete".to_owned()),
            );
            result.insert("ttlMs".to_owned(), Value::from(0));
            result.insert("cacheScope".to_owned(), Value::String("private".to_owned()));
            result.insert("_meta".to_owned(), server_result_meta());
        }
        success(id, result)
    }

    fn call_tool(&mut self, id: Value, params: Option<&Value>, protocol: RequestProtocol) -> Value {
        if protocol == RequestProtocol::Legacy && self.lifecycle != Lifecycle::Ready {
            return protocol_error(id, -32002, "Server not initialized");
        }
        let Some(params) = params.and_then(Value::as_object) else {
            return protocol_error(id, -32602, "Invalid tools/call params");
        };
        if params
            .keys()
            .any(|key| key != "name" && key != "arguments" && key != "_meta")
            || !metadata_object_or_absent(params.get("_meta"))
        {
            return protocol_error(id, -32602, "Invalid tools/call params");
        }
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return protocol_error(id, -32602, "Invalid tools/call params");
        };
        if !self.tool_surface.allows_task_control()
            && matches!(name, TASK_SUBMIT_TOOL | TASK_STATUS_TOOL)
        {
            return protocol_error(id, -32602, "Unknown tool");
        }
        let operation = match name {
            DELIVERY_RUN_TOOL if empty_object_or_absent(params.get("arguments")) => {
                ToolOperation::DeliveryRun
            }
            DELIVERY_STATUS_TOOL if empty_object_or_absent(params.get("arguments")) => {
                ToolOperation::DeliveryStatus
            }
            DELIVERY_RUN_TOOL | DELIVERY_STATUS_TOOL => {
                return protocol_error(id, -32602, "Tool accepts no arguments");
            }
            TASK_SUBMIT_TOOL => {
                let Some(arguments) = TaskSubmitArguments::from_value(params.get("arguments"))
                else {
                    return protocol_error(id, -32602, "Invalid task submit arguments");
                };
                ToolOperation::TaskSubmit(arguments)
            }
            TASK_STATUS_TOOL => {
                let Some(arguments) = TaskStatusArguments::from_value(params.get("arguments"))
                else {
                    return protocol_error(id, -32602, "Invalid task status arguments");
                };
                ToolOperation::TaskStatus(arguments)
            }
            _ => return protocol_error(id, -32602, "Unknown tool"),
        };
        if self.tool_invocations >= MAX_TOOL_INVOCATIONS_PER_SESSION {
            return protocol_error(id, -32029, "Tool invocation limit exceeded");
        }
        self.tool_invocations += 1;
        if let Some(evidence) = self.acceptance_evidence.as_mut()
            && let Err(error) = evidence.record_dispatch(name, &id)
        {
            self.acceptance_evidence_error = Some(error);
            return protocol_error(id, -32603, "Acceptance evidence rejected");
        }
        let result = match operation {
            ToolOperation::DeliveryRun if !self.tool_surface.allows_delivery_run() => {
                Err(ToolExecutionError::new(LEGACY_DELIVERY_RUN_DISABLED))
            }
            ToolOperation::DeliveryRun => self.service.run(&self.arguments),
            ToolOperation::DeliveryStatus => self.service.status(&self.arguments),
            ToolOperation::TaskSubmit(arguments) => {
                closed_task_public_status(self.service.task_submit(&arguments))
            }
            ToolOperation::TaskStatus(arguments) => {
                closed_task_public_status(self.service.task_status(&arguments))
            }
        };
        let mut result = tool_result(result);
        if protocol == RequestProtocol::Stateless {
            let result = result.as_object_mut().expect("tool result is an object");
            result.insert(
                "resultType".to_owned(),
                Value::String("complete".to_owned()),
            );
            result.insert("_meta".to_owned(), server_result_meta());
        }
        success(id, result)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ToolOperation {
    DeliveryRun,
    DeliveryStatus,
    TaskSubmit(TaskSubmitArguments),
    TaskStatus(TaskStatusArguments),
}

#[derive(Debug, Eq, PartialEq)]
enum StdioFrame {
    EndOfStream,
    Complete(Vec<u8>),
    Oversized,
    Unterminated,
}

/// Serves newline-delimited MCP JSON-RPC over the supplied streams.
///
/// # Errors
///
/// Returns only transport read/write errors. Protocol and parse errors are
/// written as JSON-RPC responses.
pub fn serve<S: DeliveryToolService, R: BufRead, W: Write>(
    service: S,
    binding: SubjectBinding,
    reader: R,
    writer: W,
) -> io::Result<()> {
    let mut server = McpServer::new(service, binding);
    server.enable_acceptance_evidence()?;
    serve_server(server, reader, writer)
}

/// Serves the legacy read-only delivery observer over the supplied streams.
///
/// # Errors
///
/// Returns only transport read/write errors. Protocol and parse errors are
/// written as JSON-RPC responses.
pub fn serve_legacy_delivery_observer<S: DeliveryToolService, R: BufRead, W: Write>(
    service: S,
    binding: SubjectBinding,
    reader: R,
    writer: W,
) -> io::Result<()> {
    let mut server = McpServer::new_legacy_delivery_observer(service, binding);
    server.enable_acceptance_evidence()?;
    serve_server(server, reader, writer)
}

fn serve_server<S: DeliveryToolService, R: BufRead, W: Write>(
    mut server: McpServer<S>,
    mut reader: R,
    mut writer: W,
) -> io::Result<()> {
    loop {
        let response = match read_bounded_frame(&mut reader)? {
            StdioFrame::EndOfStream => {
                server.close_acceptance_evidence()?;
                return Ok(());
            }
            StdioFrame::Oversized => Some(protocol_error(Value::Null, -32600, "Message too large")),
            StdioFrame::Unterminated => {
                Some(protocol_error(Value::Null, -32600, "Unterminated message"))
            }
            StdioFrame::Complete(buffer) => match serde_json::from_slice::<Value>(&buffer) {
                Ok(message) => server.handle(message),
                Err(_) => Some(protocol_error(Value::Null, -32700, "Parse error")),
            },
        };
        if let Some(error) = server.take_acceptance_evidence_error() {
            return Err(error);
        }
        if let Some(response) = response {
            serde_json::to_writer(&mut writer, &response)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }
}

fn read_bounded_frame<R: BufRead>(reader: &mut R) -> io::Result<StdioFrame> {
    let mut buffer = Vec::new();
    let mut oversized = false;
    let mut saw_bytes = false;

    loop {
        let (consumed, terminated) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                return Ok(if !saw_bytes {
                    StdioFrame::EndOfStream
                } else if oversized {
                    StdioFrame::Oversized
                } else {
                    StdioFrame::Unterminated
                });
            }

            saw_bytes = true;
            let newline = available.iter().position(|byte| *byte == b'\n');
            let consumed = newline.map_or(available.len(), |position| position + 1);
            if !oversized {
                let remaining = MAX_STDIO_MESSAGE_BYTES.saturating_sub(buffer.len());
                if consumed <= remaining {
                    buffer.extend_from_slice(&available[..consumed]);
                } else {
                    buffer.extend_from_slice(&available[..remaining]);
                    oversized = true;
                }
            }
            (consumed, newline.is_some())
        };

        reader.consume(consumed);
        if terminated {
            return Ok(if oversized {
                StdioFrame::Oversized
            } else {
                StdioFrame::Complete(buffer)
            });
        }
    }
}

fn delivery_arguments_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false
    })
}

fn task_submit_arguments_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "client_request_id": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_CLIENT_REQUEST_ID_BYTES,
                "pattern": "^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$"
            },
            "intent": {
                "type": "string",
                "enum": [CONTROLLED_CODEX_CANARY_INTENT]
            }
        },
        "required": ["client_request_id", "intent"],
        "additionalProperties": false
    })
}

fn task_status_arguments_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "task_ref": {
                "type": "string",
                "minLength": 64,
                "maxLength": 64,
                "pattern": "^[0-9a-f]{64}$"
            }
        },
        "required": ["task_ref"],
        "additionalProperties": false
    })
}

fn task_public_status_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schema_version": {
                "type": "string",
                "enum": [TASK_PUBLIC_STATUS_SCHEMA_VERSION]
            },
            "status": {
                "type": "string",
                "enum": TASK_PUBLIC_STATUS_VALUES
            },
            "task_state": {
                "type": "string",
                "enum": TASK_PUBLIC_STATE_VALUES
            },
            "task_ref": {
                "type": "string",
                "minLength": 64,
                "maxLength": 64,
                "pattern": "^[0-9a-f]{64}$"
            },
            "ledger_head_digest": {
                "type": "string",
                "minLength": 64,
                "maxLength": 64,
                "pattern": "^[0-9a-f]{64}$"
            },
            "result_digest": {
                "anyOf": [
                    {
                        "type": "string",
                        "minLength": 64,
                        "maxLength": 64,
                        "pattern": "^[0-9a-f]{64}$"
                    },
                    {"type": "null"}
                ]
            }
        },
        "required": [
            "schema_version",
            "status",
            "task_state",
            "task_ref",
            "ledger_head_digest",
            "result_digest"
        ],
        "additionalProperties": false
    })
}

fn tool_catalog(protocol: RequestProtocol, surface: ToolSurface) -> Value {
    let delivery_run_description = if surface.allows_delivery_run() {
        "Runs the one LATTICE-owned delivery profile using server configuration."
    } else {
        "Legacy name retained for compatibility; mutation requires the canonical latticed entrypoint."
    };
    let mut tools = vec![
        json!({
            "name": DELIVERY_RUN_TOOL,
            "title": "Run LATTICE delivery",
            "description": delivery_run_description,
            "inputSchema": delivery_arguments_schema()
        }),
        json!({
            "name": DELIVERY_STATUS_TOOL,
            "title": "Read LATTICE delivery status",
            "description": "Reads the durable status for the one LATTICE-owned delivery profile.",
            "inputSchema": delivery_arguments_schema()
        }),
    ];
    if surface.allows_task_control() {
        tools.extend([
            json!({
                "name": TASK_SUBMIT_TOOL,
                "title": "Submit a bounded LATTICE task",
                "description": "Submits the one approved high-level intent through the existing LATTICE service.",
                "inputSchema": task_submit_arguments_schema(),
                "outputSchema": task_public_status_schema()
            }),
            json!({
                "name": TASK_STATUS_TOOL,
                "title": "Read bounded LATTICE task status",
                "description": "Reads durable status for one validated LATTICE task reference.",
                "inputSchema": task_status_arguments_schema(),
                "outputSchema": task_public_status_schema()
            }),
        ]);
    }
    if protocol == RequestProtocol::Stateless {
        tools[0]["annotations"] = json!({
            "readOnlyHint": false,
            "destructiveHint": true,
            "idempotentHint": false,
            "openWorldHint": false
        });
        tools[1]["annotations"] = json!({
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        });
        if surface.allows_task_control() {
            tools[2]["annotations"] = json!({
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": true,
                "openWorldHint": false
            });
            tools[3]["annotations"] = json!({
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            });
        }
    }
    Value::Array(tools)
}

fn server_result_meta() -> Value {
    json!({
        META_SERVER_INFO: {
            "name": "latticed",
            "title": "LATTICE DevOS",
            "version": "1.0.0"
        }
    })
}

fn request_protocol(params: Option<&Value>) -> Result<RequestProtocol, RequestProtocolError> {
    let Some(params) = params.and_then(Value::as_object) else {
        return Ok(RequestProtocol::Legacy);
    };
    let Some(metadata) = params.get("_meta") else {
        return Ok(RequestProtocol::Legacy);
    };
    let Some(metadata) = metadata.as_object() else {
        return Err(RequestProtocolError::InvalidMetadata);
    };
    let has_modern_metadata = [
        META_PROTOCOL_VERSION,
        META_CLIENT_INFO,
        META_CLIENT_CAPABILITIES,
        META_LOG_LEVEL,
    ]
    .iter()
    .any(|key| metadata.contains_key(*key));
    if !has_modern_metadata {
        return Ok(RequestProtocol::Legacy);
    }
    let Some(version) = metadata.get(META_PROTOCOL_VERSION) else {
        return Err(RequestProtocolError::InvalidMetadata);
    };
    let Some(version) = version
        .as_str()
        .filter(|version| valid_protocol_version(version))
    else {
        return Err(RequestProtocolError::InvalidMetadata);
    };
    if version != MCP_STATELESS_PROTOCOL_VERSION {
        return Err(RequestProtocolError::Unsupported(version.to_owned()));
    }
    if !metadata
        .get(META_CLIENT_CAPABILITIES)
        .is_some_and(valid_client_capabilities)
        || metadata
            .get(META_CLIENT_INFO)
            .is_some_and(|value| !valid_implementation(value))
        || metadata
            .get(META_LOG_LEVEL)
            .is_some_and(|value| !valid_logging_level(value))
        || metadata
            .get("progressToken")
            .is_some_and(|value| !value.is_string() && !value.is_number())
    {
        return Err(RequestProtocolError::InvalidMetadata);
    }
    Ok(RequestProtocol::Stateless)
}

fn valid_protocol_version(version: &str) -> bool {
    version.len() == 10
        && version
            .bytes()
            .enumerate()
            .all(|(index, byte)| match index {
                4 | 7 => byte == b'-',
                _ => byte.is_ascii_digit(),
            })
}

fn valid_implementation(value: &Value) -> bool {
    value.as_object().is_some_and(|implementation| {
        ["name", "version"]
            .iter()
            .all(|field| implementation.get(*field).is_some_and(Value::is_string))
            && ["title", "description", "websiteUrl"]
                .iter()
                .all(|field| implementation.get(*field).is_none_or(Value::is_string))
            && implementation.get("icons").is_none_or(|icons| {
                icons
                    .as_array()
                    .is_some_and(|icons| icons.iter().all(valid_icon))
            })
    })
}

fn valid_icon(value: &Value) -> bool {
    value.as_object().is_some_and(|icon| {
        icon.get("src").is_some_and(Value::is_string)
            && icon.get("mimeType").is_none_or(Value::is_string)
            && icon.get("sizes").is_none_or(|sizes| {
                sizes
                    .as_array()
                    .is_some_and(|sizes| sizes.iter().all(Value::is_string))
            })
            && icon.get("theme").is_none_or(|theme| {
                theme
                    .as_str()
                    .is_some_and(|theme| matches!(theme, "light" | "dark"))
            })
    })
}

fn valid_client_capabilities(value: &Value) -> bool {
    value.as_object().is_some_and(|capabilities| {
        capabilities
            .get("experimental")
            .is_none_or(object_values_are_objects)
            && capabilities.get("roots").is_none_or(Value::is_object)
            && capabilities
                .get("sampling")
                .is_none_or(|sampling| object_with_object_fields(sampling, &["context", "tools"]))
            && capabilities
                .get("elicitation")
                .is_none_or(|elicitation| object_with_object_fields(elicitation, &["form", "url"]))
            && capabilities.get("extensions").is_none_or(valid_extensions)
    })
}

fn object_values_are_objects(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.values().all(Value::is_object))
}

fn valid_extensions(value: &Value) -> bool {
    value.as_object().is_some_and(|extensions| {
        extensions
            .iter()
            .all(|(key, value)| valid_prefixed_meta_key(key) && value.is_object())
    })
}

fn valid_prefixed_meta_key(key: &str) -> bool {
    let Some((prefix, name)) = key.split_once('/') else {
        return false;
    };
    !prefix.is_empty()
        && !name.contains('/')
        && prefix.split('.').all(valid_meta_prefix_label)
        && valid_meta_name(name)
}

fn valid_meta_prefix_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
}

fn valid_meta_name(name: &str) -> bool {
    if name.is_empty() {
        return true;
    }
    let bytes = name.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.'))
}

fn object_with_object_fields(value: &Value, fields: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        fields
            .iter()
            .all(|field| object.get(*field).is_none_or(Value::is_object))
    })
}

fn valid_logging_level(value: &Value) -> bool {
    value.as_str().is_some_and(|level| {
        matches!(
            level,
            "debug" | "info" | "notice" | "warning" | "error" | "critical" | "alert" | "emergency"
        )
    })
}

fn valid_client_request_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_CLIENT_REQUEST_ID_BYTES
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'_' | b':' | b'-'))
}

fn valid_task_ref(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn closed_task_public_status(
    result: Result<Value, ToolExecutionError>,
) -> Result<Value, ToolExecutionError> {
    result.and_then(|value| {
        TaskPublicStatus::from_value(&value)
            .map(TaskPublicStatus::into_value)
            .ok_or_else(|| ToolExecutionError::new("LATTICE_TASK_PUBLIC_STATUS_REJECTED"))
    })
}

fn empty_object_or_absent(value: Option<&Value>) -> bool {
    value.is_none_or(|value| value.as_object().is_some_and(Map::is_empty))
}

fn metadata_object_or_absent(value: Option<&Value>) -> bool {
    value.is_none_or(Value::is_object)
}

fn metadata_only_object_or_absent(value: Option<&Value>) -> bool {
    value.is_none_or(|value| {
        value.as_object().is_some_and(|object| {
            object.keys().all(|key| key == "_meta")
                && metadata_object_or_absent(object.get("_meta"))
        })
    })
}

fn valid_request_id(value: &Value) -> bool {
    match value {
        Value::String(_) => true,
        Value::Number(number) => number.is_i64() || number.is_u64(),
        _ => false,
    }
}

fn tool_result(result: Result<Value, ToolExecutionError>) -> Value {
    match result {
        Ok(value) => json!({
            "content": [{"type": "text", "text": value.to_string()}],
            "structuredContent": value,
            "isError": false
        }),
        Err(error) => {
            let value = json!({"status": "ERROR", "code": error.code()});
            json!({
                "content": [{"type": "text", "text": value.to_string()}],
                "structuredContent": value,
                "isError": true
            })
        }
    }
}

fn success(id: Value, result: Value) -> Value {
    let mut response = Map::new();
    response.insert("jsonrpc".to_owned(), Value::String("2.0".to_owned()));
    response.insert("id".to_owned(), id);
    response.insert("result".to_owned(), result);
    Value::Object(response)
}

fn protocol_error(id: Value, code: i32, message: &'static str) -> Value {
    let mut error = Map::new();
    error.insert("code".to_owned(), Value::from(code));
    error.insert("message".to_owned(), Value::String(message.to_owned()));
    let mut response = Map::new();
    response.insert("jsonrpc".to_owned(), Value::String("2.0".to_owned()));
    response.insert("id".to_owned(), id);
    response.insert("error".to_owned(), Value::Object(error));
    Value::Object(response)
}

fn unsupported_protocol_error(id: Value, requested: &str) -> Value {
    let mut error = Map::new();
    error.insert("code".to_owned(), Value::from(-32022));
    error.insert(
        "message".to_owned(),
        Value::String("Unsupported protocol version".to_owned()),
    );
    error.insert(
        "data".to_owned(),
        json!({
            "supported": [MCP_STATELESS_PROTOCOL_VERSION, MCP_PROTOCOL_VERSION],
            "requested": requested
        }),
    );
    let mut response = Map::new();
    response.insert("jsonrpc".to_owned(), Value::String("2.0".to_owned()));
    response.insert("id".to_owned(), id);
    response.insert("error".to_owned(), Value::Object(error));
    Value::Object(response)
}

#[cfg(test)]
mod acceptance_evidence_tests {
    use super::{ACCEPTANCE_EVIDENCE_SCHEMA, AcceptanceEvidence};
    use serde_json::{Value, json};
    use std::fs::File;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fresh_sink(label: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "lattice-mcp-acceptance-{label}-{}-{unique}.jsonl",
            std::process::id()
        ));
        File::create(&path).expect("create fresh acceptance sink");
        path
    }

    #[test]
    fn dispatch_evidence_is_a_durable_open_dispatch_close_hash_chain() {
        let path = fresh_sink("chain");
        let session_id = "0123456789abcdef0123456789abcdef".to_owned();
        let safe_config_sha256 = "ab".repeat(32);
        let mut evidence =
            AcceptanceEvidence::open(&path, session_id.clone(), safe_config_sha256.clone())
                .expect("open acceptance evidence");
        evidence
            .record_dispatch("lattice_task_status", &json!(17))
            .expect("record accepted dispatch");
        evidence.close().expect("close acceptance evidence");
        drop(evidence);

        let text = std::fs::read_to_string(&path).expect("read acceptance evidence");
        assert!(text.ends_with('\n'));
        let records = text
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("valid JSONL record"))
            .collect::<Vec<_>>();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0]["schema"], ACCEPTANCE_EVIDENCE_SCHEMA);
        assert_eq!(records[0]["record_type"], "SESSION_OPEN");
        assert_eq!(records[1]["record_type"], "DISPATCH_ACCEPTED");
        assert_eq!(records[1]["tool_name"], "lattice_task_status");
        assert_eq!(records[2]["record_type"], "SESSION_CLOSED");
        assert_eq!(records[2]["dispatch_accepted_count"], 1);
        assert_eq!(records[0]["previous_event_sha256"], "0".repeat(64));
        assert_eq!(
            records[1]["previous_event_sha256"],
            records[0]["event_sha256"]
        );
        assert_eq!(
            records[2]["previous_event_sha256"],
            records[1]["event_sha256"]
        );
        assert_eq!(records[2]["session_id"], session_id);
        assert_eq!(records[2]["safe_config_sha256"], safe_config_sha256);
        std::fs::remove_file(path).expect("remove test sink");
    }

    #[test]
    fn dispatch_evidence_rejects_nonfresh_or_noncanonical_configuration() {
        let nonfresh_path = fresh_sink("nonfresh");
        std::fs::write(&nonfresh_path, b"existing\n").expect("seed nonfresh sink");
        let nonfresh = AcceptanceEvidence::open(
            &nonfresh_path,
            "0123456789abcdef0123456789abcdef".to_owned(),
            "cd".repeat(32),
        );
        assert!(nonfresh.is_err());
        std::fs::remove_file(nonfresh_path).expect("remove nonfresh sink");

        let uppercase_path = fresh_sink("uppercase");
        let uppercase = AcceptanceEvidence::open(
            &uppercase_path,
            "0123456789abcdef0123456789abcdeF".to_owned(),
            "ef".repeat(32),
        );
        assert!(uppercase.is_err());
        std::fs::remove_file(uppercase_path).expect("remove uppercase sink");
    }
}
