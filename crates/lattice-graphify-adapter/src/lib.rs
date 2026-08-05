//! Pinned, read-only Graphify extraction over exact Git object snapshots.
//!
//! This crate is an edge adapter. It owns no durable state and exposes no
//! caller-selected command, path, provider, credential, or query surface.

mod error;
mod graph;
mod identity;
mod ports;
mod process;
mod snapshot;
#[cfg(windows)]
mod windows_job;

pub use error::{GraphifyAdapterError, GraphifyAdapterErrorKind, GraphifyAdapterResult};
pub use graph::{GraphConfidence, NormalizedGraph, NormalizedGraphEdge, NormalizedGraphNode};
pub use identity::{
    GRAPHIFY_WSL_BWRAP_HELP_SHA256, GRAPHIFY_WSL_BWRAP_PATH, GRAPHIFY_WSL_BWRAP_SHA256,
    GRAPHIFY_WSL_BWRAP_VERSION, GRAPHIFY_WSL_BWRAP_VERSION_SHA256, GRAPHIFY_WSL_DISTRO,
    GRAPHIFY_WSL_EXECUTION_IDENTITY_SHA256, GRAPHIFY_WSL_GRAPHIFY_HELP_SHA256,
    GRAPHIFY_WSL_GRAPHIFY_VERSION_SHA256, GRAPHIFY_WSL_INSTALL_REPORT_SHA256,
    GRAPHIFY_WSL_LAUNCHER_SHA256, GRAPHIFY_WSL_OS_ID, GRAPHIFY_WSL_OS_RELEASE_PATH,
    GRAPHIFY_WSL_OS_RELEASE_SHA256, GRAPHIFY_WSL_OS_VERSION_ID, GRAPHIFY_WSL_PYTHON_PATH,
    GRAPHIFY_WSL_PYTHON_SHA256, GRAPHIFY_WSL_PYTHON_VERSION, GRAPHIFY_WSL_PYTHON_VERSION_SHA256,
    GRAPHIFY_WSL_REQUIRED_BWRAP_OPTIONS, GRAPHIFY_WSL_RUNTIME_BYTE_COUNT,
    GRAPHIFY_WSL_RUNTIME_FILE_COUNT, GRAPHIFY_WSL_RUNTIME_MANIFEST_SHA256,
};
pub use lattice_contracts::{
    GRAPHIFY_LICENSE, GRAPHIFY_PACKAGE, GRAPHIFY_UPSTREAM_COMMIT, GRAPHIFY_VERSION,
    GRAPHIFY_WHEEL_SHA256,
};
pub use process::{
    GraphOutputLimits, GraphifyAnalysis, GraphifyRuntimeConfig, PinnedGraphifyAdapter,
};
pub use snapshot::{
    ExactGitSnapshotMaterializer, GitSnapshotConfig, MaterializedSnapshot, SnapshotBridge,
    SnapshotLimits, SnapshotSource,
};
