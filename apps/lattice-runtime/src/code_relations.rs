//! Bounded historical Graphify read selectors. All source authority stays server-owned.
use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{GitObjectId, ProjectId};
use serde_json::{Value, json};

/// Verifies returned content and membership in the receipt's complete record set.
/// The internal proof is discarded before exposing the bounded MCP page.
pub(crate) fn verify_page(
    page: &mut Value,
    expected_set: &str,
    expected_count: u32,
    limit: i32,
) -> Option<()> {
    let proof = page.get("record_set_proof")?.as_array()?;
    if proof.len() != usize::try_from(expected_count).ok()? || proof.len() > 100_000 {
        return None;
    }
    let mut canonical = Vec::with_capacity(proof.len());
    for (index, item) in proof.iter().enumerate() {
        if item["ordinal"].as_str()? != (index + 1).to_string() {
            return None;
        }
        canonical.push(CanonicalValue::Object(
            ["content", "id", "ordinal"]
                .map(|key| {
                    Some((
                        key.to_owned(),
                        CanonicalValue::String(item[key].as_str()?.to_owned()),
                    ))
                })
                .into_iter()
                .collect::<Option<Vec<_>>>()?,
        ));
    }
    if commitment(
        "lattice.graph-memory.record-set",
        &CanonicalValue::Array(canonical),
    )? != expected_set
    {
        return None;
    }
    let rows = page.get("records")?.as_array()?;
    if rows.len() > usize::try_from(limit).ok()? || !page.get("truncated")?.is_boolean() {
        return None;
    }
    let mut previous = 0;
    for row in rows {
        let ordinal = usize::try_from(row["ordinal"].as_u64()?).ok()?;
        if ordinal <= previous {
            return None;
        }
        previous = ordinal;
        let witness = proof.get(ordinal.checked_sub(1)?)?;
        if witness["id"] != row["record_id"]
            || witness["content"] != row["content_digest"]
            || row["record_kind"] != "OBSERVATION"
            || row["review_state"] != "CANDIDATE"
            || row["trusted_context"] != false
        {
            return None;
        }
        let mut semantic = Vec::new();
        for (key, field) in [
            ("category", "category"),
            ("confidence", "confidence"),
            ("kind", "graph_kind"),
            ("object", "object"),
            ("path", "source_path"),
            ("relation", "relation"),
            ("source_digest", "source_digest"),
            ("line_start", "line_start"),
            ("line_end", "line_end"),
            ("subject", "subject"),
        ] {
            let v = row.get(field)?;
            let value = if v.is_null() {
                CanonicalValue::Null
            } else if field.starts_with("line_") {
                CanonicalValue::String(u32::try_from(v.as_u64()?).ok()?.to_string())
            } else {
                CanonicalValue::String(v.as_str()?.to_owned())
            };
            semantic.push((key.to_owned(), value));
        }
        if commitment(
            "lattice.graph-memory.record-content",
            &CanonicalValue::Object(semantic),
        )? != row["content_digest"].as_str()?
        {
            return None;
        }
    }
    page.as_object_mut()?.remove("record_set_proof");
    Some(())
}

fn commitment(domain: &str, value: &CanonicalValue) -> Option<String> {
    canonical_sha256(&HashDomain::new(domain, "1").ok()?, value)
        .ok()
        .map(|v| v.to_hex())
}

/// Literal query within one registered project's exact retained Git commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeRelationsArguments {
    pub(crate) project_id: String,
    pub(crate) commit: String,
    pub(crate) query: String,
    pub(crate) limit: i32,
    pub(crate) task_ref: Option<String>,
}

impl CodeRelationsArguments {
    pub(crate) fn from_value(value: Option<&Value>) -> Option<Self> {
        let o = value?.as_object()?;
        if !(4..=5).contains(&o.len())
            || o.keys().any(|k| {
                !["project_id", "commit", "query", "limit", "task_ref"].contains(&k.as_str())
            })
        {
            return None;
        }
        let project_id = o.get("project_id")?.as_str()?;
        ProjectId::new(project_id).ok()?;
        let commit = o.get("commit")?.as_str()?;
        GitObjectId::new(commit).ok()?;
        let query = o.get("query")?.as_str()?;
        if query.is_empty()
            || query.len() > 512
            || query.chars().count() > 128
            || query.trim() != query
            || query.chars().any(char::is_control)
        {
            return None;
        }
        let limit = i32::try_from(o.get("limit")?.as_i64()?).ok()?;
        if !(1..=32).contains(&limit) {
            return None;
        }
        Some(Self {
            project_id: project_id.to_owned(),
            commit: commit.to_owned(),
            query: query.to_owned(),
            limit,
            task_ref: optional_task_ref(o.get("task_ref"))?,
        })
    }
}

/// Selects server-observed usage; task/project authority must be checked in PostgreSQL.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphUsageArguments {
    pub(crate) project_id: String,
    pub(crate) task_ref: Option<String>,
}

impl GraphUsageArguments {
    pub(crate) fn from_value(value: Option<&Value>) -> Option<Self> {
        let o = value?.as_object()?;
        if !(1..=2).contains(&o.len())
            || o.keys()
                .any(|k| !["project_id", "task_ref"].contains(&k.as_str()))
        {
            return None;
        }
        let project_id = o.get("project_id")?.as_str()?;
        ProjectId::new(project_id).ok()?;
        Some(Self {
            project_id: project_id.to_owned(),
            task_ref: optional_task_ref(o.get("task_ref"))?,
        })
    }
}

fn optional_task_ref(value: Option<&Value>) -> Option<Option<String>> {
    let Some(value) = value else {
        return Some(None);
    };
    let task_ref = value.as_str()?;
    if task_ref.len() != 64
        || !task_ref
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return None;
    }
    Some(Some(task_ref.to_owned()))
}

pub(crate) fn schema() -> Value {
    json!({"type":"object","additionalProperties":false,"required":["project_id","commit","query","limit"],"properties":{
        "project_id":{"type":"string","minLength":2,"maxLength":64,"pattern":"^[a-z0-9][a-z0-9._-]{1,63}$"},
        "commit":{"type":"string","pattern":"^([0-9a-f]{40}|[0-9a-f]{64})$"},
        "query":{"type":"string","minLength":1,"maxLength":128,"description":"Literal case-insensitive substring of subject, object, relation or source path."},
        "limit":{"type":"integer","minimum":1,"maximum":32},
        "task_ref":{"type":"string","minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$","description":"Optional task association; Runtime verifies the task belongs to project_id."}}})
}

pub(crate) fn graph_usage_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"required":["project_id"],"properties":{
        "project_id":{"type":"string","minLength":2,"maxLength":64,"pattern":"^[a-z0-9][a-z0-9._-]{1,63}$"},
        "task_ref":{"type":"string","minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$","description":"Optional task filter; Runtime verifies the task belongs to project_id."}}})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graph_selectors_validate_optional_task_ref_without_accepting_authority_fields() {
        let relations =
            json!({"project_id":"customer-test","commit":"a".repeat(40),"query":"name","limit":1});
        let usage = json!({"project_id":"customer-test"});
        assert_eq!(
            CodeRelationsArguments::from_value(Some(&relations))
                .unwrap()
                .task_ref,
            None
        );
        assert_eq!(
            GraphUsageArguments::from_value(Some(&usage))
                .unwrap()
                .task_ref,
            None
        );
        for task_ref in [json!("0123456789abcdef".repeat(4)), json!("0".repeat(64))] {
            let mut r = relations.clone();
            let mut u = usage.clone();
            r["task_ref"] = task_ref.clone();
            u["task_ref"] = task_ref.clone();
            assert_eq!(
                CodeRelationsArguments::from_value(Some(&r))
                    .unwrap()
                    .task_ref
                    .as_deref(),
                task_ref.as_str()
            );
            let parsed = GraphUsageArguments::from_value(Some(&u)).unwrap();
            assert_eq!(parsed.task_ref.as_deref(), task_ref.as_str());
            assert_eq!(parsed.project_id, "customer-test");
        }
        for task_ref in [
            Value::Null,
            json!(1),
            json!("A".repeat(64)),
            json!("g".repeat(64)),
            json!("a".repeat(63)),
            json!("a".repeat(65)),
            json!(" a".repeat(32)),
        ] {
            let mut r = relations.clone();
            let mut u = usage.clone();
            r["task_ref"] = task_ref.clone();
            u["task_ref"] = task_ref;
            assert!(CodeRelationsArguments::from_value(Some(&r)).is_none());
            assert!(GraphUsageArguments::from_value(Some(&u)).is_none());
        }
        for value in [
            Value::Null,
            json!([]),
            json!({}),
            json!({"task_ref":"a".repeat(64)}),
            json!({"project_id":"../other"}),
            json!({"project_id":"customer-test","source_root":"C:/other"}),
            json!({"project_id":"customer-test","task_ref":"a".repeat(64),"verified":true}),
        ] {
            assert!(
                GraphUsageArguments::from_value(Some(&value)).is_none(),
                "{value}"
            );
        }
        assert!(GraphUsageArguments::from_value(None).is_none());
        assert_eq!(
            schema()["required"],
            json!(["project_id", "commit", "query", "limit"])
        );
        assert_eq!(graph_usage_schema()["required"], json!(["project_id"]));
        assert_eq!(graph_usage_schema()["additionalProperties"], false);
    }

    #[test]
    fn code_relations_checks_real_record_content_membership_and_proof() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/code-relations-page.json"))
                .unwrap();
        let set = fixture["set"].as_str().unwrap();
        let original = fixture["page"].clone();
        let mut valid = original.clone();
        assert_eq!(verify_page(&mut valid, set, 6, 32), Some(()));
        assert!(valid.get("record_set_proof").is_none());
        assert_eq!(valid["records"].as_array().unwrap().len(), 6);
        for (field, value) in [
            ("subject", json!("corrupt()")),
            ("source_path", json!("other.py")),
            ("line_start", json!(42)),
            ("trusted_context", json!(true)),
            ("record_id", json!("f".repeat(64))),
            ("content_digest", json!("e".repeat(64))),
            ("ordinal", json!(2)),
        ] {
            let mut page = original.clone();
            page["records"][0][field] = value;
            assert!(verify_page(&mut page, set, 6, 32).is_none(), "{field}");
        }
        let mut page = original.clone();
        page["record_set_proof"][0]["content"] = json!("a".repeat(64));
        assert!(verify_page(&mut page, set, 6, 32).is_none());
        let mut page = original.clone();
        page["record_set_proof"].as_array_mut().unwrap().pop();
        assert!(verify_page(&mut page, set, 6, 32).is_none());
        assert!(verify_page(&mut original.clone(), &"a".repeat(64), 6, 32).is_none());
        assert!(verify_page(&mut original.clone(), set, 5, 32).is_none());
        assert!(verify_page(&mut original.clone(), set, 6, 1).is_none());
    }
    #[test]
    fn code_relations_rejects_authority_injection_and_invalid_bounds() {
        let valid = json!({"project_id":"customer-test","commit":"a".repeat(40),"query":"normalize_name","limit":32});
        assert!(CodeRelationsArguments::from_value(Some(&valid)).is_some());
        for (key, value) in [
            ("limit", json!(0)),
            ("limit", json!(33)),
            ("limit", json!(1.5)),
            ("query", json!("")),
            ("query", json!("a\n")),
            ("query", json!("x".repeat(129))),
            ("query", json!(" x")),
            ("commit", json!("HEAD")),
            ("project_id", json!("../other")),
            ("source_root", json!("C:/other")),
            ("analysis_digest", json!("a".repeat(64))),
        ] {
            let mut v = valid.clone();
            v[key] = value;
            assert!(
                CodeRelationsArguments::from_value(Some(&v)).is_none(),
                "{key}: {v}"
            );
        }
        for q in ["%", "_", "' OR 1=1 --", "函式"] {
            let mut v = valid.clone();
            v["query"] = json!(q);
            assert!(CodeRelationsArguments::from_value(Some(&v)).is_some());
        }
    }
}
