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
