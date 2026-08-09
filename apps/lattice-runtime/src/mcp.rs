//! Bounded MCP stdio surface for the canonical `latticed` entry.

use std::error::Error;
use std::fmt;
use std::io::{self, BufRead, Write};

use lattice_contracts::SubjectBinding;
use serde_json::{Map, Value, json};

/// Stable MCP protocol version implemented by this server.
pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
/// Sole delivery execution tool.
pub const DELIVERY_RUN_TOOL: &str = "lattice_delivery_run";
/// Sole delivery status tool.
pub const DELIVERY_STATUS_TOOL: &str = "lattice_delivery_status";

/// Maximum encoded bytes accepted for one newline-delimited stdio message.
pub const MAX_STDIO_MESSAGE_BYTES: usize = 65_536;
/// Maximum valid tool invocations accepted during one MCP server session.
pub const MAX_TOOL_INVOCATIONS_PER_SESSION: usize = 64;

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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Lifecycle {
    AwaitingInitialize,
    AwaitingInitialized,
    Ready,
}

/// Stateful MCP lifecycle and request dispatcher.
pub struct McpServer<S> {
    service: S,
    arguments: DeliveryToolArguments,
    lifecycle: Lifecycle,
    tool_invocations: usize,
}

impl<S: DeliveryToolService> McpServer<S> {
    /// Constructs an uninitialized server.
    #[must_use]
    pub const fn new(service: S, binding: SubjectBinding) -> Self {
        Self {
            service,
            arguments: DeliveryToolArguments::new(binding),
            lifecycle: Lifecycle::AwaitingInitialize,
            tool_invocations: 0,
        }
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
        match method.as_str() {
            "initialize" => Some(self.initialize(id, params.as_ref())),
            "ping" => Some(success(id, json!({}))),
            "tools/list" => Some(self.list_tools(id, params.as_ref())),
            "tools/call" => Some(self.call_tool(id, params.as_ref())),
            _ => Some(protocol_error(id, -32601, "Method not found")),
        }
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
                "instructions": "Two fixed zero-argument delivery tools. Task binding and execution configuration remain server-owned."
            }),
        )
    }

    fn list_tools(&self, id: Value, params: Option<&Value>) -> Value {
        if self.lifecycle != Lifecycle::Ready {
            return protocol_error(id, -32002, "Server not initialized");
        }
        if !metadata_only_object_or_absent(params) {
            return protocol_error(id, -32602, "Invalid tools/list params");
        }
        success(
            id,
            json!({
                "tools": [
                    {
                        "name": DELIVERY_RUN_TOOL,
                        "title": "Run LATTICE delivery",
                        "description": "Runs the one LATTICE-owned delivery profile using server configuration.",
                        "inputSchema": delivery_arguments_schema()
                    },
                    {
                        "name": DELIVERY_STATUS_TOOL,
                        "title": "Read LATTICE delivery status",
                        "description": "Reads the durable status for the one LATTICE-owned delivery profile.",
                        "inputSchema": delivery_arguments_schema()
                    }
                ]
            }),
        )
    }

    fn call_tool(&mut self, id: Value, params: Option<&Value>) -> Value {
        if self.lifecycle != Lifecycle::Ready {
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
        if !empty_object_or_absent(params.get("arguments")) {
            return protocol_error(id, -32602, "Tool accepts no arguments");
        }
        let operation = match name {
            DELIVERY_RUN_TOOL => DeliveryOperation::Run,
            DELIVERY_STATUS_TOOL => DeliveryOperation::Status,
            _ => return protocol_error(id, -32602, "Unknown tool"),
        };
        if self.tool_invocations >= MAX_TOOL_INVOCATIONS_PER_SESSION {
            return protocol_error(id, -32029, "Tool invocation limit exceeded");
        }
        self.tool_invocations += 1;
        let result = match operation {
            DeliveryOperation::Run => self.service.run(&self.arguments),
            DeliveryOperation::Status => self.service.status(&self.arguments),
        };
        success(id, tool_result(result))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeliveryOperation {
    Run,
    Status,
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
    mut reader: R,
    mut writer: W,
) -> io::Result<()> {
    let mut server = McpServer::new(service, binding);
    loop {
        let response = match read_bounded_frame(&mut reader)? {
            StdioFrame::EndOfStream => return Ok(()),
            StdioFrame::Oversized => Some(protocol_error(Value::Null, -32600, "Message too large")),
            StdioFrame::Unterminated => {
                Some(protocol_error(Value::Null, -32600, "Unterminated message"))
            }
            StdioFrame::Complete(buffer) => match serde_json::from_slice::<Value>(&buffer) {
                Ok(message) => server.handle(message),
                Err(_) => Some(protocol_error(Value::Null, -32700, "Parse error")),
            },
        };
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
