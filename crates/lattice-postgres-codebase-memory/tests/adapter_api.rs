use lattice_ports::{CodebaseMemoryPort, GraphMemoryPortError, HermesReflectionMemoryPort};
use lattice_postgres_codebase_memory::{ExtensionTarget, PostgresCodebaseMemory};
use postgres::Client;

fn assert_memory_port<T: CodebaseMemoryPort>() {}
fn assert_reflection_memory_port<T: HermesReflectionMemoryPort>() {}

#[test]
fn production_adapter_owns_only_a_typed_runtime_client_and_fixed_target() {
    assert_memory_port::<PostgresCodebaseMemory>();
    assert_reflection_memory_port::<PostgresCodebaseMemory>();
    let _: fn(Client, ExtensionTarget) -> Result<PostgresCodebaseMemory, GraphMemoryPortError> =
        PostgresCodebaseMemory::new;
    let _: fn(&PostgresCodebaseMemory) -> &lattice_contracts::CodebaseMemoryPersistenceIdentity =
        PostgresCodebaseMemory::identity;
}

#[test]
fn runtime_uses_only_v3_functions_and_row_profile_replay() {
    let source = include_str!("../src/adapter.rs");
    for function in [
        "codebase_memory_persist_analysis_v3",
        "codebase_memory_persist_retrieval_v3",
        "codebase_memory_load_receipt_v3",
        "codebase_memory_persist_reflection_v3",
        "codebase_memory_load_reflection_v3",
        "openclaw_gateway_reconcile_and_claim_v3",
        "openclaw_gateway_finalize_terminal_v3",
    ] {
        assert!(source.contains(function));
    }
    assert!(source.contains("decode_row_identity"));
    assert!(source.contains("(3, 1) => CodebaseMemoryPersistenceIdentity::v1"));
    assert!(source.contains("(3, 2) => CodebaseMemoryPersistenceIdentity::v2"));
    assert!(source.contains("(5, 3) => CodebaseMemoryPersistenceIdentity::v3"));
}
