//! Read-only runtime contracts for LATTICE's four-part core.

/// Stable identifier for a component in the approved bootstrap composition.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ComponentId {
    Lattice,
    PostgreSql,
    Graphify,
    Hermes,
}

impl ComponentId {
    /// Returns the stable text identifier used by local inspection tools.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lattice => "lattice",
            Self::PostgreSql => "postgresql",
            Self::Graphify => "graphify",
            Self::Hermes => "hermes",
        }
    }
}

/// Stable responsibility for a core runtime component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentMode {
    ControlCore,
    DurableTruth,
    DerivedRelationshipMemory,
    ReflectiveAdvisor,
}

impl ComponentMode {
    /// Returns the stable text representation used by the recovery CLI.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ControlCore => "control-core",
            Self::DurableTruth => "durable-truth",
            Self::DerivedRelationshipMemory => "derived-relationship-memory",
            Self::ReflectiveAdvisor => "reflective-advisor",
        }
    }
}

/// One entry in the runtime composition contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Component {
    pub id: ComponentId,
    pub mode: ComponentMode,
    pub failure_policy: ComponentFailurePolicy,
    pub recovery_action: RecoveryAction,
}

/// How the Runtime responds when a component is unavailable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentFailurePolicy {
    RuntimeUnavailable,
    Degraded,
}

impl ComponentFailurePolicy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeUnavailable => "runtime-unavailable",
            Self::Degraded => "degraded",
        }
    }
}

/// Safe recovery action for a component that has failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryAction {
    RepairControlCore,
    RestoreDurableFacts,
    RebuildFromPostgreSql,
    RecomputeFromFactsAndGraph,
}

impl RecoveryAction {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RepairControlCore => "repair-control-core",
            Self::RestoreDurableFacts => "restore-durable-facts",
            Self::RebuildFromPostgreSql => "rebuild-from-postgresql",
            Self::RecomputeFromFactsAndGraph => "recompute-from-facts-and-graph",
        }
    }
}

const COMPONENTS: [Component; 4] = [
    Component {
        id: ComponentId::Lattice,
        mode: ComponentMode::ControlCore,
        failure_policy: ComponentFailurePolicy::RuntimeUnavailable,
        recovery_action: RecoveryAction::RepairControlCore,
    },
    Component {
        id: ComponentId::PostgreSql,
        mode: ComponentMode::DurableTruth,
        failure_policy: ComponentFailurePolicy::RuntimeUnavailable,
        recovery_action: RecoveryAction::RestoreDurableFacts,
    },
    Component {
        id: ComponentId::Graphify,
        mode: ComponentMode::DerivedRelationshipMemory,
        failure_policy: ComponentFailurePolicy::Degraded,
        recovery_action: RecoveryAction::RebuildFromPostgreSql,
    },
    Component {
        id: ComponentId::Hermes,
        mode: ComponentMode::ReflectiveAdvisor,
        failure_policy: ComponentFailurePolicy::Degraded,
        recovery_action: RecoveryAction::RecomputeFromFactsAndGraph,
    },
];

/// Returns the stable product name.
#[must_use]
pub const fn platform_name() -> &'static str {
    "LATTICE Runtime"
}

/// Returns the compile-time four-part runtime composition.
#[must_use]
pub const fn bootstrap_manifest() -> &'static [Component; 4] {
    &COMPONENTS
}

/// Observed availability used to decide whether the runtime can keep serving.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentHealth {
    Healthy,
    Unavailable,
}

/// Runtime availability after applying the degradation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeState {
    Ready,
    Degraded,
    Unavailable,
}

/// Evaluates the only permitted degradation boundary.
///
/// `PostgreSQL` owns durable facts, so it is required. Graphify and Hermes are
/// derived intelligence layers: their absence lowers capability but must not
/// erase durable work or make the control core claim a data failure.
#[must_use]
pub const fn runtime_state(
    control_core: ComponentHealth,
    postgresql: ComponentHealth,
    graphify: ComponentHealth,
    hermes: ComponentHealth,
) -> RuntimeState {
    match (control_core, postgresql) {
        (ComponentHealth::Unavailable, _) | (_, ComponentHealth::Unavailable) => {
            RuntimeState::Unavailable
        }
        (ComponentHealth::Healthy, ComponentHealth::Healthy) => match (graphify, hermes) {
            (ComponentHealth::Healthy, ComponentHealth::Healthy) => RuntimeState::Ready,
            _ => RuntimeState::Degraded,
        },
    }
}
