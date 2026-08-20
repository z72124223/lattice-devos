//! Pure, fail-closed work coordination decisions.
//!
//! The module consumes a typed snapshot and returns data-only recommendations.
//! It performs no I/O, persistence, resource reservation, or task execution.

use std::collections::{BTreeMap, HashMap, HashSet};

/// Declared work readiness observed by the coordination gate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoordinationWorkState {
    Ready,
    Blocked,
    Unknown,
}

/// Completion state supplied by an independently verified projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionState {
    Done,
    Blocked,
    Unknown,
}

/// Evidence verification state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceState {
    Verified,
    NotVerified,
    Blocked,
    Unknown,
}

/// One referenced evidence observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceRecord {
    status: EvidenceState,
    reference: Option<String>,
}

impl EvidenceRecord {
    #[must_use]
    pub fn new<S>(status: EvidenceState, reference: Option<S>) -> Self
    where
        S: Into<String>,
    {
        Self {
            status,
            reference: reference.map(Into::into),
        }
    }
}

/// One declared work item from the current authoritative snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkItem {
    id: String,
    state: CoordinationWorkState,
    dependencies: Vec<String>,
    resources: Vec<String>,
}

impl WorkItem {
    #[must_use]
    pub fn new<I, D, DS, R, RS>(
        id: I,
        state: CoordinationWorkState,
        dependencies: D,
        resources: R,
    ) -> Self
    where
        I: Into<String>,
        D: IntoIterator<Item = DS>,
        DS: Into<String>,
        R: IntoIterator<Item = RS>,
        RS: Into<String>,
    {
        Self {
            id: id.into(),
            state,
            dependencies: dependencies.into_iter().map(Into::into).collect(),
            resources: resources.into_iter().map(Into::into).collect(),
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// One completion result registered against a declared work item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionReport {
    work_item_id: String,
    state: CompletionState,
    evidence: Vec<EvidenceRecord>,
}

impl CompletionReport {
    #[must_use]
    pub fn new<I, E>(work_item_id: I, state: CompletionState, evidence: E) -> Self
    where
        I: Into<String>,
        E: IntoIterator<Item = EvidenceRecord>,
    {
        Self {
            work_item_id: work_item_id.into(),
            state,
            evidence: evidence.into_iter().collect(),
        }
    }
}

/// Stable fail-closed reason returned by projection or round validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoordinationBlocker {
    EmptyWorkId {
        item_index: usize,
    },
    DuplicateWorkId(String),
    EmptyCompletionReportId {
        report_index: usize,
    },
    DuplicateCompletionReportId(String),
    CompletionForUndeclaredWork(String),
    WorkBlocked {
        work_item_id: String,
    },
    WorkStateUnknown {
        work_item_id: String,
    },
    CompletionBlocked {
        work_item_id: String,
    },
    CompletionStateUnknown {
        work_item_id: String,
    },
    MissingCompletionEvidence {
        work_item_id: String,
    },
    EvidenceNotVerified {
        work_item_id: String,
        evidence_index: usize,
        status: EvidenceState,
    },
    EvidenceReferenceMissing {
        work_item_id: String,
        evidence_index: usize,
    },
    UndeclaredDependency {
        work_item_id: String,
        dependency_id: String,
    },
    DuplicateDependency {
        work_item_id: String,
        dependency_id: String,
    },
    SelfDependency(String),
    MissingResources(String),
    InvalidResource {
        work_item_id: String,
        resource_index: usize,
    },
    DuplicateResource {
        work_item_id: String,
        resource_id: String,
    },
    DependencyNotVerifiedDone {
        work_item_id: String,
        dependency_id: String,
    },
    ResourceConflict {
        work_item_id: String,
        resource_id: String,
    },
}

/// Completion status after evidence projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectedCompletion {
    VerifiedDone,
    NotComplete,
}

/// Evidence-backed status view for one work item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkItemProjection {
    work_item_id: String,
    completion: ProjectedCompletion,
    verified_evidence: usize,
    total_evidence: usize,
    blockers: Vec<CoordinationBlocker>,
}

impl WorkItemProjection {
    #[must_use]
    pub fn work_item_id(&self) -> &str {
        &self.work_item_id
    }

    #[must_use]
    pub const fn completion(&self) -> ProjectedCompletion {
        self.completion
    }

    #[must_use]
    pub const fn verified_evidence(&self) -> usize {
        self.verified_evidence
    }

    #[must_use]
    pub const fn total_evidence(&self) -> usize {
        self.total_evidence
    }

    #[must_use]
    pub fn blockers(&self) -> &[CoordinationBlocker] {
        &self.blockers
    }
}

/// The only supported consumer boundary for a dispatch decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchBoundary {
    GovernedLatticeExecutionOnly,
}

/// Data-only candidate that may enter the existing governed execution path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchCandidate {
    work_item_id: String,
    resources: Vec<String>,
    boundary: DispatchBoundary,
}

impl DispatchCandidate {
    #[must_use]
    pub fn work_item_id(&self) -> &str {
        &self.work_item_id
    }

    #[must_use]
    pub fn resources(&self) -> &[String] {
        &self.resources
    }

    #[must_use]
    pub const fn boundary(&self) -> DispatchBoundary {
        self.boundary
    }
}

/// One non-dispatchable work item and its reasons.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockedWorkItem {
    work_item_id: String,
    blockers: Vec<CoordinationBlocker>,
}

impl BlockedWorkItem {
    #[must_use]
    pub fn work_item_id(&self) -> &str {
        &self.work_item_id
    }

    #[must_use]
    pub fn blockers(&self) -> &[CoordinationBlocker] {
        &self.blockers
    }
}

/// Data-only terminal retention recommendation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchiveDisposition {
    Archive,
    Retain,
}

/// Archive recommendation for one verified completed item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveDecision {
    work_item_id: String,
    disposition: ArchiveDisposition,
}

impl ArchiveDecision {
    #[must_use]
    pub fn work_item_id(&self) -> &str {
        &self.work_item_id
    }

    #[must_use]
    pub const fn disposition(&self) -> ArchiveDisposition {
        self.disposition
    }
}

/// Complete decision for one deterministic coordination round.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoordinationRound {
    structural_blockers: Vec<CoordinationBlocker>,
    dispatchable: Vec<DispatchCandidate>,
    blocked: Vec<BlockedWorkItem>,
    archive: Vec<ArchiveDecision>,
}

impl CoordinationRound {
    #[must_use]
    pub fn structural_blockers(&self) -> &[CoordinationBlocker] {
        &self.structural_blockers
    }

    #[must_use]
    pub fn dispatchable(&self) -> &[DispatchCandidate] {
        &self.dispatchable
    }

    #[must_use]
    pub fn blocked(&self) -> &[BlockedWorkItem] {
        &self.blocked
    }

    #[must_use]
    pub fn archive(&self) -> &[ArchiveDecision] {
        &self.archive
    }
}

/// Projects one work item and optional completion report.
#[must_use]
pub fn project_work_item_status(
    item: &WorkItem,
    report: Option<&CompletionReport>,
) -> WorkItemProjection {
    let mut blockers = match item.state {
        CoordinationWorkState::Ready => Vec::new(),
        CoordinationWorkState::Blocked => vec![CoordinationBlocker::WorkBlocked {
            work_item_id: item.id.clone(),
        }],
        CoordinationWorkState::Unknown => vec![CoordinationBlocker::WorkStateUnknown {
            work_item_id: item.id.clone(),
        }],
    };
    let Some(report) = report else {
        return WorkItemProjection {
            work_item_id: item.id.clone(),
            completion: ProjectedCompletion::NotComplete,
            verified_evidence: 0,
            total_evidence: 0,
            blockers,
        };
    };

    match report.state {
        CompletionState::Blocked => blockers.push(CoordinationBlocker::CompletionBlocked {
            work_item_id: item.id.clone(),
        }),
        CompletionState::Unknown => {
            blockers.push(CoordinationBlocker::CompletionStateUnknown {
                work_item_id: item.id.clone(),
            });
        }
        CompletionState::Done if report.evidence.is_empty() => {
            blockers.push(CoordinationBlocker::MissingCompletionEvidence {
                work_item_id: item.id.clone(),
            });
        }
        CompletionState::Done => {}
    }

    let mut verified_evidence = 0;
    for (index, evidence) in report.evidence.iter().enumerate() {
        if evidence.status != EvidenceState::Verified {
            blockers.push(CoordinationBlocker::EvidenceNotVerified {
                work_item_id: item.id.clone(),
                evidence_index: index,
                status: evidence.status,
            });
        } else if evidence.reference.as_deref().is_none_or(is_blank) {
            blockers.push(CoordinationBlocker::EvidenceReferenceMissing {
                work_item_id: item.id.clone(),
                evidence_index: index,
            });
        } else {
            verified_evidence += 1;
        }
    }
    let completion = if report.state == CompletionState::Done
        && !report.evidence.is_empty()
        && verified_evidence == report.evidence.len()
        && blockers.is_empty()
    {
        ProjectedCompletion::VerifiedDone
    } else {
        ProjectedCompletion::NotComplete
    };
    WorkItemProjection {
        work_item_id: item.id.clone(),
        completion,
        verified_evidence,
        total_evidence: report.evidence.len(),
        blockers,
    }
}

/// Computes one deterministic, data-only coordination round.
#[must_use]
pub fn decide_coordination_round(
    items: &[WorkItem],
    reports: &[CompletionReport],
) -> CoordinationRound {
    let structural_blockers = validate_structure(items, reports);
    if !structural_blockers.is_empty() {
        return CoordinationRound {
            structural_blockers,
            dispatchable: Vec::new(),
            blocked: Vec::new(),
            archive: Vec::new(),
        };
    }

    let reports_by_id = reports
        .iter()
        .map(|report| (report.work_item_id.as_str(), report))
        .collect::<HashMap<_, _>>();
    let projections = items
        .iter()
        .map(|item| project_work_item_status(item, reports_by_id.get(item.id.as_str()).copied()))
        .collect::<Vec<_>>();
    let completed = projections
        .iter()
        .filter(|projection| projection.completion == ProjectedCompletion::VerifiedDone)
        .map(|projection| projection.work_item_id.as_str())
        .collect::<HashSet<_>>();

    let mut candidate_blockers = build_candidate_blockers(items, &projections, &completed);
    add_resource_conflicts(items, &projections, &mut candidate_blockers);

    let mut dispatchable = Vec::new();
    let mut blocked = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if projections[index].completion == ProjectedCompletion::VerifiedDone {
            continue;
        }
        if candidate_blockers[index].is_empty() {
            dispatchable.push(DispatchCandidate {
                work_item_id: item.id.clone(),
                resources: item.resources.clone(),
                boundary: DispatchBoundary::GovernedLatticeExecutionOnly,
            });
        } else {
            blocked.push(BlockedWorkItem {
                work_item_id: item.id.clone(),
                blockers: std::mem::take(&mut candidate_blockers[index]),
            });
        }
    }

    let archive = build_archive_decisions(items, &completed);

    CoordinationRound {
        structural_blockers: Vec::new(),
        dispatchable,
        blocked,
        archive,
    }
}

fn build_candidate_blockers(
    items: &[WorkItem],
    projections: &[WorkItemProjection],
    completed: &HashSet<&str>,
) -> Vec<Vec<CoordinationBlocker>> {
    let mut blockers = vec![Vec::new(); items.len()];
    for (index, (item, projection)) in items.iter().zip(projections).enumerate() {
        if projection.completion == ProjectedCompletion::VerifiedDone {
            continue;
        }
        blockers[index].extend(projection.blockers.iter().cloned());
        for dependency in &item.dependencies {
            if !completed.contains(dependency.as_str()) {
                blockers[index].push(CoordinationBlocker::DependencyNotVerifiedDone {
                    work_item_id: item.id.clone(),
                    dependency_id: dependency.clone(),
                });
            }
        }
    }
    blockers
}

fn add_resource_conflicts(
    items: &[WorkItem],
    projections: &[WorkItemProjection],
    blockers: &mut [Vec<CoordinationBlocker>],
) {
    let mut claims: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, item) in items.iter().enumerate() {
        if projections[index].completion != ProjectedCompletion::VerifiedDone
            && blockers[index].is_empty()
        {
            for resource in &item.resources {
                claims.entry(resource).or_default().push(index);
            }
        }
    }
    for (resource, claimants) in claims {
        if claimants.len() > 1 {
            for index in claimants {
                blockers[index].push(CoordinationBlocker::ResourceConflict {
                    work_item_id: items[index].id.clone(),
                    resource_id: resource.to_owned(),
                });
            }
        }
    }
}

fn build_archive_decisions(items: &[WorkItem], completed: &HashSet<&str>) -> Vec<ArchiveDecision> {
    items
        .iter()
        .filter(|item| completed.contains(item.id.as_str()))
        .map(|item| {
            let retain = items.iter().any(|candidate| {
                !completed.contains(candidate.id.as_str())
                    && candidate
                        .dependencies
                        .iter()
                        .any(|dependency| dependency == &item.id)
            });
            ArchiveDecision {
                work_item_id: item.id.clone(),
                disposition: if retain {
                    ArchiveDisposition::Retain
                } else {
                    ArchiveDisposition::Archive
                },
            }
        })
        .collect()
}

fn validate_structure(
    items: &[WorkItem],
    reports: &[CompletionReport],
) -> Vec<CoordinationBlocker> {
    let mut blockers = Vec::new();
    let mut item_counts = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        if is_blank(&item.id) {
            blockers.push(CoordinationBlocker::EmptyWorkId { item_index: index });
        }
        *item_counts.entry(item.id.as_str()).or_insert(0_usize) += 1;
    }
    for item in items {
        if item_counts[item.id.as_str()] > 1
            && !blockers.contains(&CoordinationBlocker::DuplicateWorkId(item.id.clone()))
        {
            blockers.push(CoordinationBlocker::DuplicateWorkId(item.id.clone()));
        }
    }
    let declared_ids = items
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    for item in items {
        let mut dependencies = HashSet::new();
        for dependency in &item.dependencies {
            if dependency == &item.id {
                blockers.push(CoordinationBlocker::SelfDependency(item.id.clone()));
            } else if !declared_ids.contains(dependency.as_str()) {
                blockers.push(CoordinationBlocker::UndeclaredDependency {
                    work_item_id: item.id.clone(),
                    dependency_id: dependency.clone(),
                });
            }
            if !dependencies.insert(dependency.as_str()) {
                blockers.push(CoordinationBlocker::DuplicateDependency {
                    work_item_id: item.id.clone(),
                    dependency_id: dependency.clone(),
                });
            }
        }
        if item.resources.is_empty() {
            blockers.push(CoordinationBlocker::MissingResources(item.id.clone()));
        }
        let mut resources = HashSet::new();
        for (index, resource) in item.resources.iter().enumerate() {
            if is_blank(resource) {
                blockers.push(CoordinationBlocker::InvalidResource {
                    work_item_id: item.id.clone(),
                    resource_index: index,
                });
            }
            if !resources.insert(resource.as_str()) {
                blockers.push(CoordinationBlocker::DuplicateResource {
                    work_item_id: item.id.clone(),
                    resource_id: resource.clone(),
                });
            }
        }
    }

    let mut report_counts = HashMap::new();
    for (index, report) in reports.iter().enumerate() {
        if is_blank(&report.work_item_id) {
            blockers.push(CoordinationBlocker::EmptyCompletionReportId {
                report_index: index,
            });
        }
        if !declared_ids.contains(report.work_item_id.as_str()) {
            blockers.push(CoordinationBlocker::CompletionForUndeclaredWork(
                report.work_item_id.clone(),
            ));
        }
        *report_counts
            .entry(report.work_item_id.as_str())
            .or_insert(0_usize) += 1;
    }
    for report in reports {
        if report_counts[report.work_item_id.as_str()] > 1
            && !blockers.contains(&CoordinationBlocker::DuplicateCompletionReportId(
                report.work_item_id.clone(),
            ))
        {
            blockers.push(CoordinationBlocker::DuplicateCompletionReportId(
                report.work_item_id.clone(),
            ));
        }
    }
    blockers
}

fn is_blank(value: &str) -> bool {
    value.trim().is_empty()
}
