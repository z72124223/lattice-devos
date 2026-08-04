//! Context-free reconstruction of the complete fake aggregate owner.

use super::{
    ARTIFACT_STORE_PRODUCER_ID, ARTIFACT_STORE_PRODUCER_VERSION, ArtifactCommandHistory,
    ArtifactCommandKind, ArtifactCommandOutcome, ArtifactCommandReceipt, ArtifactCommandStorageKey,
    ArtifactLifecycleState, ArtifactObjectIdentity, ArtifactObjectKey, ArtifactQuotaScope,
    ArtifactStagingIdentity, ArtifactStagingReservation, ArtifactStoreCommandReceipt,
    ArtifactStoreIdentity, CanonicalValue, FakeArtifactBytes, FakeArtifactStore, HASH_VERSION,
    HashMap, HashSet, RuntimeKind, TaskId, command_outcome_text, request_source_string,
};
use crate::snapshot_contract::{parse_authority_receipt, parse_runtime};
use crate::snapshot_parse::{
    SnapshotParseError, SnapshotParseResult, StrictSnapshotObject, parse_digest, parse_i64,
    parse_limits, parse_object_identity, parse_optional_digest, parse_project_id, parse_task_id,
    parse_u64,
};
use crate::snapshot_quota::{parse_quota_head_set, parse_staging_state};

impl FakeArtifactStore {
    pub(crate) fn restore_snapshot(raw: &CanonicalValue) -> SnapshotParseResult<Self> {
        let root = StrictSnapshotObject::new(
            raw,
            &[
                "version",
                "producer_id",
                "producer_version",
                "runtime",
                "store_id",
                "limits",
                "limit_snapshot_digest",
                "lifecycle",
                "histories",
                "quota",
                "staging",
                "command_tasks",
                "retired_object_scopes",
                "terminal_receipts",
            ],
        )?;
        if root.string("version")? != HASH_VERSION
            || root.string("producer_id")? != ARTIFACT_STORE_PRODUCER_ID
            || root.string("producer_version")? != ARTIFACT_STORE_PRODUCER_VERSION
            || parse_runtime(root.string("runtime")?)? != RuntimeKind::Fake
        {
            return Err(SnapshotParseError);
        }
        let store_id = ArtifactStoreIdentity::new(root.string("store_id")?.to_owned())
            .map_err(|_| SnapshotParseError)?;
        let limits = parse_limits(root.get("limits")?)?;
        if parse_digest(root.string("limit_snapshot_digest")?)?
            != limits
                .limit_snapshot_digest()
                .map_err(|_| SnapshotParseError)?
        {
            return Err(SnapshotParseError);
        }

        let lifecycle = ArtifactLifecycleState::restore_snapshot(root.get("lifecycle")?, limits)?;
        let history = parse_histories(root.array("histories")?)?;
        let staging = parse_staging(root.array("staging")?)?;
        let command_tasks = parse_command_tasks(root.array("command_tasks")?)?;
        let quota_head_set = parse_quota_head_set(root.get("quota")?, limits)?;
        let retired_quota_objects = parse_retired_objects(root.array("retired_object_scopes")?)?;
        let terminal_receipts =
            parse_terminal_receipts(root.array("terminal_receipts")?, &history)?;

        let restored = Self {
            store_id,
            limits,
            lifecycle,
            bytes: FakeArtifactBytes::default(),
            history,
            staging,
            command_tasks,
            quota_head_set: Some(quota_head_set),
            retired_quota_objects,
            terminal_receipts,
        };
        restored.validate_restore_joins()?;
        restored
            .validate_snapshot_metadata()
            .map_err(|_| SnapshotParseError)?;
        if restored
            .snapshot_canonical_state()
            .map_err(|_| SnapshotParseError)?
            != *raw
        {
            return Err(SnapshotParseError);
        }
        Ok(restored)
    }

    fn validate_restore_joins(&self) -> SnapshotParseResult<()> {
        let history_receipts = self.history.sorted_receipts();
        if history_receipts.len() != self.command_tasks.len()
            || history_receipts.len() != self.terminal_receipts.len()
        {
            return Err(SnapshotParseError);
        }
        for history in history_receipts {
            let key = history.request().key();
            let task_id = self.command_tasks.get(key).ok_or(SnapshotParseError)?;
            if task_id.as_str()
                != request_source_string(history.request(), "task_id")
                    .map_err(|_| SnapshotParseError)?
            {
                return Err(SnapshotParseError);
            }
            let terminal = self.terminal_receipts.get(key).ok_or(SnapshotParseError)?;
            let authority_input_digest = parse_digest(
                request_source_string(history.request(), "authority_input_digest")
                    .map_err(|_| SnapshotParseError)?,
            )?;
            if terminal.history() != history
                || terminal.authority_input_digest() != &authority_input_digest
            {
                return Err(SnapshotParseError);
            }
            let lifecycle_shape = match history.outcome() {
                ArtifactCommandOutcome::Denied => terminal.lifecycle().is_none(),
                ArtifactCommandOutcome::Applied
                    if history.request().kind() == ArtifactCommandKind::Staging =>
                {
                    terminal.lifecycle().is_none()
                }
                ArtifactCommandOutcome::Applied => terminal.lifecycle().is_some(),
            };
            if !lifecycle_shape {
                return Err(SnapshotParseError);
            }
        }

        let current = self.lifecycle.current_object_identities();
        for retired in &self.retired_quota_objects {
            if current.iter().any(|object| object == retired)
                || self
                    .quota_head_set
                    .as_ref()
                    .and_then(|heads| heads.head(&ArtifactQuotaScope::Object(retired.clone())))
                    .is_none()
            {
                return Err(SnapshotParseError);
            }
            let Some(successor) = current.iter().find(|object| object.key() == retired.key())
            else {
                return Err(SnapshotParseError);
            };
            if successor.generation().get() <= retired.generation().get() {
                return Err(SnapshotParseError);
            }
        }
        let store_scope = ArtifactQuotaScope::Store(self.store_id.clone());
        if self
            .quota_head_set
            .as_ref()
            .and_then(|heads| heads.head(&store_scope))
            .is_none()
        {
            return Err(SnapshotParseError);
        }
        Ok(())
    }
}

fn parse_histories(values: &[CanonicalValue]) -> SnapshotParseResult<ArtifactCommandHistory> {
    let mut combined = ArtifactCommandHistory::new();
    for value in values {
        let row = StrictSnapshotObject::new(
            value,
            &[
                "project_id",
                "algorithm",
                "content_digest",
                "checkpoint",
                "strict_history",
            ],
        )?;
        if row.string("algorithm")? != "sha256" {
            return Err(SnapshotParseError);
        }
        let expected_scope = crate::history::ArtifactCommandObjectScope::new(
            parse_project_id(row.string("project_id")?)?,
            parse_digest(row.string("content_digest")?)?,
        );
        let (restored, scope) =
            ArtifactCommandHistory::restore_untrusted(row.get("strict_history")?)
                .map_err(|_| SnapshotParseError)?;
        if scope != expected_scope {
            return Err(SnapshotParseError);
        }
        verify_history_checkpoint(row.get("checkpoint")?, &restored, &scope)?;
        combined
            .merge_restored(restored)
            .map_err(|_| SnapshotParseError)?;
    }
    Ok(combined)
}

fn verify_history_checkpoint(
    value: &CanonicalValue,
    history: &ArtifactCommandHistory,
    scope: &crate::history::ArtifactCommandObjectScope,
) -> SnapshotParseResult<()> {
    let raw = StrictSnapshotObject::new(
        value,
        &[
            "high_water",
            "tail_digest",
            "denial_count",
            "denial_tail_digest",
            "head_digest",
            "checkpoint_digest",
        ],
    )?;
    let checkpoint = history.checkpoint(scope).map_err(|_| SnapshotParseError)?;
    let head = checkpoint.head();
    if parse_u64(raw.string("high_water")?)? != head.high_water().get()
        || parse_optional_digest(raw.get("tail_digest")?)? != head.tail_digest().cloned()
        || parse_u64(raw.string("denial_count")?)? != head.denial_count().get()
        || parse_optional_digest(raw.get("denial_tail_digest")?)?
            != head.denial_tail_digest().cloned()
        || parse_digest(raw.string("head_digest")?)? != *head.head_digest()
        || parse_digest(raw.string("checkpoint_digest")?)? != *checkpoint.checkpoint_digest()
    {
        return Err(SnapshotParseError);
    }
    Ok(())
}

fn parse_staging(
    values: &[CanonicalValue],
) -> SnapshotParseResult<HashMap<ArtifactStagingIdentity, ArtifactStagingReservation>> {
    let mut output = HashMap::with_capacity(values.len());
    for value in values {
        let row = StrictSnapshotObject::new(
            value,
            &[
                "project_id",
                "algorithm",
                "content_digest",
                "task_id",
                "reservation_id",
                "staging_bytes",
                "staging_streams",
                "status",
            ],
        )?;
        if row.string("algorithm")? != "sha256" {
            return Err(SnapshotParseError);
        }
        let identity = ArtifactStagingIdentity::new(
            ArtifactObjectKey::new(
                parse_project_id(row.string("project_id")?)?,
                parse_digest(row.string("content_digest")?)?,
            ),
            parse_task_id(row.string("task_id")?)?,
            row.string("reservation_id")?.to_owned(),
        )
        .map_err(|_| SnapshotParseError)?;
        let reservation = ArtifactStagingReservation::restore_exact(
            identity.clone(),
            parse_i64(row.string("staging_bytes")?)?,
            parse_i64(row.string("staging_streams")?)?,
            parse_staging_state(row.string("status")?)?,
        )
        .map_err(|_| SnapshotParseError)?;
        if output.insert(identity, reservation).is_some() {
            return Err(SnapshotParseError);
        }
    }
    Ok(output)
}

fn parse_command_tasks(
    values: &[CanonicalValue],
) -> SnapshotParseResult<HashMap<ArtifactCommandStorageKey, TaskId>> {
    let mut output = HashMap::with_capacity(values.len());
    for value in values {
        let row = StrictSnapshotObject::new(
            value,
            &[
                "project_id",
                "algorithm",
                "content_digest",
                "command_id",
                "task_id",
            ],
        )?;
        if row.string("algorithm")? != "sha256" {
            return Err(SnapshotParseError);
        }
        let key = ArtifactCommandStorageKey::new(
            parse_project_id(row.string("project_id")?)?,
            parse_digest(row.string("content_digest")?)?,
            row.string("command_id")?.to_owned(),
        )
        .map_err(|_| SnapshotParseError)?;
        if output
            .insert(key, parse_task_id(row.string("task_id")?)?)
            .is_some()
        {
            return Err(SnapshotParseError);
        }
    }
    Ok(output)
}

fn parse_retired_objects(
    values: &[CanonicalValue],
) -> SnapshotParseResult<HashSet<ArtifactObjectIdentity>> {
    let mut output = HashSet::with_capacity(values.len());
    for value in values {
        if !output.insert(parse_object_identity(value)?) {
            return Err(SnapshotParseError);
        }
    }
    Ok(output)
}

fn parse_terminal_receipts(
    values: &[CanonicalValue],
    history: &ArtifactCommandHistory,
) -> SnapshotParseResult<HashMap<ArtifactCommandStorageKey, ArtifactStoreCommandReceipt>> {
    let history_by_key = history
        .sorted_receipts()
        .into_iter()
        .map(|receipt| (receipt.request().key().clone(), receipt.clone()))
        .collect::<HashMap<_, _>>();
    let mut output = HashMap::with_capacity(values.len());
    for value in values {
        let row = StrictSnapshotObject::new(
            value,
            &[
                "project_id",
                "algorithm",
                "content_digest",
                "command_id",
                "producer_id",
                "producer_version",
                "runtime",
                "history_request_digest",
                "history_ordinal",
                "history_predecessor_digest",
                "history_outcome",
                "history_denial_code",
                "history_before_state_digest",
                "history_after_state_digest",
                "history_result_digest",
                "history_record_digest",
                "history_receipt_digest",
                "lifecycle_receipt",
                "lifecycle_receipt_digest",
                "authority_input_digest",
                "quota_checkpoint_digest",
                "aggregate_state_digest",
                "receipt_digest",
            ],
        )?;
        if row.string("algorithm")? != "sha256"
            || row.string("producer_id")? != ARTIFACT_STORE_PRODUCER_ID
            || row.string("producer_version")? != ARTIFACT_STORE_PRODUCER_VERSION
            || parse_runtime(row.string("runtime")?)? != RuntimeKind::Fake
        {
            return Err(SnapshotParseError);
        }
        let key = ArtifactCommandStorageKey::new(
            parse_project_id(row.string("project_id")?)?,
            parse_digest(row.string("content_digest")?)?,
            row.string("command_id")?.to_owned(),
        )
        .map_err(|_| SnapshotParseError)?;
        let history = history_by_key.get(&key).ok_or(SnapshotParseError)?;
        verify_history_mirror(&row, history)?;

        let lifecycle = match row.get("lifecycle_receipt")? {
            CanonicalValue::Null => None,
            value => Some(parse_authority_receipt(value)?),
        };
        let lifecycle_digest = parse_optional_digest(row.get("lifecycle_receipt_digest")?)?;
        if lifecycle
            .as_ref()
            .map(|receipt| receipt.receipt_digest().clone())
            != lifecycle_digest
        {
            return Err(SnapshotParseError);
        }
        let receipt = ArtifactStoreCommandReceipt::new(
            history.clone(),
            lifecycle,
            parse_digest(row.string("authority_input_digest")?)?,
            parse_digest(row.string("quota_checkpoint_digest")?)?,
            parse_digest(row.string("aggregate_state_digest")?)?,
        )
        .map_err(|_| SnapshotParseError)?;
        if receipt.receipt_digest() != &parse_digest(row.string("receipt_digest")?)?
            || output.insert(key, receipt).is_some()
        {
            return Err(SnapshotParseError);
        }
    }
    if output.len() != history_by_key.len() {
        return Err(SnapshotParseError);
    }
    Ok(output)
}

fn verify_history_mirror(
    raw: &StrictSnapshotObject<'_>,
    history: &ArtifactCommandReceipt,
) -> SnapshotParseResult<()> {
    if parse_digest(raw.string("history_request_digest")?)? != *history.request().request_digest()
        || parse_u64(raw.string("history_ordinal")?)? != history.ordinal().get()
        || parse_optional_digest(raw.get("history_predecessor_digest")?)?
            != history.predecessor_digest().cloned()
        || raw.string("history_outcome")? != command_outcome_text(history.outcome())
        || parse_optional_text(raw.get("history_denial_code")?)?
            != history.denial_code().map(str::to_owned)
        || parse_digest(raw.string("history_before_state_digest")?)?
            != *history.before_state_digest()
        || parse_digest(raw.string("history_after_state_digest")?)? != *history.after_state_digest()
        || parse_digest(raw.string("history_result_digest")?)? != *history.result_digest()
        || parse_digest(raw.string("history_record_digest")?)? != *history.record_digest()
        || parse_digest(raw.string("history_receipt_digest")?)? != *history.receipt_digest()
    {
        return Err(SnapshotParseError);
    }
    Ok(())
}

fn parse_optional_text(value: &CanonicalValue) -> SnapshotParseResult<Option<String>> {
    match value {
        CanonicalValue::Null => Ok(None),
        CanonicalValue::String(value) => Ok(Some(value.clone())),
        _ => Err(SnapshotParseError),
    }
}
