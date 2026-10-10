//! Maintenance-only native archive verification, without loading or dispatching a thread.
use lattice_contracts::ContentDigest;
use lattice_postgres_store::{ArchivedClaimProof, connect_project_purge, reconcile_archived_claim};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
#[cfg(windows)]
use std::fs::OpenOptions;
#[cfg(windows)]
use std::io::Read;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, &'static str>;
const LIMIT: u64 = 64 * 1024 * 1024;
fn hash(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut value = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
    }
    value
}
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or("CLAIM_RECONCILIATION_INPUT_REJECTED")
}
fn id(v: &Value, key: &str) -> Result<String> {
    let s = string(v, key)?;
    if s.is_empty()
        || s.len() > 120
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
    {
        return Err("CLAIM_RECONCILIATION_INPUT_REJECTED");
    }
    Ok(s.to_owned())
}
fn result_value(item: &Value) -> Option<Value> {
    if item["status"] != "completed" || item["result"]["isError"] != false {
        return None;
    }
    if item["result"]["structuredContent"].is_object() {
        return Some(item["result"]["structuredContent"].clone());
    }
    serde_json::from_str(
        item["result"]["content"]
            .as_array()?
            .iter()
            .find(|c| c["type"] == "text")?["text"]
            .as_str()?,
    )
    .ok()
}
struct Archive {
    thread: String,
    turn: String,
    started: i64,
    completed: i64,
}
fn archive_facts(bytes: &[u8]) -> Result<Archive> {
    let text = std::str::from_utf8(bytes).map_err(|_| "CLAIM_RECONCILIATION_EVIDENCE_REJECTED")?;
    let mut thread = None;
    let mut first = None;
    let mut turns = BTreeMap::new();
    let mut children = BTreeSet::new();
    for line in text.lines() {
        let v: Value =
            serde_json::from_str(line).map_err(|_| "CLAIM_RECONCILIATION_EVIDENCE_REJECTED")?;
        let p = &v["payload"];
        if v["type"] == "session_meta" {
            if thread.is_some() {
                return Err("CLAIM_RECONCILIATION_EVIDENCE_REJECTED");
            }
            thread = Some(id(p, "id")?);
        }
        if v["type"] != "event_msg" {
            continue;
        }
        match p["type"].as_str() {
            Some("task_started") => {
                let turn = id(p, "turn_id")?;
                let started = p["started_at"]
                    .as_i64()
                    .filter(|t| *t > 0)
                    .ok_or("CLAIM_RECONCILIATION_EVIDENCE_REJECTED")?;
                if first.is_none() {
                    first = Some(turn.clone());
                }
                if turns.insert(turn, (started, None)).is_some() {
                    return Err("CLAIM_RECONCILIATION_EVIDENCE_REJECTED");
                }
            }
            Some("task_complete" | "turn_aborted") => {
                let turn = id(p, "turn_id")?;
                let (started, terminal) = turns
                    .get_mut(&turn)
                    .ok_or("CLAIM_RECONCILIATION_EVIDENCE_REJECTED")?;
                let completed = p["completed_at"]
                    .as_i64()
                    .ok_or("CLAIM_RECONCILIATION_EVIDENCE_REJECTED")?;
                if terminal.is_some()
                    || p["started_at"].as_i64() != Some(*started)
                    || completed < *started
                {
                    return Err("CLAIM_RECONCILIATION_EVIDENCE_REJECTED");
                }
                *terminal = Some((completed, p["type"] == "task_complete"));
            }
            Some("item_completed") if p["item"]["type"] == "SubAgentActivity" => {
                let child = id(&p["item"], "agent_thread_id")?;
                match p["item"]["kind"].as_str() {
                    Some("started" | "interacted") => {
                        children.insert(child);
                    }
                    Some("completed") => {
                        children.remove(&child);
                    }
                    _ => return Err("CLAIM_RECONCILIATION_CHILD_UNCERTAIN"),
                }
            }
            _ => {}
        }
    }
    if !children.is_empty() || turns.values().any(|(_, t)| t.is_none()) {
        return Err("CLAIM_RECONCILIATION_TURN_NOT_TERMINAL");
    }
    let turn = first.ok_or("CLAIM_RECONCILIATION_EVIDENCE_REJECTED")?;
    let (started, terminal) = turns[&turn];
    let (completed, success) = terminal.ok_or("CLAIM_RECONCILIATION_TURN_NOT_TERMINAL")?;
    if !success {
        return Err("CLAIM_RECONCILIATION_ORIGINAL_TURN_NOT_COMPLETED");
    }
    Ok(Archive {
        thread: thread.ok_or("CLAIM_RECONCILIATION_EVIDENCE_REJECTED")?,
        turn,
        started,
        completed,
    })
}
fn parent_facts(
    bytes: &[u8],
    archive: &Archive,
    project: &str,
    task: &str,
    claim: &str,
) -> Result<String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "CLAIM_RECONCILIATION_EVIDENCE_REJECTED")?;
    let mut created = 0;
    let mut bound = None;
    let mut completed = false;
    let mut progressing = false;
    for line in text.lines() {
        let v: Value =
            serde_json::from_str(line).map_err(|_| "CLAIM_RECONCILIATION_EVIDENCE_REJECTED")?;
        if v["type"] != "event_msg" || v["payload"]["type"] != "item_completed" {
            continue;
        }
        let item = &v["payload"]["item"];
        let Some(result) = result_value(item) else {
            continue;
        };
        if item["server"] == "codex_app"
            && item["tool"] == "create_thread"
            && result["threadId"] == archive.thread
        {
            let prompt = item["arguments"]["prompt"]
                .as_str()
                .ok_or("CLAIM_RECONCILIATION_SOURCE_LINK_MISSING")?;
            if ![project, task, claim].iter().all(|s| prompt.contains(s)) {
                return Err("CLAIM_RECONCILIATION_SOURCE_LINK_MISSING");
            }
            created += 1;
        }
        if item["server"] == "lattice" && item["tool"] == "lattice_control_update" {
            let a = &item["arguments"];
            let r = &result["record"];
            if r["claim_id"] == claim && r["kind"] == "THREAD_BOUND" && r["sequence"] == 1 {
                if created != 1
                    || a["task_ref"] != task
                    || r["thread_id"] != archive.thread
                    || bound.is_some()
                {
                    return Err("CLAIM_RECONCILIATION_SOURCE_LINK_MISSING");
                }
                let digest = string(r, "request_digest")?;
                ContentDigest::from_sha256(digest)
                    .map_err(|_| "CLAIM_RECONCILIATION_SOURCE_LINK_MISSING")?;
                bound = Some(digest.to_owned());
            }
        }
        if item["server"] == "codex_app" && item["tool"] == "wait_threads" && created == 1 {
            for poll in result["polls"].as_array().into_iter().flatten() {
                let t = &poll["latestTurn"];
                if poll["thread"]["id"] == archive.thread
                    && t["id"] == archive.turn
                    && t["startedAt"] == archive.started
                {
                    if t["status"] == "inProgress" && t["completedAt"].is_null() {
                        progressing = true;
                    }
                    if bound.is_some()
                        && t["status"] == "completed"
                        && t["completedAt"] == archive.completed
                    {
                        completed = true;
                    }
                }
            }
        }
    }
    if created != 1 || !progressing || !completed {
        return Err("CLAIM_RECONCILIATION_SOURCE_LINK_MISSING");
    }
    bound.ok_or("CLAIM_RECONCILIATION_SOURCE_LINK_MISSING")
}

#[cfg(windows)]
fn locked_archive(home: &Path, supplied: &str, expected: &str) -> Result<(File, Vec<u8>)> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let path = PathBuf::from(supplied);
    if !path.is_absolute() || path.extension().and_then(|x| x.to_str()) != Some("jsonl") {
        return Err("CLAIM_RECONCILIATION_ARCHIVE_REQUIRED");
    }
    for ancestor in path.ancestors() {
        if std::fs::symlink_metadata(ancestor)
            .map_err(|_| "CLAIM_RECONCILIATION_ARCHIVE_REQUIRED")?
            .file_attributes()
            & 0x400
            != 0
        {
            return Err("CLAIM_RECONCILIATION_ARCHIVE_REQUIRED");
        }
    }
    let root = home
        .join("archived_sessions")
        .canonicalize()
        .map_err(|_| "CLAIM_RECONCILIATION_ARCHIVE_REQUIRED")?;
    let canonical = path
        .canonicalize()
        .map_err(|_| "CLAIM_RECONCILIATION_ARCHIVE_REQUIRED")?;
    if canonical.parent() != Some(root.as_path()) {
        return Err("CLAIM_RECONCILIATION_ARCHIVE_REQUIRED");
    }
    // Deny write/delete sharing through the complete database transaction. An
    // unarchive/append cannot silently race the persisted terminal observation.
    let mut file = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&canonical)
        .map_err(|_| "CLAIM_RECONCILIATION_ARCHIVE_BUSY")?;
    let metadata = file
        .metadata()
        .map_err(|_| "CLAIM_RECONCILIATION_ARCHIVE_REQUIRED")?;
    if !metadata.is_file() || metadata.len() > LIMIT {
        return Err("CLAIM_RECONCILIATION_EVIDENCE_TOO_LARGE");
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "CLAIM_RECONCILIATION_EVIDENCE_REJECTED")?;
    if bytes.len() as u64 > LIMIT || hash(&bytes) != expected {
        return Err("CLAIM_RECONCILIATION_EVIDENCE_CHANGED");
    }
    Ok((file, bytes))
}
#[cfg(not(windows))]
fn locked_archive(_: &Path, _: &str, _: &str) -> Result<(File, Vec<u8>)> {
    Err("CLAIM_RECONCILIATION_PLATFORM_UNSUPPORTED")
}

/// Verify actual native archive records and atomically reconcile an existing claim.
/// No online/MCP fallback exists. Current delivery supports Windows archive locks.
/// # Errors
/// Rejects incomplete provenance, live or unarchived turns, changed evidence,
/// unsupported hosts, non-stopped Store or an incompatible existing claim.
pub fn execute(request: &Value, port: u16, run_id: &str, password: &str) -> Result<Value> {
    let object = request
        .as_object()
        .ok_or("CLAIM_RECONCILIATION_INPUT_REJECTED")?;
    let keys = [
        "schema",
        "action",
        "authorization",
        "projectId",
        "taskRef",
        "claimId",
        "operationId",
        "expectedSequence",
        "parentArchive",
        "threadArchive",
        "parentSha256",
        "threadSha256",
    ];
    if object.len() != keys.len()
        || !keys.iter().all(|k| object.contains_key(*k))
        || request["schema"] != "lattice.project-purge.request.v1"
        || request["action"] != "reconcile-archived-claim"
        || request["authorization"] != "RECONCILE_ARCHIVED_CLAIM"
        || request["expectedSequence"] != 1
    {
        return Err("CLAIM_RECONCILIATION_INPUT_REJECTED");
    }
    let project = id(request, "projectId")?;
    let claim = id(request, "claimId")?;
    let operation = id(request, "operationId")?;
    let task = ContentDigest::from_sha256(string(request, "taskRef")?)
        .map_err(|_| "CLAIM_RECONCILIATION_INPUT_REJECTED")?;
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or("CLAIM_RECONCILIATION_CODEX_HOME_REQUIRED")?;
    let (_parent_lock, parent) = locked_archive(
        &home,
        string(request, "parentArchive")?,
        string(request, "parentSha256")?,
    )?;
    let (_thread_lock, thread) = locked_archive(
        &home,
        string(request, "threadArchive")?,
        string(request, "threadSha256")?,
    )?;
    let archive = archive_facts(&thread)?;
    let bound_digest = parent_facts(&parent, &archive, &project, task.as_str(), &claim)?;
    let source = json!({"schema":"lattice.archived-claim-evidence.v1","parentSha256":hash(&parent),"threadSha256":hash(&thread),
        "boundDigest":bound_digest,"projectId":project,"taskRef":task.as_str(),"claimId":claim,
        "threadId":archive.thread,"turnId":archive.turn,"startedAt":archive.started,"completedAt":archive.completed});
    let proof = ArchivedClaimProof {
        project_id: project,
        task_ref: task,
        claim_id: claim,
        thread_id: archive.thread,
        turn_id: archive.turn,
        operation_id: operation,
        evidence_digest: hash(source.to_string().as_bytes()),
        bound_digest,
        started_at: archive.started,
        completed_at: archive.completed,
    };
    let (mut client, target) = connect_project_purge(port, run_id, password)?;
    let mut result = reconcile_archived_claim(&mut client, &target, port, password, &proof)?;
    result["sourceEvidence"] = source;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bytes(lines: &[Value]) -> Vec<u8> {
        lines
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes()
    }
    fn archive() -> Vec<Value> {
        vec![
            json!({"type":"session_meta","payload":{"id":"thread-1"}}),
            json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-1","started_at":100}}),
            json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-1","started_at":100,"completed_at":120}}),
        ]
    }
    fn item(server: &str, tool: &str, args: Value, result: Value) -> Value {
        json!({"type":"event_msg","payload":{"type":"item_completed","item":{"server":server,"tool":tool,
            "status":"completed","arguments":args,"result":{"isError":false,"structuredContent":result}}}})
    }
    fn parent() -> Vec<Value> {
        vec![
            item(
                "codex_app",
                "create_thread",
                json!({"prompt":"project-1 task-1 claim-1"}),
                json!({"threadId":"thread-1"}),
            ),
            item(
                "codex_app",
                "wait_threads",
                json!({}),
                json!({"polls":[{"thread":{"id":"thread-1"},"latestTurn":{"id":"turn-1","status":"inProgress","startedAt":100,"completedAt":null}}]}),
            ),
            item(
                "lattice",
                "lattice_control_update",
                json!({"task_ref":"task-1"}),
                json!({"record":{"claim_id":"claim-1","kind":"THREAD_BOUND","sequence":1,"thread_id":"thread-1","request_digest":"a".repeat(64)}}),
            ),
            item(
                "codex_app",
                "wait_threads",
                json!({}),
                json!({"polls":[{"thread":{"id":"thread-1"},"latestTurn":{"id":"turn-1","status":"completed","startedAt":100,"completedAt":120}}]}),
            ),
        ]
    }
    #[test]
    fn original_event_chain_is_required_not_latest_terminal_guess() {
        let a = archive_facts(&bytes(&archive())).unwrap();
        assert_eq!(
            parent_facts(&bytes(&parent()), &a, "project-1", "task-1", "claim-1").unwrap(),
            "a".repeat(64)
        );
        let mut p = parent();
        p[3]["payload"]["item"]["result"]["structuredContent"]["polls"][0]["latestTurn"]["id"] =
            json!("later-unrelated-turn");
        assert_eq!(
            parent_facts(&bytes(&p), &a, "project-1", "task-1", "claim-1").unwrap_err(),
            "CLAIM_RECONCILIATION_SOURCE_LINK_MISSING"
        );
        assert!(parent_facts(&bytes(&parent()), &a, "other-project", "task-1", "claim-1").is_err());
        let mut p = parent();
        p.remove(2);
        assert!(parent_facts(&bytes(&p), &a, "project-1", "task-1", "claim-1").is_err());
        let mut p = parent();
        p.push(p[0].clone());
        assert!(parent_facts(&bytes(&p), &a, "project-1", "task-1", "claim-1").is_err());
        let mut p = parent();
        let mut failed = p[0].clone();
        failed["payload"]["item"]["result"]["isError"] = json!(true);
        p.insert(0, failed);
        assert!(parent_facts(&bytes(&p), &a, "project-1", "task-1", "claim-1").is_ok());
    }
    #[test]
    fn incomplete_or_ambiguous_archive_never_proves_terminal() {
        let mut a = archive();
        a.pop();
        assert!(archive_facts(&bytes(&a)).is_err());
        let mut a = archive();
        a[2]["payload"]["type"] = json!("turn_aborted");
        assert!(archive_facts(&bytes(&a)).is_err());
        let mut a = archive();
        a.push(json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-2","started_at":130}}));
        assert!(archive_facts(&bytes(&a)).is_err());
        let mut a = archive();
        a.push(a[2].clone());
        assert!(archive_facts(&bytes(&a)).is_err());
        let mut a = archive();
        a[2]["payload"]["completed_at"] = json!(90);
        assert!(archive_facts(&bytes(&a)).is_err());
    }
    #[test]
    fn pending_child_and_subsequent_unfinished_turn_block() {
        let mut a = archive();
        a.push(json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"SubAgentActivity","agent_thread_id":"child","kind":"started"}}}));
        assert!(archive_facts(&bytes(&a)).is_err());
        a.push(json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"SubAgentActivity","agent_thread_id":"child","kind":"completed"}}}));
        assert!(archive_facts(&bytes(&a)).is_ok());
        a.push(json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"later","started_at":130}}));
        a.push(json!({"type":"event_msg","payload":{"type":"turn_aborted","turn_id":"later","started_at":130,"completed_at":150}}));
        assert_eq!(archive_facts(&bytes(&a)).unwrap().turn, "turn-1");
    }
    #[cfg(windows)]
    #[test]
    fn real_archive_handle_blocks_writes_and_rename_until_released() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let home = std::env::temp_dir().join(format!(
            "lattice-archive-lock-{}-{nonce}",
            std::process::id()
        ));
        let root = home.join("archived_sessions");
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("fixture.jsonl");
        let content = bytes(&archive());
        std::fs::write(&file, &content).unwrap();
        let (lock, read) = locked_archive(&home, file.to_str().unwrap(), &hash(&content)).unwrap();
        assert_eq!(read, content);
        assert!(std::fs::write(&file, b"changed").is_err());
        assert!(std::fs::rename(&file, root.join("moved.jsonl")).is_err());
        assert!(locked_archive(&home, file.to_str().unwrap(), &"0".repeat(64)).is_err());
        drop(lock);
        std::fs::remove_file(file).unwrap();
        std::fs::remove_dir(root).unwrap();
        std::fs::remove_dir(home).unwrap();
    }
}
