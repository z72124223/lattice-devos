//! Decision rows are project-owned; their shared head and retired identities are not.
use super::project_purge::{Result, db};
use postgres::GenericClient;
use serde_json::{Value, json};

const HEAD: &str = "SELECT to_jsonb(s) FROM ONLY control_product.decision_state s WHERE singleton";
const DIGEST: &str = "SELECT encode(sha256(convert_to(COALESCE(jsonb_agg(jsonb_build_array(decision_id,request_digest) ORDER BY decision_id COLLATE \"C\"),'[]'::jsonb)::text,'UTF8')),'hex') FROM ONLY control_product.decisions WHERE source IN('user_confirmation','approved_document')";
const RETIRED: &str = "SELECT kind,key_digest FROM ONLY control_product.decision_retired_keys ORDER BY kind COLLATE \"C\",key_digest COLLATE \"C\"";

pub(super) struct DecisionPurge {
    before: Value,
    after: Value,
    retired: Vec<(String, String)>,
    new_keys: Vec<(String, String)>,
}

impl DecisionPurge {
    pub(super) fn prepare<C: GenericClient>(
        client: &mut C,
        project: &str,
        installed: bool,
    ) -> Result<Option<Self>> {
        let rows = db(client.query("SELECT decision_id,client_request_id FROM ONLY control_product.decisions WHERE project_id=$1 ORDER BY decision_id COLLATE \"C\"", &[&project]))?;
        if rows.is_empty() {
            return Ok(None);
        }
        let before: Value = db(client.query_one(HEAD, &[]))?.get(0);
        let actual: String = db(client.query_one(DIGEST, &[]))?.get(0);
        let max: i64 = db(client.query_one(
            "SELECT COALESCE(max(decision_sequence),0) FROM ONLY control_product.decisions",
            &[],
        ))?
        .get(0);
        if before["digest"] != actual
            || before["revision"]
                .as_i64()
                .is_none_or(|revision| revision < max)
        {
            return Err("PROJECT_PURGE_DECISION_HEAD_CORRUPT");
        }
        let mut after = before.clone();
        after["digest"] = json!(
            db(client.query_one(&format!("{DIGEST} AND project_id<>$1"), &[&project]))?
                .get::<_, String>(0)
        );
        // revision is the writer's high-water mark, not the live row count.
        // Retain it even when the highest sequence is erased; digest invalidates old packets.
        let mut new_keys = Vec::new();
        for row in rows {
            for (kind, domain, index) in [
                ("decision", "lattice.decision.retired-id.v1", 0),
                ("request", "lattice.decision.retired-request.v1", 1),
            ] {
                new_keys.push((
                    kind.to_owned(),
                    super::project_purge::digest(
                        format!("{domain}\n{}", row.get::<_, String>(index)).as_bytes(),
                    ),
                ));
            }
        }
        let mut retired = if installed {
            read_retired(client)?
        } else {
            Vec::new()
        };
        retired.extend(new_keys.iter().cloned());
        retired.sort();
        retired.dedup();
        Ok(Some(Self {
            before,
            after,
            retired,
            new_keys,
        }))
    }

    pub(super) fn history(&self) -> Value {
        json!({"before":self.before,"after":self.after,"retiredKeys":self.new_keys.len(),
            "revisionSemantics":"MONOTONIC_WRITER_HIGH_WATER_MARK","identifierRetention":"DOMAIN_SHA256_ONLY_NOT_ANONYMOUS"})
    }

    pub(super) fn apply<C: GenericClient>(&self, client: &mut C) -> Result<()> {
        if db(client.query_one(HEAD, &[]))?.get::<_, Value>(0) != self.before {
            return Err("PROJECT_PURGE_DECISION_HEAD_CHANGED");
        }
        for (kind, key) in &self.new_keys {
            db(client.execute("INSERT INTO control_product.decision_retired_keys(kind,key_digest) VALUES($1,$2) ON CONFLICT DO NOTHING", &[kind,key]))?;
        }
        db(client.execute(
            "UPDATE ONLY control_product.decision_state SET digest=$1 WHERE singleton",
            &[&self.after["digest"]
                .as_str()
                .ok_or("PROJECT_PURGE_DECISION_HEAD_CORRUPT")?],
        ))?;
        if db(client.query_one(HEAD, &[]))?.get::<_, Value>(0) != self.after
            || db(client.query_one(DIGEST, &[]))?.get::<_, String>(0) != self.after["digest"]
            || read_retired(client)? != self.retired
        {
            return Err("PROJECT_PURGE_DECISION_READBACK_FAILED");
        }
        Ok(())
    }
}

fn read_retired<C: GenericClient>(client: &mut C) -> Result<Vec<(String, String)>> {
    Ok(db(client.query(RETIRED, &[]))?
        .into_iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect())
}

pub(super) fn shared_table(table: &str) -> bool {
    matches!(
        table,
        "control_product.decision_state" | "control_product.decision_retired_keys"
    )
}
