//! Inert bootstrap contracts for `LATTICE DevOS`.

/// Stable identifier for a component in the approved bootstrap composition.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ComponentId {
    RustCore,
    OpenClaw,
    PostgreSql,
    Codex,
    Graphify,
    Hermes,
    CodebaseMemory,
    Guardian,
}

impl ComponentId {
    /// Returns the stable text identifier used by local inspection tools.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RustCore => "rust-core",
            Self::OpenClaw => "openclaw",
            Self::PostgreSql => "postgresql",
            Self::Codex => "codex",
            Self::Graphify => "graphify",
            Self::Hermes => "hermes",
            Self::CodebaseMemory => "codebase-memory",
            Self::Guardian => "guardian",
        }
    }
}

/// Bootstrap authority or access mode for a component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentMode {
    Gateway,
    ControlCore,
    DurableTruth,
    SoleWriter,
    ReadOnlyEvidence,
    DurableMemory,
    ApprovalGated,
}

impl ComponentMode {
    /// Returns the stable text representation used by the recovery CLI.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gateway => "gateway",
            Self::ControlCore => "control-core",
            Self::DurableTruth => "durable-truth",
            Self::SoleWriter => "sole-writer",
            Self::ReadOnlyEvidence => "read-only-evidence",
            Self::DurableMemory => "durable-memory",
            Self::ApprovalGated => "approval-gated",
        }
    }
}

/// One inert entry in the bootstrap composition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Component {
    pub id: ComponentId,
    pub mode: ComponentMode,
}

const COMPONENTS: [Component; 8] = [
    Component {
        id: ComponentId::RustCore,
        mode: ComponentMode::ControlCore,
    },
    Component {
        id: ComponentId::OpenClaw,
        mode: ComponentMode::Gateway,
    },
    Component {
        id: ComponentId::PostgreSql,
        mode: ComponentMode::DurableTruth,
    },
    Component {
        id: ComponentId::Codex,
        mode: ComponentMode::SoleWriter,
    },
    Component {
        id: ComponentId::Graphify,
        mode: ComponentMode::ReadOnlyEvidence,
    },
    Component {
        id: ComponentId::Hermes,
        mode: ComponentMode::ReadOnlyEvidence,
    },
    Component {
        id: ComponentId::CodebaseMemory,
        mode: ComponentMode::DurableMemory,
    },
    Component {
        id: ComponentId::Guardian,
        mode: ComponentMode::ApprovalGated,
    },
];

/// Returns the stable product name.
#[must_use]
pub const fn platform_name() -> &'static str {
    "LATTICE DevOS"
}

/// Returns the inert, compile-time bootstrap composition.
#[must_use]
pub const fn bootstrap_manifest() -> &'static [Component; 8] {
    &COMPONENTS
}
