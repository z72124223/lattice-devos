//! Program-observed graph usage. These receipts never approve task completion.
use std::fmt::Write as _;
use std::time::Instant;

use lattice_postgres_store::{PostgresControlProduct, PostgresGraphUsage};
use postgres::Client;
use serde_json::{Value, json};

pub(crate) struct GraphUsage {
    journal: PostgresGraphUsage,
    usage_id: String,
    started: Instant,
    initial_analysis_calls: u64,
    initial_query_calls: u64,
    // Runtime operations are synchronous. Prevent moving an observation to a
    // different thread, where these thread-local counters would be unrelated.
    _same_thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl GraphUsage {
    /// START must persist before the observed operation. A process crash leaves
    /// it pending, not a fabricated zero or successful terminal receipt.
    pub(crate) fn begin(client: Client, mut metadata: Value) -> Result<Self, &'static str> {
        let mut random = [0_u8; 32];
        getrandom::fill(&mut random).map_err(|_| "GRAPH_USAGE_ID_UNAVAILABLE")?;
        let mut usage_id = String::with_capacity(64);
        for byte in random {
            write!(&mut usage_id, "{byte:02x}").map_err(|_| "GRAPH_USAGE_ID_UNAVAILABLE")?;
        }
        metadata["usage_id"] = json!(usage_id);
        let mut journal = PostgresGraphUsage::new(client)?;
        journal.begin(&metadata)?;
        Ok(Self {
            journal,
            usage_id,
            started: Instant::now(),
            initial_analysis_calls: lattice_graphify_adapter::analysis_call_count(),
            initial_query_calls: PostgresControlProduct::code_relations_call_count(),
            _same_thread: std::marker::PhantomData,
        })
    }

    pub(crate) fn analysis_calls(&self) -> u64 {
        lattice_graphify_adapter::analysis_call_count() - self.initial_analysis_calls
    }

    pub(crate) fn identify_result(&self, value: &mut Value) {
        value["usage_id"] = json!(self.usage_id);
        // The value is returned only after finish succeeds.
        value["usage_status"] = json!("RECORDED");
    }

    /// Measures only the application result JSON, before MCP/JSON-RPC wrapping.
    /// A failed FINISH is exposed to the caller; the retained START stays pending.
    pub(crate) fn finish(
        mut self,
        outcome: &str,
        source_receipt_digest: Option<&str>,
        record_count: Option<usize>,
        result: Option<&Value>,
        error_code: Option<&str>,
    ) -> Result<(), &'static str> {
        let analysis_calls = self.analysis_calls();
        let query_calls =
            PostgresControlProduct::code_relations_call_count() - self.initial_query_calls;
        let duration_ms = u64::try_from(self.started.elapsed().as_millis())
            .map_err(|_| "GRAPH_USAGE_DURATION_REJECTED")?;
        self.journal.finish(&json!({
            "usage_id": self.usage_id,
            "outcome": outcome,
            "source_receipt_digest": source_receipt_digest,
            "record_count": record_count,
            "result_bytes": result.map(|value| value.to_string().len()),
            "duration_ms": duration_ms,
            "error_code": error_code,
            "analysis_calls": analysis_calls,
            "query_calls": query_calls,
        }))
    }
}
