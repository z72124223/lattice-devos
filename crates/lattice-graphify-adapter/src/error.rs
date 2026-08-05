use std::error::Error;
use std::fmt;

/// Stable failure categories for the Graphify edge adapter.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GraphifyAdapterErrorKind {
    Configuration,
    GitIdentity,
    GitObject,
    UnsafeSnapshot,
    SnapshotLimit,
    SnapshotIo,
    SnapshotChanged,
    GraphifyIdentity,
    Spawn,
    Timeout,
    TeardownAmbiguous,
    NonZeroExit,
    MissingOutput,
    OutputLimit,
    MalformedOutput,
    PartialOutput,
    ForeignSource,
    EmptyAnalysis,
}

/// Fail-closed adapter error containing only a stable, non-source diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphifyAdapterError {
    kind: GraphifyAdapterErrorKind,
    code: &'static str,
}

impl GraphifyAdapterError {
    #[must_use]
    pub const fn new(kind: GraphifyAdapterErrorKind, code: &'static str) -> Self {
        Self { kind, code }
    }

    #[must_use]
    pub const fn kind(&self) -> GraphifyAdapterErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for GraphifyAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.kind, self.code)
    }
}

impl Error for GraphifyAdapterError {}

pub type GraphifyAdapterResult<T> = Result<T, GraphifyAdapterError>;
