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
    CODEX_HOME_OWNERSHIP_MARKER_BYTES, CODEX_HOME_OWNERSHIP_MARKER_NAME, run_codex_app_server,
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
            validate_completed_tool_evidence(turn, items)?;
        }

        Ok(Some(TurnOutcome {
            turn_id: turn_id.to_owned(),
            status,
            error_message,
        }))
    }
}

const YIELDED_EXEC_MARKER: &str = "Script running with cell ID ";

fn validate_completed_tool_evidence(
    turn: &serde_json::Map<String, Value>,
    items: &[Value],
) -> Result<(), ProtocolError> {
    if let Some(items_view) = turn.get("itemsView")
        && items_view.as_str() != Some("full")
    {
        return Err(ProtocolError::IncompleteToolExecution);
    }

    let mut pending_cells = BTreeSet::new();
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
                    || object.get("success").and_then(Value::as_bool) == Some(false)
                {
                    return Err(ProtocolError::IncompleteToolExecution);
                }
                let tool = object
                    .get("tool")
                    .and_then(Value::as_str)
                    .ok_or(ProtocolError::MalformedTerminal)?;
                let output = dynamic_tool_output(object)?;
                for cell_id in yielded_cell_ids(&output)? {
                    pending_cells.insert(cell_id);
                }
                if tool == "wait"
                    && let Some((cell_id, terminate)) = wait_arguments(object)
                {
                    if terminate {
                        return Err(ProtocolError::IncompleteToolExecution);
                    }
                    if completed_wait_result(&output) {
                        pending_cells.remove(&cell_id);
                    }
                }
            }
            "commandExecution" | "mcpToolCall" | "fileChange"
                if object.get("status").and_then(Value::as_str) != Some("completed") =>
            {
                return Err(ProtocolError::IncompleteToolExecution);
            }
            _ => {}
        }
    }
    if pending_cells.is_empty() {
        Ok(())
    } else {
        Err(ProtocolError::IncompleteToolExecution)
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

fn wait_arguments(object: &serde_json::Map<String, Value>) -> Option<(String, bool)> {
    let arguments = object.get("arguments")?;
    let parsed;
    let arguments = match arguments {
        Value::Object(arguments) => arguments,
        Value::String(encoded) => {
            parsed = serde_json::from_str::<Value>(encoded).ok()?;
            parsed.as_object()?
        }
        _ => return None,
    };
    let cell_id = arguments.get("cell_id")?.as_str()?.to_owned();
    let terminate = arguments
        .get("terminate")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some((cell_id, terminate))
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
