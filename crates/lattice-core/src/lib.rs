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

/// Product lifecycle tier. This is independent from a component's authority
/// mode: retained code can leave the MVP without losing its stable identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentTier {
    Core,
    Optional,
    Deferred,
    Retired,
}

impl ComponentTier {
    /// Returns the stable text representation used by local inspection tools.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Optional => "optional",
            Self::Deferred => "deferred",
            Self::Retired => "retired",
        }
    }
}

/// One inert entry in the bootstrap composition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Component {
    pub id: ComponentId,
    pub mode: ComponentMode,
    pub tier: ComponentTier,
}

const COMPONENTS: [Component; 8] = [
    Component {
        id: ComponentId::RustCore,
        mode: ComponentMode::ControlCore,
        tier: ComponentTier::Core,
    },
    Component {
        id: ComponentId::OpenClaw,
        mode: ComponentMode::Gateway,
        tier: ComponentTier::Retired,
    },
    Component {
        id: ComponentId::PostgreSql,
        mode: ComponentMode::DurableTruth,
        tier: ComponentTier::Core,
    },
    Component {
        id: ComponentId::Codex,
        mode: ComponentMode::SoleWriter,
        tier: ComponentTier::Core,
    },
    Component {
        id: ComponentId::Graphify,
        mode: ComponentMode::ReadOnlyEvidence,
        tier: ComponentTier::Optional,
    },
    Component {
        id: ComponentId::Hermes,
        mode: ComponentMode::ReadOnlyEvidence,
        tier: ComponentTier::Deferred,
    },
    Component {
        id: ComponentId::CodebaseMemory,
        mode: ComponentMode::DurableMemory,
        tier: ComponentTier::Optional,
    },
    Component {
        id: ComponentId::Guardian,
        mode: ComponentMode::ApprovalGated,
        tier: ComponentTier::Deferred,
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
