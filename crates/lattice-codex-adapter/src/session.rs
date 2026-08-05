//! Pure state machine for one Codex app-server initialize/thread/turn session.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{AppServerProtocol, ProtocolError, TurnOutcome};

const INITIALIZE_RESPONSE_ID: i64 = 0;
const THREAD_START_RESPONSE_ID: i64 = 1;
const TURN_START_RESPONSE_ID: i64 = 2;

/// Server identity returned by the required `initialize` response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitializeEvidence {
    pub user_agent: String,
    pub platform_family: String,
    pub platform_os: String,
    pub codex_home: PathBuf,
}

/// The first still-missing piece of a session's canonical lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionPhase {
    Initialize,
    ThreadStart,
    TurnStart,
    Terminal,
    Complete,
}

/// One client request whose response may be admitted by the session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionRequest {
    Initialize,
    ThreadStart,
    TurnStart,
}

impl SessionRequest {
    const fn index(self) -> usize {
        match self {
            Self::Initialize => 0,
            Self::ThreadStart => 1,
            Self::TurnStart => 2,
        }
    }
}

/// Fail-closed errors emitted while correlating one app-server session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionError {
    MalformedJson,
    MalformedMessage(&'static str),
    NonIntegerResponseId,
    UnexpectedResponseId(i64),
    DuplicateResponseId(i64),
    ResponseBeforeRequest(i64),
    DuplicateRequest(SessionRequest),
    Rpc {
        request_id: i64,
        code: i64,
        message: String,
    },
    MalformedResponse {
        request_id: i64,
        field: &'static str,
    },
    CodexHomeNotAbsolute(String),
    DuplicateTerminal,
    TerminalBeforeTurnStart,
    Terminal(ProtocolError),
    UnexpectedEof(SessionPhase),
}

/// Correlates the fixed response ids and terminal notification for one run.
///
/// Responses and notifications may be supplied in any order. A terminal is
/// exposed only after initialize, thread, and turn evidence are all present
/// and [`AppServerProtocol`] has bound it to the exact thread and turn ids.
#[derive(Debug, Default)]
pub struct AppServerSession {
    initialize: Option<InitializeEvidence>,
    thread_id: Option<String>,
    turn_id: Option<String>,
    pending_terminal: Option<Value>,
    validated_terminal: Option<TurnOutcome>,
    seen_responses: [bool; 3],
    sent_requests: [bool; 3],
    completion_emitted: bool,
    failure: Option<SessionError>,
}

impl AppServerSession {
    /// Creates an empty session expecting response ids 0, 1, and 2.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Admits only the response corresponding to a request the client has sent.
    ///
    /// # Errors
    ///
    /// Returns a latched error if the same request is marked twice.
    pub fn mark_request_sent(&mut self, request: SessionRequest) -> Result<(), SessionError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let index = request.index();
        if self.sent_requests[index] {
            return self.fail(SessionError::DuplicateRequest(request));
        }
        self.sent_requests[index] = true;
        Ok(())
    }

    /// Accepts one decoded app-server message.
    ///
    /// The returned outcome is emitted exactly once, when every required
    /// lifecycle result and the matching terminal notification are present.
    ///
    /// # Errors
    ///
    /// Returns a typed, latched error for malformed or ambiguous evidence.
    /// Once failed, the session continues returning its first error.
    pub fn ingest(&mut self, message: Value) -> Result<Option<TurnOutcome>, SessionError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }

        let result = self.ingest_inner(message);
        if let Err(error) = &result {
            self.failure = Some(error.clone());
        }
        result
    }

    /// Decodes and accepts one JSONL payload without performing any I/O.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::MalformedJson`] when the line is not one JSON
    /// value, or any typed lifecycle error returned by [`Self::ingest`].
    pub fn ingest_json_line(&mut self, line: &str) -> Result<Option<TurnOutcome>, SessionError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }

        let Ok(message) = serde_json::from_str(line) else {
            return self.fail(SessionError::MalformedJson);
        };
        self.ingest(message)
    }

    /// Marks the input stream as closed and returns the proven terminal.
    ///
    /// # Errors
    ///
    /// Returns the first prior failure, or [`SessionError::UnexpectedEof`]
    /// when required evidence is still missing.
    pub fn finish_eof(&mut self) -> Result<TurnOutcome, SessionError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if let Some(outcome) = self.outcome().cloned() {
            return Ok(outcome);
        }

        self.fail(SessionError::UnexpectedEof(self.phase()))
    }

    /// Returns the validated initialize evidence, when received.
    #[must_use]
    pub fn initialize_evidence(&self) -> Option<&InitializeEvidence> {
        self.initialize.as_ref()
    }

    /// Returns the native thread id captured only from response id 1.
    #[must_use]
    pub fn thread_id(&self) -> Option<&str> {
        self.thread_id.as_deref()
    }

    /// Returns the native turn id captured only from response id 2.
    #[must_use]
    pub fn turn_id(&self) -> Option<&str> {
        self.turn_id.as_deref()
    }

    /// Returns the terminal only when the complete session is unambiguous.
    #[must_use]
    pub fn outcome(&self) -> Option<&TurnOutcome> {
        if self.initialize.is_some() && self.thread_id.is_some() && self.turn_id.is_some() {
            self.validated_terminal.as_ref()
        } else {
            None
        }
    }

    /// Returns the first lifecycle piece still required for completion.
    #[must_use]
    pub fn phase(&self) -> SessionPhase {
        if self.initialize.is_none() {
            SessionPhase::Initialize
        } else if self.thread_id.is_none() {
            SessionPhase::ThreadStart
        } else if self.turn_id.is_none() {
            SessionPhase::TurnStart
        } else if self.validated_terminal.is_none() {
            SessionPhase::Terminal
        } else {
            SessionPhase::Complete
        }
    }

    /// Returns the latched first failure, if any.
    #[must_use]
    pub fn failure(&self) -> Option<&SessionError> {
        self.failure.as_ref()
    }

    fn ingest_inner(&mut self, message: Value) -> Result<Option<TurnOutcome>, SessionError> {
        let object = message
            .as_object()
            .ok_or(SessionError::MalformedMessage("message must be an object"))?;

        if object.contains_key("id") {
            self.ingest_response(&message)?;
        } else {
            self.ingest_notification(message)?;
        }

        self.reconcile()
    }

    fn ingest_response(&mut self, message: &Value) -> Result<(), SessionError> {
        let object = message
            .as_object()
            .ok_or(SessionError::MalformedMessage("response must be an object"))?;
        if object.contains_key("method") {
            return Err(SessionError::MalformedMessage(
                "server request is not a response",
            ));
        }

        let request_id = object
            .get("id")
            .and_then(Value::as_i64)
            .ok_or(SessionError::NonIntegerResponseId)?;
        let response_index = match request_id {
            INITIALIZE_RESPONSE_ID => 0,
            THREAD_START_RESPONSE_ID => 1,
            TURN_START_RESPONSE_ID => 2,
            other => return Err(SessionError::UnexpectedResponseId(other)),
        };
        if !self.sent_requests[response_index] {
            return Err(SessionError::ResponseBeforeRequest(request_id));
        }
        if self.seen_responses[response_index] {
            return Err(SessionError::DuplicateResponseId(request_id));
        }
        self.seen_responses[response_index] = true;

        let result = object.get("result");
        let error = object.get("error");
        match (result, error) {
            (Some(_), Some(_)) | (None, None) => {
                return Err(SessionError::MalformedResponse {
                    request_id,
                    field: "exactly one of result or error",
                });
            }
            (None, Some(error)) => return Err(parse_rpc_error(request_id, error)?),
            (Some(result), None) => match request_id {
                INITIALIZE_RESPONSE_ID => {
                    self.initialize = Some(parse_initialize_result(result)?);
                }
                THREAD_START_RESPONSE_ID => {
                    self.thread_id = Some(parse_nested_id(
                        result,
                        THREAD_START_RESPONSE_ID,
                        "thread",
                        "result.thread.id",
                    )?);
                }
                TURN_START_RESPONSE_ID => {
                    self.turn_id = Some(parse_nested_id(
                        result,
                        TURN_START_RESPONSE_ID,
                        "turn",
                        "result.turn.id",
                    )?);
                }
                _ => unreachable!("request id was exhaustively checked"),
            },
        }

        Ok(())
    }

    fn ingest_notification(&mut self, message: Value) -> Result<(), SessionError> {
        let method =
            message
                .get("method")
                .and_then(Value::as_str)
                .ok_or(SessionError::MalformedMessage(
                    "notification method must be a string",
                ))?;

        if method == "turn/completed" {
            if !self.sent_requests[SessionRequest::TurnStart.index()] {
                return Err(SessionError::TerminalBeforeTurnStart);
            }
            if self.pending_terminal.is_some() || self.validated_terminal.is_some() {
                return Err(SessionError::DuplicateTerminal);
            }
            self.pending_terminal = Some(message);
        }

        Ok(())
    }

    fn reconcile(&mut self) -> Result<Option<TurnOutcome>, SessionError> {
        if self.validated_terminal.is_none()
            && let (Some(thread_id), Some(turn_id), Some(message)) = (
                self.thread_id.as_deref(),
                self.turn_id.as_deref(),
                self.pending_terminal.as_ref(),
            )
        {
            let outcome = AppServerProtocol::parse_turn_completed(message, thread_id, turn_id)
                .map_err(SessionError::Terminal)?
                .ok_or(SessionError::MalformedMessage(
                    "pending terminal was not turn/completed",
                ))?;
            self.validated_terminal = Some(outcome);
            self.pending_terminal = None;
        }

        if !self.completion_emitted
            && let Some(outcome) = self.outcome().cloned()
        {
            self.completion_emitted = true;
            return Ok(Some(outcome));
        }

        Ok(None)
    }

    fn fail<T>(&mut self, error: SessionError) -> Result<T, SessionError> {
        self.failure = Some(error.clone());
        Err(error)
    }
}

fn parse_initialize_result(result: &Value) -> Result<InitializeEvidence, SessionError> {
    let object = result.as_object().ok_or(SessionError::MalformedResponse {
        request_id: INITIALIZE_RESPONSE_ID,
        field: "result",
    })?;
    let user_agent = required_string(
        object.get("userAgent"),
        INITIALIZE_RESPONSE_ID,
        "result.userAgent",
    )?;
    let platform_family = required_string(
        object.get("platformFamily"),
        INITIALIZE_RESPONSE_ID,
        "result.platformFamily",
    )?;
    let platform_os = required_string(
        object.get("platformOs"),
        INITIALIZE_RESPONSE_ID,
        "result.platformOs",
    )?;
    let codex_home = required_string(
        object.get("codexHome"),
        INITIALIZE_RESPONSE_ID,
        "result.codexHome",
    )?;
    if !is_absolute_path(&codex_home) {
        return Err(SessionError::CodexHomeNotAbsolute(codex_home));
    }

    Ok(InitializeEvidence {
        user_agent,
        platform_family,
        platform_os,
        codex_home: PathBuf::from(codex_home),
    })
}

fn parse_nested_id(
    result: &Value,
    request_id: i64,
    container: &str,
    field: &'static str,
) -> Result<String, SessionError> {
    let value = result
        .as_object()
        .and_then(|object| object.get(container))
        .and_then(Value::as_object)
        .and_then(|object| object.get("id"));
    required_string(value, request_id, field)
}

fn required_string(
    value: Option<&Value>,
    request_id: i64,
    field: &'static str,
) -> Result<String, SessionError> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or(SessionError::MalformedResponse { request_id, field })
}

fn parse_rpc_error(request_id: i64, error: &Value) -> Result<SessionError, SessionError> {
    let object = error.as_object().ok_or(SessionError::MalformedResponse {
        request_id,
        field: "error",
    })?;
    let code =
        object
            .get("code")
            .and_then(Value::as_i64)
            .ok_or(SessionError::MalformedResponse {
                request_id,
                field: "error.code",
            })?;
    let message =
        object
            .get("message")
            .and_then(Value::as_str)
            .ok_or(SessionError::MalformedResponse {
                request_id,
                field: "error.message",
            })?;

    Ok(SessionError::Rpc {
        request_id,
        code,
        message: message.to_owned(),
    })
}

fn is_absolute_path(value: &str) -> bool {
    if Path::new(value).is_absolute() || value.starts_with('/') {
        return true;
    }

    let bytes = value.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
    {
        return true;
    }

    let Some(unc_tail) = value.strip_prefix(r"\\") else {
        return false;
    };
    let mut components = unc_tail
        .split(['\\', '/'])
        .filter(|component| !component.is_empty());
    components.next().is_some() && components.next().is_some()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::TurnStatus;

    fn initialize_response() -> Value {
        json!({
            "id": 0,
            "result": {
                "userAgent": "codex_cli_rs/0.144.6",
                "platformFamily": "windows",
                "platformOs": "windows",
                "codexHome": r"C:\lattice\codex-home"
            }
        })
    }

    fn thread_response() -> Value {
        json!({"id": 1, "result": {"thread": {"id": "thr_123"}}})
    }

    fn turn_response() -> Value {
        json!({"id": 2, "result": {"turn": {"id": "turn_456"}}})
    }

    fn terminal(status: &str) -> Value {
        let items = if status == "completed" {
            json!([
                {
                    "id": "tool_apply",
                    "type": "dynamicToolCall",
                    "tool": "exec",
                    "arguments": {},
                    "status": "completed",
                    "success": true
                },
                {
                    "id": "tool_verify",
                    "type": "dynamicToolCall",
                    "tool": "exec",
                    "arguments": {},
                    "status": "completed",
                    "success": true
                }
            ])
        } else {
            json!([])
        };
        let mut terminal = json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thr_123",
                "turn": {"id": "turn_456", "items": items, "status": status, "error": null}
            }
        });
        if status == "completed" {
            terminal["params"]["turn"]["itemsView"] = json!("full");
        }
        terminal
    }

    fn sent_session() -> AppServerSession {
        let mut session = AppServerSession::new();
        for request in [
            SessionRequest::Initialize,
            SessionRequest::ThreadStart,
            SessionRequest::TurnStart,
        ] {
            session
                .mark_request_sent(request)
                .expect("each lifecycle request is sent exactly once");
        }
        session
    }

    #[test]
    fn rejects_lifecycle_evidence_before_the_corresponding_request_is_sent() {
        let mut response = AppServerSession::new();
        assert_eq!(
            response.ingest(thread_response()),
            Err(SessionError::ResponseBeforeRequest(1))
        );

        let mut terminal_session = AppServerSession::new();
        assert_eq!(
            terminal_session.ingest(terminal("completed")),
            Err(SessionError::TerminalBeforeTurnStart)
        );
    }

    #[test]
    fn demultiplexes_responses_and_notifications_in_any_order() {
        let mut session = sent_session();

        assert_eq!(session.ingest(terminal("completed")), Ok(None));
        assert_eq!(session.ingest(turn_response()), Ok(None));
        assert_eq!(
            session.ingest(json!({"method": "item/completed", "params": {}})),
            Ok(None)
        );
        assert_eq!(session.ingest(initialize_response()), Ok(None));
        let outcome = session
            .ingest(thread_response())
            .expect("the final missing response should reconcile the session")
            .expect("completion is emitted once");

        assert_eq!(outcome.turn_id, "turn_456");
        assert_eq!(outcome.status, TurnStatus::Completed);
        assert_eq!(session.thread_id(), Some("thr_123"));
        assert_eq!(session.turn_id(), Some("turn_456"));
        assert_eq!(session.phase(), SessionPhase::Complete);
        assert_eq!(
            session
                .initialize_evidence()
                .expect("initialize evidence is retained")
                .platform_os,
            "windows"
        );

        assert_eq!(
            session.ingest(json!({"method": "item/completed", "params": {}})),
            Ok(None),
            "the terminal outcome must not be emitted twice"
        );
    }

    #[test]
    fn validates_all_initialize_strings_and_absolute_codex_home() {
        let mut missing_string = sent_session();
        let mut response = initialize_response();
        response["result"]["platformOs"] = Value::Null;
        assert_eq!(
            missing_string.ingest(response),
            Err(SessionError::MalformedResponse {
                request_id: 0,
                field: "result.platformOs"
            })
        );

        let mut relative_home = sent_session();
        let mut response = initialize_response();
        response["result"]["codexHome"] = json!(r"relative\codex-home");
        assert_eq!(
            relative_home.ingest(response),
            Err(SessionError::CodexHomeNotAbsolute(
                r"relative\codex-home".to_owned()
            ))
        );
    }

    #[test]
    fn rejects_non_integer_unknown_and_duplicate_response_ids() {
        let mut non_integer = sent_session();
        assert_eq!(
            non_integer.ingest(json!({"id": "1", "result": {}})),
            Err(SessionError::NonIntegerResponseId)
        );

        let mut unknown = sent_session();
        assert_eq!(
            unknown.ingest(json!({"id": 7, "result": {}})),
            Err(SessionError::UnexpectedResponseId(7))
        );

        let mut duplicate = sent_session();
        assert_eq!(duplicate.ingest(thread_response()), Ok(None));
        assert_eq!(
            duplicate.ingest(thread_response()),
            Err(SessionError::DuplicateResponseId(1))
        );
    }

    #[test]
    fn accepts_only_the_terminal_bound_to_captured_thread_and_turn() {
        let mut session = sent_session();
        assert_eq!(session.ingest(thread_response()), Ok(None));
        assert_eq!(session.ingest(turn_response()), Ok(None));

        let mut wrong_turn = terminal("completed");
        wrong_turn["params"]["turn"]["id"] = json!("other");
        assert_eq!(
            session.ingest(wrong_turn),
            Err(SessionError::Terminal(ProtocolError::UnexpectedTurn))
        );
    }

    #[test]
    fn preserves_failed_and_interrupted_terminal_states() {
        for (status, expected) in [
            ("failed", TurnStatus::Failed),
            ("interrupted", TurnStatus::Interrupted),
        ] {
            let mut session = sent_session();
            session
                .ingest(initialize_response())
                .expect("initialize response is valid");
            session
                .ingest(thread_response())
                .expect("thread response is valid");
            session
                .ingest(turn_response())
                .expect("turn response is valid");
            let outcome = session
                .ingest(terminal(status))
                .expect("terminal notification is valid")
                .expect("terminal completes the session");
            assert_eq!(outcome.status, expected);
        }
    }

    #[test]
    fn rpc_errors_and_malformed_json_are_typed_and_latched() {
        let mut rpc = sent_session();
        let expected = SessionError::Rpc {
            request_id: 1,
            code: -32_000,
            message: "thread start failed".to_owned(),
        };
        assert_eq!(
            rpc.ingest(json!({
                "id": 1,
                "error": {"code": -32000, "message": "thread start failed"}
            })),
            Err(expected.clone())
        );
        assert_eq!(rpc.ingest(thread_response()), Err(expected));

        let mut malformed = sent_session();
        assert_eq!(
            malformed.ingest_json_line("{not json}"),
            Err(SessionError::MalformedJson)
        );
        assert_eq!(
            malformed.ingest(initialize_response()),
            Err(SessionError::MalformedJson)
        );
    }

    #[test]
    fn eof_fails_closed_until_the_complete_terminal_is_proven() {
        let mut incomplete = sent_session();
        incomplete
            .ingest(initialize_response())
            .expect("initialize response is valid");
        incomplete
            .ingest(thread_response())
            .expect("thread response is valid");
        incomplete
            .ingest(turn_response())
            .expect("turn response is valid");
        assert_eq!(
            incomplete.finish_eof(),
            Err(SessionError::UnexpectedEof(SessionPhase::Terminal))
        );

        let mut complete = sent_session();
        complete
            .ingest(initialize_response())
            .expect("initialize response is valid");
        complete
            .ingest(thread_response())
            .expect("thread response is valid");
        complete
            .ingest(turn_response())
            .expect("turn response is valid");
        complete
            .ingest(terminal("completed"))
            .expect("terminal notification is valid");
        assert_eq!(
            complete
                .finish_eof()
                .expect("EOF after a proven terminal is unambiguous")
                .status,
            TurnStatus::Completed
        );
    }
}
