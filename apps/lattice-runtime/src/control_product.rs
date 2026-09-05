//! Closed MCP inputs for the product projection and native Codex observations.
use lattice_contracts::ContentDigest;
use lattice_postgres_store::{ControlProductCommand, PostgresControlProduct};
use lattice_task_ledger::task_submission_text_contains_secret;
use serde_json::{Map, Value, json};

const OBSERVATION_KINDS: &[&str] = &[
    "THREAD_BOUND",
    "DISPATCH_STARTED",
    "TURN_BOUND",
    "PROGRESS",
    "APPROVAL_REQUESTED",
    "APPROVAL_RESOLVED",
    "TURN_COMPLETED",
    "TURN_FAILED",
    "INTERRUPTED",
    "ARCHIVED",
    "REOPENED",
    "VERIFICATION_FAILED",
    "VERIFICATION_PASSED",
    "INPUT_QUEUED",
    "QUESTION_REQUESTED",
    "QUESTION_RESOLVED",
    "CLAIM_FAILED",
];

/// One project page, or the detailed restart state for one of its tasks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlSnapshotArguments {
    /// Registered project identity.
    pub project_id: String,
    /// Optional exact task; mutually exclusive with page continuation.
    pub task_ref: Option<ContentDigest>,
    /// Exclusive task-reference cursor, or the empty string.
    pub after_task_ref: String,
    /// Exact saved native question response for this task, independent of previews.
    pub question_id: Option<String>,
    /// A closed full-decision read, mutually exclusive with task projection fields.
    pub decisions: Option<DecisionSnapshotQuery>,
}

impl ControlSnapshotArguments {
    pub(crate) fn from_value(value: Option<&Value>) -> Option<Self> {
        let object = value?.as_object()?;
        if object.contains_key("decisions") {
            exact_keys(object, &["decisions"], &[])?;
            return Some(Self {
                project_id: String::new(),
                task_ref: None,
                after_task_ref: String::new(),
                question_id: None,
                decisions: Some(DecisionSnapshotQuery::from_value(object.get("decisions")?)?),
            });
        }
        exact_keys(
            object,
            &["project_id"],
            &["task_ref", "after_task_ref", "question_id"],
        )?;
        let project_id = identifier(object, "project_id", 64)?;
        let task_ref = optional_digest(object, "task_ref")?;
        let after_task_ref = optional_digest(object, "after_task_ref")?;
        if task_ref.is_some() && after_task_ref.is_some() {
            return None;
        }
        let question_id = if object.contains_key("question_id") {
            Some(identifier(object, "question_id", 128)?)
        } else {
            None
        };
        if question_id.is_some() && task_ref.is_none() {
            return None;
        }
        Some(Self {
            project_id,
            task_ref,
            after_task_ref: after_task_ref.map_or_else(String::new, |v| v.as_str().to_owned()),
            question_id,
            decisions: None,
        })
    }
}

/// Closed selectors for full decision reads; exact-ID reads require no scope map.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecisionSnapshotQuery {
    /// Current decisions within one exact scope and optional subject.
    Current {
        scope: String,
        subject: Option<String>,
        limit: i32,
    },
    /// One decision and the bounded chain containing it at an exact durable head.
    Read {
        decision_id: String,
        max_depth: i32,
        revision: i64,
        digest: String,
    },
    /// Current and retained decisions containing a literal query.
    Search {
        scope: String,
        query: String,
        limit: i32,
        revision: i64,
        digest: String,
    },
}

impl DecisionSnapshotQuery {
    fn from_value(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        match object.get("mode")?.as_str()? {
            "current" => {
                exact_keys(object, &["mode", "scope", "limit"], &["subject"])?;
                Some(Self::Current {
                    scope: identifier(object, "scope", 64)?,
                    subject: if object.contains_key("subject") {
                        Some(text(object, "subject", 256, false)?)
                    } else {
                        None
                    },
                    limit: decision_bound(object, "limit", 32)?,
                })
            }
            "read" => {
                exact_keys(
                    object,
                    &["mode", "decision_id", "max_depth", "revision", "digest"],
                    &[],
                )?;
                Some(Self::Read {
                    decision_id: identifier(object, "decision_id", 128)?,
                    max_depth: decision_bound(object, "max_depth", 64)?,
                    revision: decision_revision(object, "revision")?,
                    digest: digest(object, "digest")?.as_str().to_owned(),
                })
            }
            "search" => {
                exact_keys(
                    object,
                    &["mode", "scope", "query", "limit", "revision", "digest"],
                    &[],
                )?;
                let query = text(object, "query", 128, false)?;
                if query.chars().any(char::is_control) {
                    return None;
                }
                Some(Self::Search {
                    scope: identifier(object, "scope", 64)?,
                    query: query.trim().to_owned(),
                    limit: decision_bound(object, "limit", 20)?,
                    revision: decision_revision(object, "revision")?,
                    digest: digest(object, "digest")?.as_str().to_owned(),
                })
            }
            _ => None,
        }
    }

    pub(crate) fn scope(&self) -> Option<&str> {
        match self {
            Self::Current { scope, .. } | Self::Search { scope, .. } => Some(scope),
            Self::Read { .. } => None,
        }
    }

    pub(crate) fn read(&self, store: &mut PostgresControlProduct) -> Result<Value, &'static str> {
        match self {
            Self::Current {
                scope,
                subject,
                limit,
            } => store.decisions(
                "current",
                Some(scope),
                None,
                subject.as_deref(),
                None,
                Some(*limit),
                None,
                None,
                None,
            ),
            Self::Read {
                decision_id,
                max_depth,
                revision,
                digest,
            } => store.decisions(
                "read",
                None,
                Some(decision_id),
                None,
                None,
                None,
                Some(*max_depth),
                Some(*revision),
                Some(digest),
            ),
            Self::Search {
                scope,
                query,
                limit,
                revision,
                digest,
            } => store.decisions(
                "search",
                Some(scope),
                None,
                None,
                Some(query),
                Some(*limit),
                None,
                Some(*revision),
                Some(digest),
            ),
        }
    }
}

fn decision_bound(object: &Map<String, Value>, key: &str, maximum: i32) -> Option<i32> {
    let value = i32::try_from(object.get(key)?.as_i64()?).ok()?;
    (1..=maximum).contains(&value).then_some(value)
}
fn decision_revision(object: &Map<String, Value>, key: &str) -> Option<i64> {
    let value = revision(object, key)?;
    (value <= 9_007_199_254_740_991).then_some(value)
}
fn source_segment(value: &str, maximum: usize, allowed: &[u8]) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || allowed.contains(&byte))
}
fn decision_source(value: &Value) -> Option<(String, String)> {
    let object = value.as_object()?;
    exact_keys(object, &["kind", "reference"], &[])?;
    let kind = text(object, "kind", 24, false)?;
    let reference = text(object, "reference", 512, false)?;
    let valid = match kind.as_str() {
        "user_confirmation" => reference
            .strip_prefix("thread:")
            .and_then(|rest| rest.split_once('/'))
            .is_some_and(|(thread, tail)| {
                let turn = tail
                    .strip_prefix("turn:")
                    .or_else(|| tail.strip_prefix("delegation:"));
                source_segment(thread, 128, b"._-")
                    && turn.is_some_and(|turn| {
                        let (id, fragment) = turn
                            .split_once('#')
                            .map_or((turn, None), |(id, fragment)| (id, Some(fragment)));
                        source_segment(id, 128, b"._:-")
                            && fragment
                                .is_none_or(|fragment| source_segment(fragment, 128, b"._:-"))
                    })
            }),
        "approved_document" => reference
            .split_once('#')
            .is_some_and(|(document, fragment)| {
                let document = document
                    .strip_prefix("file:")
                    .map(|body| (body, b"._/-".as_slice()))
                    .or_else(|| {
                        document
                            .strip_prefix("document:")
                            .map(|body| (body, b"._:/-".as_slice()))
                    });
                source_segment(fragment, 128, b"._:-")
                    && document.is_some_and(|(body, allowed)| source_segment(body, 384, allowed))
            }),
        _ => false,
    };
    valid.then_some((kind, reference))
}
fn byte_bounded(value: &str, maximum: usize) -> String {
    let mut end = value.len().min(maximum);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}
fn decision_source_schema() -> Value {
    json!({"oneOf":[
      {"type":"object","additionalProperties":false,"required":["kind","reference"],"properties":{
        "kind":{"const":"user_confirmation"},"reference":{"type":"string","minLength":1,"maxLength":512,
          "pattern":"^thread:[A-Za-z0-9][A-Za-z0-9._-]{0,127}/(?:turn|delegation):[A-Za-z0-9][A-Za-z0-9._:-]{0,127}(?:#[A-Za-z0-9][A-Za-z0-9._:-]{0,127})?$"}}},
      {"type":"object","additionalProperties":false,"required":["kind","reference"],"properties":{
        "kind":{"const":"approved_document"},"reference":{"type":"string","minLength":1,"maxLength":512,
          "pattern":"^(?:file:[A-Za-z0-9][A-Za-z0-9._/-]{0,383}|document:[A-Za-z0-9][A-Za-z0-9._:/-]{0,383})#[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$"}}}
    ]})
}

/// A product edit, or a claim whose model and workspace are selected by Runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlUpdateArguments {
    /// Runtime selects execution configuration after validating this task.
    Claim {
        task_ref: ContentDigest,
        claim_id: String,
        phase: String,
        prompt: String,
    },
    /// Closed observation or metadata command; contains no SQL or completion setter.
    Command(ControlProductCommand),
}

impl ControlUpdateArguments {
    pub(crate) fn from_value(value: Option<&Value>) -> Option<Self> {
        let object = value?.as_object()?;
        match object.get("action")?.as_str()? {
            "METADATA" => {
                exact_keys(
                    object,
                    &[
                        "action",
                        "task_ref",
                        "request_id",
                        "expected_revision",
                        "title",
                        "success_criteria",
                        "priority",
                    ],
                    &["parent_ref", "dependency_refs"],
                )?;
                let priority = i32::try_from(object.get("priority")?.as_i64()?).ok()?;
                if !(0..=3).contains(&priority) {
                    return None;
                }
                let dependency_refs = match object.get("dependency_refs") {
                    None => Vec::new(),
                    Some(value) => value
                        .as_array()?
                        .iter()
                        .map(|v| {
                            Some(
                                ContentDigest::from_sha256(v.as_str()?)
                                    .ok()?
                                    .as_str()
                                    .to_owned(),
                            )
                        })
                        .collect::<Option<Vec<_>>>()?,
                };
                if dependency_refs.len() > 64 {
                    return None;
                }
                Some(Self::Command(ControlProductCommand::Metadata {
                    task_ref: digest(object, "task_ref")?,
                    request_id: identifier(object, "request_id", 128)?,
                    expected_revision: revision(object, "expected_revision")?,
                    title: text(object, "title", 256, false)?,
                    success_criteria: text(object, "success_criteria", 8192, false)?,
                    priority,
                    parent_ref: optional_digest(object, "parent_ref")?
                        .map(|v| v.as_str().to_owned()),
                    dependency_refs,
                }))
            }
            "CLAIM" => {
                exact_keys(
                    object,
                    &["action", "task_ref", "claim_id", "phase", "prompt"],
                    &[],
                )?;
                let phase = text(object, "phase", 16, false)?;
                if !["EXECUTION", "VERIFICATION"].contains(&phase.as_str()) {
                    return None;
                }
                Some(Self::Claim {
                    task_ref: digest(object, "task_ref")?,
                    claim_id: identifier(object, "claim_id", 128)?,
                    phase,
                    prompt: text(object, "prompt", 16384, false)?,
                })
            }
            "OBSERVE" => {
                exact_keys(
                    object,
                    &[
                        "action",
                        "task_ref",
                        "claim_id",
                        "request_id",
                        "expected_sequence",
                        "kind",
                        "summary",
                    ],
                    &[
                        "thread_id",
                        "turn_id",
                        "evidence_ref",
                        "approval_id",
                        "decision",
                        "input_id",
                        "payload",
                    ],
                )?;
                let kind = text(object, "kind", 32, false)?;
                if !OBSERVATION_KINDS.contains(&kind.as_str()) {
                    return None;
                }
                let decision = optional_text(object, "decision", 24)?;
                if decision.as_ref().is_some_and(|value| {
                    !["accept", "acceptForSession", "decline", "cancel"].contains(&value.as_str())
                }) {
                    return None;
                }
                let evidence_ref = optional_text(object, "evidence_ref", 80)?;
                if evidence_ref.as_ref().is_some_and(|value| {
                    value
                        .strip_prefix("evidence:sha256:")
                        .and_then(|v| ContentDigest::from_sha256(v).ok())
                        .is_none()
                }) {
                    return None;
                }
                let payload = match object.get("payload") {
                    None | Some(Value::Null) => None,
                    Some(value)
                        if value.is_object()
                            && value.to_string().len() <= 16384
                            && !task_submission_text_contains_secret(&value.to_string()) =>
                    {
                        Some(value.clone())
                    }
                    Some(_) => return None,
                };
                Some(Self::Command(ControlProductCommand::Observe {
                    task_ref: digest(object, "task_ref")?,
                    claim_id: identifier(object, "claim_id", 128)?,
                    request_id: identifier(object, "request_id", 128)?,
                    expected_sequence: revision(object, "expected_sequence")?,
                    kind,
                    thread_id: optional_identifier(object, "thread_id")?,
                    turn_id: optional_identifier(object, "turn_id")?,
                    summary: text(object, "summary", 16384, true)?,
                    evidence_ref,
                    approval_id: optional_identifier(object, "approval_id")?,
                    decision,
                    input_id: optional_identifier(object, "input_id")?,
                    payload,
                }))
            }
            "DECISION" => {
                exact_keys(
                    object,
                    &[
                        "action",
                        "decision_id",
                        "project_id",
                        "subject",
                        "content",
                        "reason",
                        "source",
                        "client_request_id",
                        "expected_revision",
                        "expected_digest",
                    ],
                    &["task_ref", "supersedes_id"],
                )?;
                let (source, source_reference) = decision_source(object.get("source")?)?;
                Some(Self::Command(ControlProductCommand::Decision {
                    decision_id: identifier(object, "decision_id", 128)?,
                    project_id: identifier(object, "project_id", 64)?,
                    task_ref: optional_digest(object, "task_ref")?.map(|v| v.as_str().to_owned()),
                    subject: text(object, "subject", 256, false)?,
                    content: text(object, "content", 4096, false)?,
                    reason: text(object, "reason", 4096, false)?,
                    source,
                    source_reference,
                    supersedes_id: optional_identifier(object, "supersedes_id")?,
                    client_request_id: identifier(object, "client_request_id", 128)?,
                    expected_revision: decision_revision(object, "expected_revision")?,
                    expected_digest: digest(object, "expected_digest")?.as_str().to_owned(),
                }))
            }
            _ => None,
        }
    }
}

fn exact_keys(object: &Map<String, Value>, required: &[&str], optional: &[&str]) -> Option<()> {
    if required.iter().all(|key| object.contains_key(*key))
        && object
            .keys()
            .all(|key| required.contains(&key.as_str()) || optional.contains(&key.as_str()))
    {
        Some(())
    } else {
        None
    }
}
fn text(object: &Map<String, Value>, key: &str, maximum: usize, empty: bool) -> Option<String> {
    let value = object.get(key)?.as_str()?;
    if value.len() > maximum
        || (!empty && value.trim().is_empty())
        || value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        || task_submission_text_contains_secret(value)
    {
        return None;
    }
    Some(value.to_owned())
}
fn optional_text(object: &Map<String, Value>, key: &str, maximum: usize) -> Option<Option<String>> {
    match object.get(key) {
        None | Some(Value::Null) => Some(None),
        Some(_) => Some(Some(text(object, key, maximum, false)?)),
    }
}
fn identifier(object: &Map<String, Value>, key: &str, maximum: usize) -> Option<String> {
    let value = text(object, key, maximum, false)?;
    if !value
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || b"._:-".contains(&c))
    {
        return None;
    }
    Some(value)
}
fn optional_identifier(object: &Map<String, Value>, key: &str) -> Option<Option<String>> {
    match object.get(key) {
        None | Some(Value::Null) => Some(None),
        Some(_) => Some(Some(identifier(object, key, 128)?)),
    }
}
fn digest(object: &Map<String, Value>, key: &str) -> Option<ContentDigest> {
    ContentDigest::from_sha256(object.get(key)?.as_str()?).ok()
}
fn optional_digest(object: &Map<String, Value>, key: &str) -> Option<Option<ContentDigest>> {
    match object.get(key) {
        None | Some(Value::Null) => Some(None),
        Some(_) => Some(Some(digest(object, key)?)),
    }
}
fn revision(object: &Map<String, Value>, key: &str) -> Option<i64> {
    let value = object.get(key)?.as_i64()?;
    if value < 0 { None } else { Some(value) }
}

pub(crate) fn bound_snapshot(value: &mut Value, detail: bool) {
    let decision_count = value["decisions"].as_array().map_or(0, Vec::len);
    let observation_count = value["observations"].as_array().map_or(0, Vec::len);
    if let Some(claims) = value["claims"].as_array_mut() {
        for claim in claims {
            if let Some(outcome) = claim
                .get_mut("verification_outcome")
                .and_then(Value::as_object_mut)
                && let Some(text) = outcome.get("summary").and_then(Value::as_str)
            {
                let clipped = byte_bounded(text, 2048);
                outcome.insert("summary_truncated".to_owned(), json!(clipped != text));
                outcome.insert("summary".to_owned(), json!(clipped));
            }
            if let Some(questions) = claim["pending_questions"].as_array_mut() {
                for question in questions {
                    if let Some(text) = question["summary"].as_str() {
                        let clipped = text.chars().take(256).collect::<String>();
                        question["summary_truncated"] = json!(clipped != text);
                        question["summary"] = json!(clipped);
                    }
                }
            }
            if !detail && let Some(object) = claim.as_object_mut() {
                object.remove("prompt");
                for field in ["pending_inputs", "pending_questions"] {
                    let count = object
                        .get(field)
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len);
                    object.insert(format!("{field}_count"), json!(count));
                    object.remove(field);
                }
            }
        }
    }
    if let Some(observations) = value["observations"].as_array_mut() {
        let mut retained = Vec::new();
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for mut item in observations.drain(..).rev() {
            let key = item["claim_id"].as_str().unwrap_or_default().to_owned();
            let count = counts.entry(key).or_default();
            if *count >= if detail { 10 } else { 1 } {
                continue;
            }
            *count += 1;
            if let Some(object) = item.as_object_mut() {
                object.remove("payload");
                if let Some(text) = object.get("summary").and_then(Value::as_str) {
                    let clipped = text.chars().take(256).collect::<String>();
                    object.insert("summary_truncated".to_owned(), json!(clipped != text));
                    object.insert("summary".to_owned(), json!(clipped));
                }
            }
            retained.push(item);
        }
        retained.reverse();
        *observations = retained;
    }
    if let Some(decisions) = value["decisions"].as_array_mut() {
        if decisions.len() > 32 {
            *decisions = decisions.split_off(decisions.len() - 32);
        }
        {
            for decision in decisions {
                for field in ["content", "reason"] {
                    if let Some(text) = decision[field].as_str() {
                        let clipped = text.chars().take(128).collect::<String>();
                        decision[format!("{field}_truncated")] = json!(clipped != text);
                        decision[field] = json!(clipped);
                    }
                }
            }
        }
    }
    if !detail && let Some(metadata) = value["metadata"].as_array_mut() {
        for item in metadata {
            if let Some(criteria) = item["success_criteria"].as_str() {
                item["success_criteria"] = json!(criteria.chars().take(512).collect::<String>());
            }
        }
    }
    value["truncation"] = json!({"decisions":decision_count>32,"observations":observation_count>value["observations"].as_array().map_or(0,Vec::len),"decision_text":"PREVIEW"});
}

pub(crate) fn snapshot_schema() -> Value {
    let id = json!({"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9._:-]+$"});
    let scope =
        json!({"type":"string","minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9._:-]+$"});
    let digest = json!({"type":"string","pattern":"^[a-f0-9]{64}$"});
    let revision = json!({"type":"integer","minimum":0,"maximum":9007199254740991_i64});
    json!({"type":"object","oneOf":[
      {"type":"object","additionalProperties":false,"required":["project_id"],
       "properties":{"project_id":scope,"task_ref":digest,"after_task_ref":digest,"question_id":id},
       "allOf":[{"not":{"required":["task_ref","after_task_ref"]}},
         {"if":{"required":["question_id"]},"then":{"required":["task_ref"]}}]},
      {"type":"object","additionalProperties":false,"required":["decisions"],"properties":{"decisions":{"oneOf":[
        {"type":"object","additionalProperties":false,"required":["mode","scope","limit"],
         "properties":{"mode":{"const":"current"},"scope":scope,"limit":{"type":"integer","minimum":1,"maximum":32},
           "subject":{"type":"string","minLength":1,"maxLength":256}}},
        {"type":"object","additionalProperties":false,"required":["mode","decision_id","max_depth","revision","digest"],
         "properties":{"mode":{"const":"read"},"decision_id":id,"max_depth":{"type":"integer","minimum":1,"maximum":64},"revision":revision,"digest":digest}},
        {"type":"object","additionalProperties":false,"required":["mode","scope","query","limit","revision","digest"],
         "properties":{"mode":{"const":"search"},"scope":scope,"query":{"type":"string","minLength":1,"maxLength":128},
           "limit":{"type":"integer","minimum":1,"maximum":20},"revision":revision,"digest":digest}}
      ]}}}
    ]})
}

pub(crate) fn update_schema() -> Value {
    let identifier =
        json!({"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9._:-]+$"});
    let digest = json!({"type":"string","pattern":"^[a-f0-9]{64}$"});
    let nullable_id = json!({"anyOf":[identifier,{"type":"null"}]});
    let nullable_digest = json!({"anyOf":[digest,{"type":"null"}]});
    let bounded = |max| json!({"type":"string","maxLength":max});
    json!({"type":"object","oneOf":[
        {"type":"object","additionalProperties":false,
         "required":["action","task_ref","request_id","expected_revision","title","success_criteria","priority"],
         "properties":{"action":{"const":"METADATA"},"task_ref":digest,"request_id":identifier,
            "expected_revision":{"type":"integer","minimum":0},"title":bounded(256),"success_criteria":bounded(8192),
            "priority":{"type":"integer","minimum":0,"maximum":3},"parent_ref":nullable_digest,
            "dependency_refs":{"type":"array","maxItems":64,"uniqueItems":true,"items":digest}}},
        {"type":"object","additionalProperties":false,"required":["action","task_ref","claim_id","phase","prompt"],
         "properties":{"action":{"const":"CLAIM"},"task_ref":digest,"claim_id":identifier,
            "phase":{"enum":["EXECUTION","VERIFICATION"]},"prompt":bounded(16384)}},
        {"type":"object","additionalProperties":false,"required":["action","task_ref","claim_id","request_id","expected_sequence","kind","summary"],
         "properties":{"action":{"const":"OBSERVE"},"task_ref":digest,"claim_id":identifier,"request_id":identifier,
            "expected_sequence":{"type":"integer","minimum":0},"kind":{"enum":OBSERVATION_KINDS},"summary":bounded(16384),
            "thread_id":nullable_id,"turn_id":nullable_id,"input_id":nullable_id,"approval_id":nullable_id,
            "evidence_ref":{"anyOf":[{"type":"string","pattern":"^evidence:sha256:[a-f0-9]{64}$"},{"type":"null"}]},
            "decision":{"enum":["accept","acceptForSession","decline","cancel",null]},
            "payload":{"type":["object","null"]}}},
        {"type":"object","additionalProperties":false,"required":["action","decision_id","project_id","subject","content","reason","source","client_request_id","expected_revision","expected_digest"],
         "properties":{"action":{"const":"DECISION"},"decision_id":identifier,"project_id":bounded(64),
            "task_ref":nullable_digest,"subject":bounded(256),"content":bounded(4096),"reason":bounded(4096),
            "source":decision_source_schema(),"supersedes_id":nullable_id,"client_request_id":identifier,
            "expected_revision":{"type":"integer","minimum":0,"maximum":9007199254740991_i64},"expected_digest":digest}}
    ]})
}

#[cfg(test)]
mod decision_tests {
    use super::*;

    #[test]
    fn decision_snapshot_closed_selectors_preserve_exact_id_lookup() {
        let digest = "a".repeat(64);
        assert!(ControlSnapshotArguments::from_value(Some(&json!({"decisions":{"mode":"read","decision_id":"decision-1","max_depth":64,"revision":0,"digest":digest}}))).is_some());
        assert!(
            ControlSnapshotArguments::from_value(Some(
                &json!({"project_id":"p","decisions":{"mode":"current","scope":"p","limit":1}})
            ))
            .is_none()
        );
        assert!(ControlSnapshotArguments::from_value(Some(&json!({"decisions":{"mode":"read","decision_id":"decision-1","scope":"p","max_depth":64,"revision":0,"digest":digest}}))).is_none());
        assert!(
            ControlSnapshotArguments::from_value(Some(
                &json!({"decisions":{"mode":"current","scope":"p","limit":33}})
            ))
            .is_none()
        );
        assert!(ControlSnapshotArguments::from_value(Some(&json!({"decisions":{"mode":"search","scope":"p","query":"test","limit":1,"revision":9_007_199_254_740_992_i64,"digest":digest}}))).is_none());
    }

    #[test]
    fn question_lookup_requires_one_exact_task() {
        let task = "b".repeat(64);
        assert!(
            ControlSnapshotArguments::from_value(Some(
                &json!({"project_id":"p","task_ref":task,"question_id":"q:123"})
            ))
            .is_some()
        );
        assert!(
            ControlSnapshotArguments::from_value(Some(
                &json!({"project_id":"p","question_id":"q:123"})
            ))
            .is_none()
        );
        assert!(ControlSnapshotArguments::from_value(Some(&json!({"project_id":"p","task_ref":task,"after_task_ref":task,"question_id":"q:123"}))).is_none());
        assert!(
            ControlSnapshotArguments::from_value(Some(
                &json!({"project_id":"p","task_ref":task,"question_id":"問題"})
            ))
            .is_none()
        );
    }

    #[test]
    fn decision_write_requires_exact_source_and_idempotency_head() {
        let mut input = json!({"action":"DECISION","decision_id":"decision-1","project_id":"p","subject":"topic",
            "content":"choice","reason":"reason","source":{"kind":"user_confirmation","reference":"thread:thread-1/turn:turn-1"},
            "client_request_id":"request-1","expected_revision":0,"expected_digest":"c".repeat(64)});
        assert!(ControlUpdateArguments::from_value(Some(&input)).is_some());
        input["source"]["reference"] = json!("thread:thread-1");
        assert!(ControlUpdateArguments::from_value(Some(&input)).is_none());
        input["source"] =
            json!({"kind":"approved_document","reference":"document:rules/v1#decision-1"});
        assert!(ControlUpdateArguments::from_value(Some(&input)).is_some());
        input.as_object_mut().unwrap().remove("expected_digest");
        assert!(ControlUpdateArguments::from_value(Some(&input)).is_none());
    }

    #[test]
    fn verdict_preview_is_byte_bounded_and_exact_response_is_retained() {
        let answer = "答".repeat(1024);
        let mut value = json!({"claims":[{"repair_attempts":2,"verification_outcome":{"kind":"VERIFICATION_FAILED","summary":"錯".repeat(1024)}}],
            "question_resolution":{"payload":{"answers":{"q":{"answers":[answer]}}}},"observations":[],"decisions":[]});
        bound_snapshot(&mut value, true);
        assert!(
            value["claims"][0]["verification_outcome"]["summary"]
                .as_str()
                .unwrap()
                .len()
                <= 2048
        );
        assert_eq!(
            value["claims"][0]["verification_outcome"]["summary_truncated"],
            true
        );
        assert_eq!(value["claims"][0]["repair_attempts"], 2);
        assert_eq!(
            value["question_resolution"]["payload"]["answers"]["q"]["answers"][0],
            answer
        );
    }
}
