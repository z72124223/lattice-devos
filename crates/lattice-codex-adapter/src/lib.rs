//! Supervised Codex app-server adapter.

mod delivery;
mod identity;
mod process;
mod session;

pub use delivery::{CodexDeliveryAdapter, CodexDeliveryAdapterConfig};
pub use identity::{
    CodexIdentityError, CodexIdentityErrorKind, CodexIdentityEvidence, CodexIdentityExpectation,
    preflight_codex_identity,
};
pub use process::{
    AppServerRunConfig, AppServerRunError, AppServerRunErrorKind, AppServerRunEvidence,
    CODEX_HOME_OWNERSHIP_MARKER_BYTES, CODEX_HOME_OWNERSHIP_MARKER_NAME,
    PinnedCodexResourceDigests, PinnedCodexResources, run_codex_app_server,
    run_codex_app_server_until,
};
pub use session::{
    AppServerSession, InitializeEvidence, SessionError, SessionPhase, SessionRequest,
};

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

fn protected_environment_name(name: &OsStr) -> bool {
    let name = name.to_string_lossy().to_ascii_uppercase();
    matches!(
        name.as_str(),
        "DATABASE_URL"
            | "PGPASSWORD"
            | "PGPASSFILE"
            | "GIT_ASKPASS"
            | "SSH_ASKPASS"
            | "LATTICE_TASK019_PASSWORD"
            | "API_KEY"
            | "TOKEN"
            | "SECRET"
            | "PASSWORD"
    ) || name.starts_with("CODEX_")
        || name.starts_with("OPENAI_")
        || name.starts_with("AZURE_OPENAI_")
        || matches!(
            name.as_str(),
            "HTTP_PROXY" | "HTTPS_PROXY" | "ALL_PROXY" | "NO_PROXY"
        )
        || [
            "_PASSWORD",
            "_TOKEN",
            "_SECRET",
            "_API_KEY",
            "_PRIVATE_KEY",
            "_ACCESS_KEY",
            "_CREDENTIAL",
            "_CREDENTIALS",
            "_CONNECTION_STRING",
        ]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

pub(crate) fn scrub_protected_environment(command: &mut Command) {
    for name in [
        "DATABASE_URL",
        "PGPASSWORD",
        "PGPASSFILE",
        "GIT_ASKPASS",
        "SSH_ASKPASS",
        "LATTICE_TASK019_PASSWORD",
    ] {
        command.env_remove(name);
    }
    for (name, _) in std::env::vars_os() {
        if protected_environment_name(&name) {
            command.env_remove(name);
        }
    }
}

/// Stable terminal states emitted by `turn/completed`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TurnStatus {
    Completed,
    Failed,
    Interrupted,
}

/// Terminal evidence bound to one exact app-server turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnOutcome {
    pub turn_id: String,
    pub status: TurnStatus,
    pub error_message: Option<String>,
}

/// Fail-closed protocol parsing failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    UnexpectedThread,
    UnexpectedTurn,
    MalformedTerminal,
    IncompleteToolExecution,
}

/// Builds and validates the stable subset of the Codex app-server protocol.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppServerProtocol {
    client_name: String,
    client_version: String,
}

impl AppServerProtocol {
    /// Creates a protocol client identity used by initialization and metrics.
    #[must_use]
    pub fn new(client_name: impl Into<String>, client_version: impl Into<String>) -> Self {
        Self {
            client_name: client_name.into(),
            client_version: client_version.into(),
        }
    }

    /// Builds the required first request for one stdio connection.
    #[must_use]
    pub fn initialize_request(&self, id: u64) -> Value {
        json!({
            "method": "initialize",
            "id": id,
            "params": {
                "clientInfo": {
                    "name": self.client_name,
                    "title": "LATTICE DevOS",
                    "version": self.client_version
                }
            }
        })
    }

    /// Builds the required acknowledgement after initialization succeeds.
    #[must_use]
    pub fn initialized_notification(&self) -> Value {
        json!({"method": "initialized"})
    }

    /// Starts one non-interactive workspace-write thread in the bounded root.
    #[must_use]
    pub fn thread_start_request(&self, id: u64, working_directory: &Path) -> Value {
        let cwd = working_directory.to_string_lossy();
        json!({
            "method": "thread/start",
            "id": id,
            "params": {
                "cwd": cwd,
                "approvalPolicy": "never",
                "sandbox": "workspace-write",
                "serviceName": self.client_name
            }
        })
    }

    /// Starts one turn with network disabled and only the task root writable.
    #[must_use]
    pub fn turn_start_request(
        &self,
        id: u64,
        thread_id: &str,
        working_directory: &Path,
        prompt: &str,
    ) -> Value {
        let cwd = working_directory.to_string_lossy();
        json!({
            "method": "turn/start",
            "id": id,
            "params": {
                "threadId": thread_id,
                "input": [{"type": "text", "text": prompt}],
                "cwd": cwd,
                "approvalPolicy": "never",
                "sandboxPolicy": {
                    "type": "workspaceWrite",
                    "writableRoots": [cwd],
                    "networkAccess": false
                }
            }
        })
    }

    /// Parses only `turn/completed` and binds it to the expected turn.
    ///
    /// Non-terminal notifications return `Ok(None)`. A malformed terminal or
    /// terminal for another turn fails closed.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedTerminal`] when terminal evidence is
    /// incomplete, or an unexpected thread/turn error when it is not bound to
    /// the requested execution.
    pub fn parse_turn_completed(
        message: &Value,
        expected_thread_id: &str,
        expected_turn_id: &str,
    ) -> Result<Option<TurnOutcome>, ProtocolError> {
        Self::parse_turn_completed_with_items(message, expected_thread_id, expected_turn_id, &[])
    }

    /// Parses one terminal and validates it against the ordered
    /// `item/completed` evidence already bound to the same thread and turn.
    ///
    /// # Errors
    ///
    /// Returns a fail-closed protocol error when the terminal shape, binding,
    /// or completed delivery-tool sequence is incomplete or ambiguous.
    pub(crate) fn parse_turn_completed_with_items(
        message: &Value,
        expected_thread_id: &str,
        expected_turn_id: &str,
        completed_items: &[Value],
    ) -> Result<Option<TurnOutcome>, ProtocolError> {
        if message.get("method").and_then(Value::as_str) != Some("turn/completed") {
            return Ok(None);
        }

        let Some(params) = message.get("params") else {
            return Err(ProtocolError::MalformedTerminal);
        };
        let Some(thread_id) = params.get("threadId").and_then(Value::as_str) else {
            return Err(ProtocolError::MalformedTerminal);
        };
        if thread_id != expected_thread_id {
            return Err(ProtocolError::UnexpectedThread);
        }
        let Some(turn) = params.get("turn").and_then(Value::as_object) else {
            return Err(ProtocolError::MalformedTerminal);
        };
        let items = turn
            .get("items")
            .and_then(Value::as_array)
            .ok_or(ProtocolError::MalformedTerminal)?;
        let Some(turn_id) = turn.get("id").and_then(Value::as_str) else {
            return Err(ProtocolError::MalformedTerminal);
        };
        if turn_id != expected_turn_id {
            return Err(ProtocolError::UnexpectedTurn);
        }
        let status = match turn.get("status").and_then(Value::as_str) {
            Some("completed") => TurnStatus::Completed,
            Some("failed") => TurnStatus::Failed,
            Some("interrupted") => TurnStatus::Interrupted,
            _ => return Err(ProtocolError::MalformedTerminal),
        };
        let error = turn.get("error").filter(|error| !error.is_null());
        let error_message = match (status, error) {
            (_, None) => None,
            (TurnStatus::Completed | TurnStatus::Interrupted, Some(_)) => {
                return Err(ProtocolError::MalformedTerminal);
            }
            (TurnStatus::Failed, Some(error)) => {
                let message = error
                    .as_object()
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .ok_or(ProtocolError::MalformedTerminal)?;
                Some(message.to_owned())
            }
        };
        if status == TurnStatus::Completed {
            validate_completed_terminal_items(turn, items)?;
            validate_completed_tool_evidence(completed_items)?;
        }

        Ok(Some(TurnOutcome {
            turn_id: turn_id.to_owned(),
            status,
            error_message,
        }))
    }
}

const YIELDED_EXEC_MARKER: &str = "Script running with cell ID ";

fn validate_completed_terminal_items(
    turn: &serde_json::Map<String, Value>,
    items: &[Value],
) -> Result<(), ProtocolError> {
    match turn.get("itemsView").and_then(Value::as_str) {
        Some("summary")
            if items.len() == 1
                && items[0].get("type").and_then(Value::as_str) == Some("agentMessage") =>
        {
            Ok(())
        }
        Some("notLoaded") if items.is_empty() => Ok(()),
        _ => Err(ProtocolError::MalformedTerminal),
    }
}

fn validate_completed_tool_evidence(items: &[Value]) -> Result<(), ProtocolError> {
    let mut pending_cell = None;
    let mut completed_execs = 0_usize;
    let mut completed_commands = 0_usize;
    let mut seen_dynamic_items = BTreeSet::new();
    for item in items {
        let Some(object) = item.as_object() else {
            return Err(ProtocolError::MalformedTerminal);
        };
        let Some(item_type) = object.get("type").and_then(Value::as_str) else {
            return Err(ProtocolError::MalformedTerminal);
        };
        match item_type {
            "dynamicToolCall" => {
                if object.get("status").and_then(Value::as_str) != Some("completed")
                    || object.get("success").and_then(Value::as_bool) != Some(true)
                {
                    return Err(ProtocolError::IncompleteToolExecution);
                }
                let item_id = object
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|item_id| !item_id.is_empty())
                    .ok_or(ProtocolError::MalformedTerminal)?;
                if !seen_dynamic_items.insert(item_id.to_owned()) {
                    return Err(ProtocolError::IncompleteToolExecution);
                }
                let tool = object
                    .get("tool")
                    .and_then(Value::as_str)
                    .ok_or(ProtocolError::MalformedTerminal)?;
                let output = dynamic_tool_output(object)?;
                match tool {
                    "exec" => {
                        if pending_cell.is_some() || completed_commands != 0 || completed_execs >= 2
                        {
                            return Err(ProtocolError::IncompleteToolExecution);
                        }
                        completed_execs += 1;
                        let mut yielded_cells = yielded_cell_ids(&output)?;
                        if yielded_cells.len() > 1 {
                            return Err(ProtocolError::IncompleteToolExecution);
                        }
                        validate_nested_exit_evidence(&output, yielded_cells.is_empty())?;
                        pending_cell = yielded_cells.pop();
                    }
                    "wait" => {
                        let expected_cell = pending_cell
                            .as_deref()
                            .ok_or(ProtocolError::IncompleteToolExecution)?;
                        let (cell_id, terminate) = wait_arguments(object)?;
                        if cell_id != expected_cell {
                            return Err(ProtocolError::IncompleteToolExecution);
                        }
                        let yielded_cells = yielded_cell_ids(&output)?;
                        let completed = completed_wait_result(&output);
                        if (completed && !yielded_cells.is_empty())
                            || yielded_cells
                                .iter()
                                .any(|yielded_cell_id| yielded_cell_id != expected_cell)
                        {
                            return Err(ProtocolError::IncompleteToolExecution);
                        }
                        if terminate {
                            return Err(ProtocolError::IncompleteToolExecution);
                        }
                        validate_nested_exit_evidence(&output, completed)?;
                        if completed {
                            pending_cell = None;
                        }
                    }
                    _ => return Err(ProtocolError::IncompleteToolExecution),
                }
            }
            "commandExecution" => {
                if pending_cell.is_some() || completed_execs != 0 || completed_commands >= 2 {
                    return Err(ProtocolError::IncompleteToolExecution);
                }
                let item_id = validated_command_execution_id(object)?;
                if !seen_dynamic_items.insert(item_id.to_owned()) {
                    return Err(ProtocolError::IncompleteToolExecution);
                }
                completed_commands += 1;
            }
            "userMessage" | "agentMessage" | "reasoning" => {}
            _ => return Err(ProtocolError::IncompleteToolExecution),
        }
    }
    if pending_cell.is_none()
        && ((completed_execs == 2 && completed_commands == 0)
            || (completed_execs == 0 && completed_commands == 2))
    {
        Ok(())
    } else {
        Err(ProtocolError::IncompleteToolExecution)
    }
}

fn validated_command_execution_id(
    object: &serde_json::Map<String, Value>,
) -> Result<&str, ProtocolError> {
    if object.get("status").and_then(Value::as_str) != Some("completed")
        || object
            .get("command")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        || object
            .get("cwd")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        || object
            .get("commandActions")
            .and_then(Value::as_array)
            .is_none()
        || object
            .get("aggregatedOutput")
            .and_then(Value::as_str)
            .is_none()
        || object.get("exitCode").and_then(Value::as_i64) != Some(0)
        || object.contains_key("success")
        || object
            .get("content")
            .is_some_and(|content| !content.is_null())
        || object
            .get("contentItems")
            .is_some_and(|content| !content.is_null())
    {
        return Err(ProtocolError::IncompleteToolExecution);
    }
    object
        .get("id")
        .and_then(Value::as_str)
        .filter(|item_id| !item_id.is_empty())
        .ok_or(ProtocolError::MalformedTerminal)
}

fn validate_nested_exit_evidence(
    output: &str,
    require_success_marker: bool,
) -> Result<(), ProtocolError> {
    if output.contains("Script failed") || output.contains("Script error:") {
        return Err(ProtocolError::IncompleteToolExecution);
    }

    let mut marker_count = 0_usize;
    for marker in ["Exit code:", "Process exited with code"] {
        let mut remaining = output;
        while let Some(offset) = remaining.find(marker) {
            let tail = &remaining[offset + marker.len()..];
            let tail = tail.trim_start_matches([' ', '\t']);
            let digits = tail
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>();
            if digits.parse::<u32>() != Ok(0) {
                return Err(ProtocolError::IncompleteToolExecution);
            }
            marker_count += 1;
            remaining = tail.get(digits.len()..).unwrap_or_default();
        }
    }

    if require_success_marker && marker_count == 0 {
        Err(ProtocolError::IncompleteToolExecution)
    } else {
        Ok(())
    }
}

fn dynamic_tool_output(object: &serde_json::Map<String, Value>) -> Result<String, ProtocolError> {
    let Some(content_items) = object.get("contentItems") else {
        return Ok(String::new());
    };
    if content_items.is_null() {
        return Ok(String::new());
    }
    let content_items = content_items
        .as_array()
        .ok_or(ProtocolError::MalformedTerminal)?;
    let mut output = String::new();
    for content_item in content_items {
        let content_item = content_item
            .as_object()
            .ok_or(ProtocolError::MalformedTerminal)?;
        if content_item.get("type").and_then(Value::as_str) == Some("inputText") {
            let text = content_item
                .get("text")
                .and_then(Value::as_str)
                .ok_or(ProtocolError::MalformedTerminal)?;
            output.push_str(text);
            output.push('\n');
        }
    }
    Ok(output)
}

fn yielded_cell_ids(output: &str) -> Result<Vec<String>, ProtocolError> {
    let mut ids = Vec::new();
    let mut remaining = output;
    while let Some(offset) = remaining.find(YIELDED_EXEC_MARKER) {
        let tail = &remaining[offset + YIELDED_EXEC_MARKER.len()..];
        let cell_id = tail
            .chars()
            .take_while(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
            .collect::<String>();
        if cell_id.is_empty() {
            return Err(ProtocolError::IncompleteToolExecution);
        }
        let cell_id_length = cell_id.len();
        ids.push(cell_id);
        remaining = tail.get(cell_id_length..).unwrap_or_default();
    }
    Ok(ids)
}

fn wait_arguments(
    object: &serde_json::Map<String, Value>,
) -> Result<(String, bool), ProtocolError> {
    let arguments = object
        .get("arguments")
        .ok_or(ProtocolError::IncompleteToolExecution)?;
    let parsed;
    let arguments = match arguments {
        Value::Object(arguments) => arguments,
        Value::String(encoded) => {
            parsed = serde_json::from_str::<Value>(encoded)
                .map_err(|_| ProtocolError::IncompleteToolExecution)?;
            parsed
                .as_object()
                .ok_or(ProtocolError::IncompleteToolExecution)?
        }
        _ => return Err(ProtocolError::IncompleteToolExecution),
    };
    let cell_id = arguments
        .get("cell_id")
        .and_then(Value::as_str)
        .filter(|cell_id| !cell_id.is_empty())
        .ok_or(ProtocolError::IncompleteToolExecution)?
        .to_owned();
    let terminate = match arguments.get("terminate") {
        None => false,
        Some(Value::Bool(terminate)) => *terminate,
        Some(_) => return Err(ProtocolError::IncompleteToolExecution),
    };
    Ok((cell_id, terminate))
}

fn completed_wait_result(output: &str) -> bool {
    output.lines().any(|line| line.trim() == "Script completed")
        && output
            .lines()
            .any(|line| matches!(line.trim(), "Exit code: 0" | "Process exited with code 0"))
}

#[cfg(test)]
mod environment_tests {
    use super::*;

    #[test]
    fn database_and_ambient_credential_names_are_protected() {
        for name in [
            "LATTICE_TASK019_PASSWORD",
            "PGPASSWORD",
            "DATABASE_URL",
            "API_KEY",
            "TOKEN",
            "SECRET",
            "PASSWORD",
            "OPENAI_API_KEY",
            "GH_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "AZURE_CLIENT_SECRET",
        ] {
            assert!(protected_environment_name(OsStr::new(name)), "{name}");
        }
        for name in [
            "CODEX_HOME",
            "CODEX_SQLITE_HOME",
            "CODEX_PERMISSION_PROFILE",
            "CODEX_EXEC_SERVER_URL",
            "OPENAI_BASE_URL",
            "HTTPS_PROXY",
        ] {
            assert!(protected_environment_name(OsStr::new(name)), "{name}");
        }
        for name in ["PATH", "SystemRoot", "LATTICE_TASK019_PORT"] {
            assert!(!protected_environment_name(OsStr::new(name)), "{name}");
        }
    }

    #[test]
    fn scrub_marks_the_database_password_removed_from_a_child() {
        let mut command = Command::new("unused");
        command
            .env("LATTICE_TASK019_PASSWORD", "must-not-leak")
            .env("LATTICE_SAFE_VALUE", "retained");
        scrub_protected_environment(&mut command);

        let password = command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new("LATTICE_TASK019_PASSWORD"))
            .expect("password removal is explicit");
        let safe = command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new("LATTICE_SAFE_VALUE"))
            .expect("safe value remains explicit");
        assert!(password.1.is_none());
        assert_eq!(safe.1, Some(OsStr::new("retained")));
    }
}
