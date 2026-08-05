use lattice_contracts::{CodebaseMemoryPersistenceIdentity, ContentDigest};
use lattice_postgres_codebase_memory::{
    CODEBASE_MEMORY_EXTENSION_ID, CODEBASE_MEMORY_EXTENSION_PATH,
    CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION, verify_embedded_extension_manifest,
};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("digest")
}

#[test]
fn exact_extension_manifest_and_typed_identity_are_required() {
    let manifest = verify_embedded_extension_manifest().expect("exact embedded extension");
    assert_eq!(manifest.extension_id(), CODEBASE_MEMORY_EXTENSION_ID);
    assert_eq!(manifest.path(), CODEBASE_MEMORY_EXTENSION_PATH);
    assert_eq!(
        manifest.schema_version(),
        CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION
    );
    assert!(!manifest.bytes().is_empty());
    assert_eq!(manifest.byte_length(), manifest.bytes().len());
    assert_eq!(manifest.sql_sha256().as_str().len(), 64);
    assert_eq!(manifest.manifest_sha256().as_str().len(), 64);

    let identity = CodebaseMemoryPersistenceIdentity::v2(
        digest('1'),
        digest('2'),
        manifest.sql_sha256().clone(),
        manifest.manifest_sha256().clone(),
    )
    .expect("typed database and extension identity");
    assert_eq!(identity.global_schema_version(), 3);
    assert_eq!(identity.extension_id(), CODEBASE_MEMORY_EXTENSION_ID);
    assert_eq!(
        identity.extension_schema_version(),
        CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION
    );
}
