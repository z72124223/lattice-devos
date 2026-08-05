use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    CodebaseMemoryPersistenceIdentity, ContentDigest, GraphMemoryPersistenceEvidence,
    GraphMemoryReceipt, GraphMemoryRunRequest, MemoryRetrievalDisposition, MemoryRetrievalEvidence,
    MemoryRetrievalPlan, NormalizedGraphAnalysis, RankedMemoryRecord,
};
use lattice_ports::{
    CodebaseMemoryPort, GraphMemoryFailureCertainty, GraphMemoryPortError, GraphMemoryPortResult,
    GraphMemoryStage, PortErrorKind,
};
use postgres::error::SqlState;
use postgres::{Client, GenericClient, IsolationLevel, Row};

use crate::{ExtensionTarget, verify_embedded_extension_manifest};

const GLOBAL_MANIFEST_SHA256: &str =
    "09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407";

/// Production repository adapter for the fixed same-database Memory profile.
///
/// It accepts neither SQL, a path, credentials, a connection string, nor a
/// schema selector. Administrative installation remains a separate API.
pub struct PostgresCodebaseMemory {
    client: Client,
    target: ExtensionTarget,
    identity: CodebaseMemoryPersistenceIdentity,
}

impl PostgresCodebaseMemory {
    /// Binds one runtime connection to the only admitted disposable target and
    /// exact embedded extension identity.
    ///
    /// # Errors
    ///
    /// Returns a typed failure if the compile-time manifest or identity cannot
    /// be reconstructed.
    pub fn new(client: Client, target: ExtensionTarget) -> Result<Self, GraphMemoryPortError> {
        let manifest = verify_embedded_extension_manifest().map_err(|_| {
            known(
                GraphMemoryStage::Persistence,
                PortErrorKind::VersionMismatch,
                "MEMORY_ADAPTER_MANIFEST_INVALID",
            )
        })?;
        let global_manifest_digest = ContentDigest::from_sha256(GLOBAL_MANIFEST_SHA256)
            .map_err(|_| contract_error(GraphMemoryStage::Persistence))?;
        let identity = CodebaseMemoryPersistenceIdentity::v1(
            target.expected_database_identity_digest().clone(),
            global_manifest_digest,
            manifest.sql_sha256().clone(),
            manifest.manifest_sha256().clone(),
        )
        .map_err(|_| contract_error(GraphMemoryStage::Persistence))?;
        Ok(Self {
            client,
            target,
            identity,
        })
    }

    /// Returns the complete typed database/extension identity carried by every
    /// durable evidence value.
    #[must_use]
    pub const fn identity(&self) -> &CodebaseMemoryPersistenceIdentity {
        &self.identity
    }

    fn assert_target_binding(&self) -> GraphMemoryPortResult<()> {
        if self.identity.database_identity_digest()
            != self.target.expected_database_identity_digest()
        {
            return Err(known(
                GraphMemoryStage::Persistence,
                PortErrorKind::Denied,
                "MEMORY_ADAPTER_TARGET_IDENTITY_MISMATCH",
            ));
        }
        Ok(())
    }
}

impl CodebaseMemoryPort for PostgresCodebaseMemory {
    #[allow(clippy::too_many_lines)]
    fn persist_analysis(
        &mut self,
        analysis: &NormalizedGraphAnalysis,
    ) -> GraphMemoryPortResult<GraphMemoryPersistenceEvidence> {
        self.assert_target_binding()?;
        let record_count = u32::try_from(analysis.records().len())
            .map_err(|_| contract_error(GraphMemoryStage::Persistence))?;
        let persistence_digest = persistence_digest(analysis, &self.identity, record_count)?;
        let evidence = GraphMemoryPersistenceEvidence::new(
            analysis,
            self.identity.clone(),
            persistence_digest.clone(),
        )
        .map_err(|_| contract_error(GraphMemoryStage::Persistence))?;

        let identity = identity_bytes(&self.identity, GraphMemoryStage::Persistence)?;
        let invocation = analysis.request().invocation();
        let contract_version = i16::try_from(invocation.version())
            .map_err(|_| contract_error(GraphMemoryStage::Persistence))?;
        let retrieval_limit = i16::try_from(analysis.request().retrieval_limit())
            .map_err(|_| contract_error(GraphMemoryStage::Persistence))?;
        let subject_digest =
            digest_bytes(invocation.subject_digest(), GraphMemoryStage::Persistence)?;
        let query_digest = digest_bytes(
            analysis.request().query_digest(),
            GraphMemoryStage::Persistence,
        )?;
        let configuration_digest = digest_bytes(
            analysis.request().configuration_digest(),
            GraphMemoryStage::Persistence,
        )?;
        let manifest_digest =
            digest_bytes(analysis.manifest_digest(), GraphMemoryStage::Persistence)?;
        let exclusion_digest =
            digest_bytes(analysis.exclusion_digest(), GraphMemoryStage::Persistence)?;
        let graphify_identity_digest =
            digest_bytes(analysis.identity_digest(), GraphMemoryStage::Persistence)?;
        let graph_artifact_digest = digest_bytes(
            analysis.graph_artifact_digest(),
            GraphMemoryStage::Persistence,
        )?;
        let raw_output_digest =
            digest_bytes(analysis.raw_output_digest(), GraphMemoryStage::Persistence)?;
        let raw_evidence_digest = digest_bytes(
            analysis.raw_evidence_digest(),
            GraphMemoryStage::Persistence,
        )?;
        let record_set_digest =
            digest_bytes(analysis.record_set_digest(), GraphMemoryStage::Persistence)?;
        let analysis_digest =
            digest_bytes(analysis.analysis_digest(), GraphMemoryStage::Persistence)?;
        let persistence_digest_bytes =
            digest_bytes(&persistence_digest, GraphMemoryStage::Persistence)?;

        let mut ordinals = Vec::with_capacity(analysis.records().len());
        let mut record_ids = Vec::with_capacity(analysis.records().len());
        let mut graph_kinds = Vec::with_capacity(analysis.records().len());
        let mut subjects = Vec::with_capacity(analysis.records().len());
        let mut categories = Vec::with_capacity(analysis.records().len());
        let mut relations = Vec::with_capacity(analysis.records().len());
        let mut objects = Vec::with_capacity(analysis.records().len());
        let mut source_paths = Vec::with_capacity(analysis.records().len());
        let mut source_digests = Vec::with_capacity(analysis.records().len());
        let mut line_starts = Vec::with_capacity(analysis.records().len());
        let mut line_ends = Vec::with_capacity(analysis.records().len());
        let mut confidences = Vec::with_capacity(analysis.records().len());
        let mut content_digests = Vec::with_capacity(analysis.records().len());
        for record in analysis.records() {
            ordinals.push(
                i32::try_from(record.ordinal())
                    .map_err(|_| contract_error(GraphMemoryStage::Persistence))?,
            );
            record_ids.push(digest_bytes(
                record.record_id(),
                GraphMemoryStage::Persistence,
            )?);
            graph_kinds.push(record.graph_kind().as_str().to_owned());
            subjects.push(record.subject().to_owned());
            categories.push(record.category().to_owned());
            relations.push(record.relation().map(ToOwned::to_owned));
            objects.push(record.object().map(ToOwned::to_owned));
            source_paths.push(record.provenance().relative_path().to_owned());
            source_digests.push(digest_bytes(
                record.provenance().content_digest(),
                GraphMemoryStage::Persistence,
            )?);
            line_starts.push(
                record
                    .provenance()
                    .line_start()
                    .map(i32::try_from)
                    .transpose()
                    .map_err(|_| contract_error(GraphMemoryStage::Persistence))?,
            );
            line_ends.push(
                record
                    .provenance()
                    .line_end()
                    .map(i32::try_from)
                    .transpose()
                    .map_err(|_| contract_error(GraphMemoryStage::Persistence))?,
            );
            confidences.push(record.confidence().as_str().to_owned());
            content_digests.push(digest_bytes(
                record.content_digest(),
                GraphMemoryStage::Persistence,
            )?);
        }

        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::Serializable)
            .start()
            .map_err(|error| database_error(GraphMemoryStage::Persistence, &error))?;
        harden_write(&mut transaction, GraphMemoryStage::Persistence)?;
        let row = transaction
            .query_one(
                "SELECT persistence_status, record_count \
                   FROM memory.codebase_memory_persist_analysis_v1(\
                       $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,\
                       $20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,$31,$32,$33,$34,$35,$36,$37,$38\
                   )",
                &[
                    &identity.database,
                    &identity.global_manifest,
                    &identity.extension_sql,
                    &identity.extension_manifest,
                    &contract_version,
                    &invocation.request_id().as_str(),
                    &invocation.task_id().as_str(),
                    &invocation.attempt_id().as_str(),
                    &invocation.project_snapshot_id().as_str(),
                    &subject_digest,
                    &analysis.project_id().as_str(),
                    &analysis.commit_id().as_str(),
                    &query_digest,
                    &configuration_digest,
                    &retrieval_limit,
                    &analysis.tree_id().as_str(),
                    &manifest_digest,
                    &exclusion_digest,
                    &graphify_identity_digest,
                    &graph_artifact_digest,
                    &raw_output_digest,
                    &raw_evidence_digest,
                    &record_set_digest,
                    &analysis_digest,
                    &persistence_digest_bytes,
                    &ordinals,
                    &record_ids,
                    &graph_kinds,
                    &subjects,
                    &categories,
                    &relations,
                    &objects,
                    &source_paths,
                    &source_digests,
                    &line_starts,
                    &line_ends,
                    &confidences,
                    &content_digests,
                ],
            )
            .map_err(|error| database_error(GraphMemoryStage::Persistence, &error))?;
        let status: String = row.get(0);
        let returned_count: i32 = row.get(1);
        if !matches!(status.as_str(), "PERSISTED" | "REPLAYED")
            || returned_count != i32::try_from(record_count).unwrap_or(-1)
        {
            return Err(corrupt(
                GraphMemoryStage::Persistence,
                "MEMORY_PERSISTENCE_RESULT_MISMATCH",
            ));
        }
        transaction.commit().map_err(|_| {
            ambiguous(
                GraphMemoryStage::Persistence,
                "MEMORY_PERSISTENCE_COMMIT_OUTCOME_UNKNOWN",
            )
        })?;
        Ok(evidence)
    }

    #[allow(clippy::too_many_lines)]
    fn retrieve(
        &mut self,
        persistence: &GraphMemoryPersistenceEvidence,
        plan: MemoryRetrievalPlan,
    ) -> GraphMemoryPortResult<GraphMemoryReceipt> {
        self.assert_target_binding()?;
        if persistence.identity() != &self.identity {
            return Err(known(
                GraphMemoryStage::Retrieval,
                PortErrorKind::Denied,
                "MEMORY_RETRIEVAL_IDENTITY_MISMATCH",
            ));
        }
        let retrieval_digest = retrieval_digest(persistence, &plan)?;
        let retrieval = MemoryRetrievalEvidence::new(persistence, plan, retrieval_digest.clone())
            .map_err(|_| contract_error(GraphMemoryStage::Retrieval))?;
        let receipt_digest = receipt_digest(persistence, &retrieval)?;
        let expected = GraphMemoryReceipt::new(
            persistence.clone(),
            retrieval.clone(),
            receipt_digest.clone(),
        )
        .map_err(|_| contract_error(GraphMemoryStage::Retrieval))?;

        let identity = identity_bytes(&self.identity, GraphMemoryStage::Retrieval)?;
        let analysis_digest =
            digest_bytes(persistence.analysis_digest(), GraphMemoryStage::Retrieval)?;
        let persistence_digest_bytes = digest_bytes(
            persistence.persistence_digest(),
            GraphMemoryStage::Retrieval,
        )?;
        let query_digest = digest_bytes(retrieval.query_digest(), GraphMemoryStage::Retrieval)?;
        let retrieval_limit = i16::try_from(retrieval.limit())
            .map_err(|_| contract_error(GraphMemoryStage::Retrieval))?;
        let result_record_ids = retrieval
            .results()
            .iter()
            .map(|result| digest_bytes(result.record_id(), GraphMemoryStage::Retrieval))
            .collect::<Result<Vec<_>, _>>()?;
        let result_record_digests = retrieval
            .results()
            .iter()
            .map(|result| digest_bytes(result.record_digest(), GraphMemoryStage::Retrieval))
            .collect::<Result<Vec<_>, _>>()?;
        let result_scores = retrieval
            .results()
            .iter()
            .map(|result| i64::from(result.score()))
            .collect::<Vec<_>>();
        let result_set_digest =
            digest_bytes(retrieval.result_set_digest(), GraphMemoryStage::Retrieval)?;
        let retrieval_digest_bytes = digest_bytes(&retrieval_digest, GraphMemoryStage::Retrieval)?;
        let receipt_digest_bytes = digest_bytes(&receipt_digest, GraphMemoryStage::Retrieval)?;

        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::Serializable)
            .start()
            .map_err(|error| database_error(GraphMemoryStage::Retrieval, &error))?;
        harden_write(&mut transaction, GraphMemoryStage::Retrieval)?;
        let row = transaction
            .query_one(
                "SELECT retrieval_status \
                   FROM memory.codebase_memory_persist_retrieval_v1(\
                       $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15\
                   )",
                &[
                    &identity.database,
                    &identity.global_manifest,
                    &identity.extension_sql,
                    &identity.extension_manifest,
                    &analysis_digest,
                    &persistence_digest_bytes,
                    &query_digest,
                    &retrieval_limit,
                    &retrieval.disposition().as_str(),
                    &result_record_ids,
                    &result_record_digests,
                    &result_scores,
                    &result_set_digest,
                    &retrieval_digest_bytes,
                    &receipt_digest_bytes,
                ],
            )
            .map_err(|error| database_error(GraphMemoryStage::Retrieval, &error))?;
        let status: String = row.get(0);
        if !matches!(status.as_str(), "PERSISTED" | "REPLAYED") {
            return Err(corrupt(
                GraphMemoryStage::Retrieval,
                "MEMORY_RETRIEVAL_RESULT_MISMATCH",
            ));
        }
        transaction.commit().map_err(|_| {
            ambiguous(
                GraphMemoryStage::Retrieval,
                "MEMORY_RETRIEVAL_COMMIT_OUTCOME_UNKNOWN",
            )
        })?;

        let replayed = self.load_receipt(persistence.request())?;
        if replayed != expected {
            return Err(corrupt(
                GraphMemoryStage::Receipt,
                "MEMORY_RECEIPT_POST_WRITE_MISMATCH",
            ));
        }
        Ok(replayed)
    }

    fn load_receipt(
        &mut self,
        request: &GraphMemoryRunRequest,
    ) -> GraphMemoryPortResult<GraphMemoryReceipt> {
        self.assert_target_binding().map_err(|_| {
            known(
                GraphMemoryStage::Receipt,
                PortErrorKind::Denied,
                "MEMORY_RECEIPT_TARGET_IDENTITY_MISMATCH",
            )
        })?;
        let identity = identity_bytes(&self.identity, GraphMemoryStage::Receipt)?;
        let invocation = request.invocation();
        let contract_version = i16::try_from(invocation.version())
            .map_err(|_| contract_error(GraphMemoryStage::Receipt))?;
        let subject_digest = digest_bytes(invocation.subject_digest(), GraphMemoryStage::Receipt)?;
        let query_digest = digest_bytes(request.query_digest(), GraphMemoryStage::Receipt)?;
        let configuration_digest =
            digest_bytes(request.configuration_digest(), GraphMemoryStage::Receipt)?;
        let retrieval_limit = i16::try_from(request.retrieval_limit())
            .map_err(|_| contract_error(GraphMemoryStage::Receipt))?;

        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(|error| database_error(GraphMemoryStage::Receipt, &error))?;
        harden_read(&mut transaction)?;
        let rows = transaction
            .query(
                "SELECT analysis_digest, record_set_digest, record_count, persistence_digest, \
                        disposition, result_record_ids, result_record_digests, result_scores, \
                        result_set_digest, retrieval_digest, receipt_digest \
                   FROM memory.codebase_memory_load_receipt_v1(\
                       $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15\
                   )",
                &[
                    &identity.database,
                    &identity.global_manifest,
                    &identity.extension_sql,
                    &identity.extension_manifest,
                    &contract_version,
                    &invocation.request_id().as_str(),
                    &invocation.task_id().as_str(),
                    &invocation.attempt_id().as_str(),
                    &invocation.project_snapshot_id().as_str(),
                    &subject_digest,
                    &request.project_id().as_str(),
                    &request.commit_id().as_str(),
                    &query_digest,
                    &configuration_digest,
                    &retrieval_limit,
                ],
            )
            .map_err(|error| database_error(GraphMemoryStage::Receipt, &error))?;
        if rows.len() != 1 {
            return Err(corrupt(
                GraphMemoryStage::Receipt,
                "MEMORY_RECEIPT_CARDINALITY_MISMATCH",
            ));
        }
        let receipt = decode_receipt(request, self.identity.clone(), &rows[0])?;
        transaction
            .commit()
            .map_err(|error| database_error(GraphMemoryStage::Receipt, &error))?;
        Ok(receipt)
    }
}

struct IdentityBytes {
    database: Vec<u8>,
    global_manifest: Vec<u8>,
    extension_sql: Vec<u8>,
    extension_manifest: Vec<u8>,
}

fn identity_bytes(
    identity: &CodebaseMemoryPersistenceIdentity,
    stage: GraphMemoryStage,
) -> GraphMemoryPortResult<IdentityBytes> {
    Ok(IdentityBytes {
        database: digest_bytes(identity.database_identity_digest(), stage)?,
        global_manifest: digest_bytes(identity.global_manifest_digest(), stage)?,
        extension_sql: digest_bytes(identity.extension_sql_digest(), stage)?,
        extension_manifest: digest_bytes(identity.extension_manifest_digest(), stage)?,
    })
}

fn decode_receipt(
    request: &GraphMemoryRunRequest,
    identity: CodebaseMemoryPersistenceIdentity,
    row: &Row,
) -> GraphMemoryPortResult<GraphMemoryReceipt> {
    let analysis_digest = row_digest(row, 0)?;
    let record_set_digest = row_digest(row, 1)?;
    let record_count: i32 = row.get(2);
    let record_count = u32::try_from(record_count).map_err(|_| {
        corrupt(
            GraphMemoryStage::Receipt,
            "MEMORY_RECEIPT_RECORD_COUNT_INVALID",
        )
    })?;
    let persistence_digest = row_digest(row, 3)?;
    let disposition: String = row.get(4);
    let disposition = match disposition.as_str() {
        "RESULTS" => MemoryRetrievalDisposition::Results,
        "NO_ANSWER" => MemoryRetrievalDisposition::NoAnswer,
        _ => {
            return Err(corrupt(
                GraphMemoryStage::Receipt,
                "MEMORY_RECEIPT_DISPOSITION_INVALID",
            ));
        }
    };
    let record_ids: Vec<Vec<u8>> = row.get(5);
    let record_digests: Vec<Vec<u8>> = row.get(6);
    let scores: Vec<i64> = row.get(7);
    if record_ids.len() != record_digests.len() || record_ids.len() != scores.len() {
        return Err(corrupt(
            GraphMemoryStage::Receipt,
            "MEMORY_RECEIPT_RESULT_ARRAY_MISMATCH",
        ));
    }
    let mut results = Vec::with_capacity(record_ids.len());
    for (index, ((record_id, record_digest), score)) in record_ids
        .into_iter()
        .zip(record_digests)
        .zip(scores)
        .enumerate()
    {
        let rank = u16::try_from(index + 1)
            .map_err(|_| corrupt(GraphMemoryStage::Receipt, "MEMORY_RECEIPT_RANK_INVALID"))?;
        let score = u32::try_from(score)
            .map_err(|_| corrupt(GraphMemoryStage::Receipt, "MEMORY_RECEIPT_SCORE_INVALID"))?;
        results.push(
            RankedMemoryRecord::replay(
                bytes_digest(&record_id)?,
                bytes_digest(&record_digest)?,
                rank,
                score,
            )
            .map_err(|_| corrupt(GraphMemoryStage::Receipt, "MEMORY_RECEIPT_RESULT_INVALID"))?,
        );
    }
    let result_set_digest = row_digest(row, 8)?;
    let retrieval_digest = row_digest(row, 9)?;
    let receipt_digest = row_digest(row, 10)?;
    let persistence = GraphMemoryPersistenceEvidence::replay(
        request.clone(),
        identity,
        analysis_digest,
        record_set_digest,
        record_count,
        persistence_digest,
    )
    .map_err(|_| {
        corrupt(
            GraphMemoryStage::Receipt,
            "MEMORY_RECEIPT_PERSISTENCE_INVALID",
        )
    })?;
    let retrieval = MemoryRetrievalEvidence::replay(
        &persistence,
        request.retrieval_limit(),
        disposition,
        results,
        result_set_digest,
        retrieval_digest,
    )
    .map_err(|_| {
        corrupt(
            GraphMemoryStage::Receipt,
            "MEMORY_RECEIPT_RETRIEVAL_INVALID",
        )
    })?;
    GraphMemoryReceipt::new(persistence, retrieval, receipt_digest)
        .map_err(|_| corrupt(GraphMemoryStage::Receipt, "MEMORY_RECEIPT_BINDING_INVALID"))
}

fn persistence_digest(
    analysis: &NormalizedGraphAnalysis,
    identity: &CodebaseMemoryPersistenceIdentity,
    record_count: u32,
) -> GraphMemoryPortResult<ContentDigest> {
    hash(
        GraphMemoryStage::Persistence,
        "lattice.postgres-codebase-memory.persistence",
        &CanonicalValue::Object(vec![
            (
                "analysis".to_owned(),
                string(analysis.analysis_digest().as_str()),
            ),
            ("commit".to_owned(), string(analysis.commit_id().as_str())),
            (
                "configuration".to_owned(),
                string(analysis.request().configuration_digest().as_str()),
            ),
            ("identity".to_owned(), identity_value(identity)),
            ("project".to_owned(), string(analysis.project_id().as_str())),
            (
                "query".to_owned(),
                string(analysis.request().query_digest().as_str()),
            ),
            ("record_count".to_owned(), string(record_count.to_string())),
            (
                "record_set".to_owned(),
                string(analysis.record_set_digest().as_str()),
            ),
            ("request".to_owned(), request_value(analysis.request())),
        ]),
    )
}

fn retrieval_digest(
    persistence: &GraphMemoryPersistenceEvidence,
    plan: &MemoryRetrievalPlan,
) -> GraphMemoryPortResult<ContentDigest> {
    hash(
        GraphMemoryStage::Retrieval,
        "lattice.postgres-codebase-memory.retrieval",
        &CanonicalValue::Object(vec![
            ("algorithm".to_owned(), string(plan.algorithm())),
            (
                "analysis".to_owned(),
                string(plan.analysis_digest().as_str()),
            ),
            (
                "disposition".to_owned(),
                string(plan.disposition().as_str()),
            ),
            (
                "identity".to_owned(),
                identity_value(persistence.identity()),
            ),
            ("limit".to_owned(), string(plan.limit().to_string())),
            (
                "persistence".to_owned(),
                string(persistence.persistence_digest().as_str()),
            ),
            ("query".to_owned(), string(plan.query_digest().as_str())),
            (
                "result_set".to_owned(),
                string(plan.result_set_digest().as_str()),
            ),
            ("results".to_owned(), results_value(plan.results())),
        ]),
    )
}

fn receipt_digest(
    persistence: &GraphMemoryPersistenceEvidence,
    retrieval: &MemoryRetrievalEvidence,
) -> GraphMemoryPortResult<ContentDigest> {
    hash(
        GraphMemoryStage::Retrieval,
        "lattice.postgres-codebase-memory.receipt",
        &CanonicalValue::Object(vec![
            (
                "analysis".to_owned(),
                string(persistence.analysis_digest().as_str()),
            ),
            (
                "identity".to_owned(),
                identity_value(persistence.identity()),
            ),
            (
                "persistence".to_owned(),
                string(persistence.persistence_digest().as_str()),
            ),
            (
                "query".to_owned(),
                string(retrieval.query_digest().as_str()),
            ),
            (
                "retrieval".to_owned(),
                string(retrieval.retrieval_digest().as_str()),
            ),
        ]),
    )
}

fn request_value(request: &GraphMemoryRunRequest) -> CanonicalValue {
    let invocation = request.invocation();
    CanonicalValue::Object(vec![
        (
            "attempt".to_owned(),
            string(invocation.attempt_id().as_str()),
        ),
        (
            "contract".to_owned(),
            string(invocation.version().to_string()),
        ),
        (
            "project_snapshot".to_owned(),
            string(invocation.project_snapshot_id().as_str()),
        ),
        (
            "request".to_owned(),
            string(invocation.request_id().as_str()),
        ),
        (
            "retrieval_limit".to_owned(),
            string(request.retrieval_limit().to_string()),
        ),
        (
            "subject".to_owned(),
            string(invocation.subject_digest().as_str()),
        ),
        ("task".to_owned(), string(invocation.task_id().as_str())),
    ])
}

fn identity_value(identity: &CodebaseMemoryPersistenceIdentity) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "database".to_owned(),
            string(identity.database_identity_digest().as_str()),
        ),
        ("extension_id".to_owned(), string(identity.extension_id())),
        (
            "extension_manifest".to_owned(),
            string(identity.extension_manifest_digest().as_str()),
        ),
        (
            "extension_schema".to_owned(),
            string(identity.extension_schema_version().to_string()),
        ),
        (
            "extension_sql".to_owned(),
            string(identity.extension_sql_digest().as_str()),
        ),
        (
            "global_manifest".to_owned(),
            string(identity.global_manifest_digest().as_str()),
        ),
        (
            "global_schema".to_owned(),
            string(identity.global_schema_version().to_string()),
        ),
    ])
}

fn results_value(results: &[RankedMemoryRecord]) -> CanonicalValue {
    CanonicalValue::Array(
        results
            .iter()
            .map(|result| {
                CanonicalValue::Object(vec![
                    ("digest".to_owned(), string(result.record_digest().as_str())),
                    ("id".to_owned(), string(result.record_id().as_str())),
                    ("rank".to_owned(), string(result.rank().to_string())),
                    ("score".to_owned(), string(result.score().to_string())),
                ])
            })
            .collect(),
    )
}

fn hash(
    stage: GraphMemoryStage,
    schema_id: &str,
    value: &CanonicalValue,
) -> GraphMemoryPortResult<ContentDigest> {
    let domain = HashDomain::new(schema_id, "1").map_err(|_| contract_error(stage))?;
    let digest = canonical_sha256(&domain, value).map_err(|_| contract_error(stage))?;
    ContentDigest::from_sha256(digest.to_hex()).map_err(|_| contract_error(stage))
}

fn string(value: impl Into<String>) -> CanonicalValue {
    CanonicalValue::String(value.into())
}

fn harden_write(
    client: &mut impl GenericClient,
    stage: GraphMemoryStage,
) -> GraphMemoryPortResult<()> {
    client
        .batch_execute(
            "SET LOCAL search_path = pg_catalog; \
             SET LOCAL lock_timeout = '5s'; \
             SET LOCAL statement_timeout = '30s'; \
             SET LOCAL idle_in_transaction_session_timeout = '30s'; \
             SET LOCAL synchronous_commit = on;",
        )
        .map_err(|error| database_error(stage, &error))
}

fn harden_read(client: &mut impl GenericClient) -> GraphMemoryPortResult<()> {
    client
        .batch_execute(
            "SET LOCAL search_path = pg_catalog; \
             SET LOCAL lock_timeout = '5s'; \
             SET LOCAL statement_timeout = '30s'; \
             SET LOCAL idle_in_transaction_session_timeout = '30s';",
        )
        .map_err(|error| database_error(GraphMemoryStage::Receipt, &error))
}

fn digest_bytes(digest: &ContentDigest, stage: GraphMemoryStage) -> GraphMemoryPortResult<Vec<u8>> {
    hex_bytes(digest.as_str()).ok_or_else(|| contract_error(stage))
}

fn bytes_digest(bytes: &[u8]) -> GraphMemoryPortResult<ContentDigest> {
    if bytes.len() != 32 {
        return Err(corrupt(
            GraphMemoryStage::Receipt,
            "MEMORY_RECEIPT_DIGEST_LENGTH_INVALID",
        ));
    }
    let mut output = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    let digest = ContentDigest::from_sha256(output)
        .map_err(|_| corrupt(GraphMemoryStage::Receipt, "MEMORY_RECEIPT_DIGEST_INVALID"))?;
    if digest.as_str().bytes().all(|byte| byte == b'0') {
        return Err(corrupt(
            GraphMemoryStage::Receipt,
            "MEMORY_RECEIPT_ZERO_DIGEST",
        ));
    }
    Ok(digest)
}

fn row_digest(row: &Row, index: usize) -> GraphMemoryPortResult<ContentDigest> {
    let bytes: Vec<u8> = row.get(index);
    bytes_digest(&bytes)
}

fn hex_bytes(value: &str) -> Option<Vec<u8>> {
    if value.len() != 64 {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_nibble(pair[0])?;
            let low = hex_nibble(pair[1])?;
            Some((high << 4) | low)
        })
        .collect()
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn database_error(stage: GraphMemoryStage, error: &postgres::Error) -> GraphMemoryPortError {
    match error.code().map(SqlState::code) {
        Some("LCM01") => known(
            stage,
            PortErrorKind::Denied,
            "MEMORY_RUNTIME_BOUNDARY_DENIED",
        ),
        Some("LCM02") => known(
            stage,
            PortErrorKind::VersionMismatch,
            "MEMORY_IDENTITY_MISMATCH",
        ),
        Some("LCM03") => known(stage, PortErrorKind::Malformed, "MEMORY_INPUT_REJECTED"),
        Some("LCM04") => known(stage, PortErrorKind::Denied, "MEMORY_SUBSTITUTION_REJECTED"),
        Some("LCM05") => corrupt(stage, "MEMORY_RECORD_SET_CORRUPT"),
        Some("LCM06") => known(
            stage,
            PortErrorKind::Unavailable,
            "MEMORY_RECEIPT_UNAVAILABLE",
        ),
        Some("40001") => known(
            stage,
            PortErrorKind::Unavailable,
            "MEMORY_SERIALIZATION_RETRY_REQUIRED",
        ),
        Some("42501") => known(
            stage,
            PortErrorKind::Denied,
            "MEMORY_DATABASE_PERMISSION_DENIED",
        ),
        _ => known(
            stage,
            PortErrorKind::Unavailable,
            "MEMORY_DATABASE_OPERATION_FAILED",
        ),
    }
}

fn contract_error(stage: GraphMemoryStage) -> GraphMemoryPortError {
    known(
        stage,
        PortErrorKind::Malformed,
        "MEMORY_ADAPTER_CONTRACT_REJECTED",
    )
}

fn corrupt(stage: GraphMemoryStage, code: &'static str) -> GraphMemoryPortError {
    known(stage, PortErrorKind::Malformed, code)
}

fn known(stage: GraphMemoryStage, kind: PortErrorKind, code: &'static str) -> GraphMemoryPortError {
    GraphMemoryPortError::new(stage, kind, GraphMemoryFailureCertainty::Known, code)
}

fn ambiguous(stage: GraphMemoryStage, code: &'static str) -> GraphMemoryPortError {
    GraphMemoryPortError::new(
        stage,
        PortErrorKind::Ambiguous,
        GraphMemoryFailureCertainty::Ambiguous,
        code,
    )
}
