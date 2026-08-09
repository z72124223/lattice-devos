//! Scope Check 1.1 is a pure, detection-only classifier over normalized Git evidence.

use std::error::Error;
use std::fmt;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256, normalize_nfc};
use lattice_workspace_git::{ChangeManifest, ChangeOperation, PathKind};

/// Stable fail-closed errors returned by Scope Check 1.1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeError {
    /// A configured path pattern is malformed or ambiguous.
    InvalidPattern,
    /// Caller-supplied evidence is not a normalized repository-relative path.
    InvalidEvidence,
    /// The rule set is incomplete or contains an unsupported operation.
    InvalidRules,
    /// Canonical evidence hashing failed.
    HashFailed,
}

impl ScopeError {
    /// Returns a stable machine-readable reason code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidPattern => "SCOPE_INVALID_PATTERN",
            Self::InvalidEvidence => "SCOPE_INVALID_EVIDENCE",
            Self::InvalidRules => "SCOPE_INVALID_RULES",
            Self::HashFailed => "SCOPE_HASH_FAILED",
        }
    }
}

impl fmt::Display for ScopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for ScopeError {}

/// Versioned language-neutral path-pattern semantics.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PatternKind {
    /// Match one exact repository-relative path.
    Exact,
    /// Match one complete segment prefix and all paths beneath it.
    Prefix,
    /// Match `*`, `?`, and whole-segment `**` wildcards.
    Glob,
}

impl PatternKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "EXACT",
            Self::Prefix => "PREFIX",
            Self::Glob => "GLOB",
        }
    }
}

/// One validated repository-relative path pattern.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PathPattern {
    kind: PatternKind,
    value: String,
}

fn validate_repository_path(value: &str) -> Result<(), ScopeError> {
    if value.is_empty()
        || value.len() > 4096
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains(['\0', '\\'])
        || value.chars().any(char::is_control)
        || normalize_nfc(value) != value
    {
        return Err(ScopeError::InvalidEvidence);
    }
    let mut segments = value.split('/');
    if segments.clone().count() > 256
        || segments.any(|segment| {
            segment.is_empty()
                || matches!(segment, "." | "..")
                || segment.ends_with(['.', ' '])
                || is_windows_reserved_component(segment)
                || segment.eq_ignore_ascii_case(".git")
                || segment.contains(':')
        })
    {
        return Err(ScopeError::InvalidEvidence);
    }
    Ok(())
}

fn is_windows_reserved_component(component: &str) -> bool {
    let base = component.split('.').next().unwrap_or(component);
    let upper = base.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
    ) {
        return true;
    }
    let numbered = upper.as_bytes();
    (numbered.len() == 4
        && matches!(&numbered[..3], b"COM" | b"LPT")
        && matches!(numbered[3], b'1'..=b'9'))
        || matches!(
            upper.as_str(),
            "COM¹" | "COM²" | "COM³" | "LPT¹" | "LPT²" | "LPT³"
        )
}

impl PathPattern {
    /// Constructs one validated version-1.1 pattern.
    ///
    /// # Errors
    ///
    /// Absolute, traversal, `.git`, drive-qualified, backslash-bearing,
    /// non-NFC, or ambiguous wildcard input fails closed.
    pub fn new(kind: PatternKind, value: impl Into<String>) -> Result<Self, ScopeError> {
        let value = value.into();
        validate_repository_path(&value).map_err(|_| ScopeError::InvalidPattern)?;
        match kind {
            PatternKind::Exact | PatternKind::Prefix if value.contains(['*', '?']) => {
                return Err(ScopeError::InvalidPattern);
            }
            PatternKind::Glob
                if value
                    .split('/')
                    .any(|segment| segment.contains("**") && segment != "**") =>
            {
                return Err(ScopeError::InvalidPattern);
            }
            _ => {}
        }
        Ok(Self { kind, value })
    }

    /// Returns the pattern kind.
    #[must_use]
    pub const fn kind(&self) -> PatternKind {
        self.kind
    }

    /// Returns the normalized pattern text.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Matches one normalized repository-relative path.
    ///
    /// # Errors
    ///
    /// Malformed candidate evidence fails closed.
    pub fn matches_path(&self, path: &str) -> Result<bool, ScopeError> {
        validate_repository_path(path)?;
        Ok(match self.kind {
            PatternKind::Exact => path == self.value,
            PatternKind::Prefix => {
                path == self.value
                    || path
                        .strip_prefix(&self.value)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            }
            PatternKind::Glob => glob_matches(&self.value, path),
        })
    }
}

fn glob_matches(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    let mut memo = vec![vec![None; path.len() + 1]; pattern.len() + 1];
    glob_segments_match(&pattern, &path, 0, 0, &mut memo)
}

fn glob_segments_match(
    pattern: &[&str],
    path: &[&str],
    pattern_index: usize,
    path_index: usize,
    memo: &mut [Vec<Option<bool>>],
) -> bool {
    if let Some(result) = memo[pattern_index][path_index] {
        return result;
    }
    let result = if pattern_index == pattern.len() {
        path_index == path.len()
    } else if pattern[pattern_index] == "**" {
        glob_segments_match(pattern, path, pattern_index + 1, path_index, memo)
            || (path_index < path.len()
                && glob_segments_match(pattern, path, pattern_index, path_index + 1, memo))
    } else {
        path_index < path.len()
            && glob_segment_matches(pattern[pattern_index], path[path_index])
            && glob_segments_match(pattern, path, pattern_index + 1, path_index + 1, memo)
    };
    memo[pattern_index][path_index] = Some(result);
    result
}

fn glob_segment_matches(pattern: &str, value: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let value: Vec<char> = value.chars().collect();
    let mut table = vec![vec![false; value.len() + 1]; pattern.len() + 1];
    table[0][0] = true;
    for pattern_index in 1..=pattern.len() {
        if pattern[pattern_index - 1] == '*' {
            table[pattern_index][0] = table[pattern_index - 1][0];
        }
        for value_index in 1..=value.len() {
            table[pattern_index][value_index] = match pattern[pattern_index - 1] {
                '*' => {
                    table[pattern_index - 1][value_index] || table[pattern_index][value_index - 1]
                }
                '?' => table[pattern_index - 1][value_index - 1],
                literal => {
                    literal == value[value_index - 1] && table[pattern_index - 1][value_index - 1]
                }
            };
        }
    }
    table[pattern.len()][value.len()]
}

/// Immutable version-1.1 path and operation rules.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeRules {
    allowed: Vec<PathPattern>,
    forbidden: Vec<PathPattern>,
    allowed_operations: Vec<ChangeOperation>,
    digest: String,
}

impl ScopeRules {
    /// Constructs one deterministic rule set.
    ///
    /// # Errors
    ///
    /// An empty allow set, empty operation set, unknown operation, or hashing
    /// failure fails closed.
    pub fn new(
        mut allowed: Vec<PathPattern>,
        mut forbidden: Vec<PathPattern>,
        mut allowed_operations: Vec<ChangeOperation>,
    ) -> Result<Self, ScopeError> {
        allowed.sort();
        allowed.dedup();
        forbidden.sort();
        forbidden.dedup();
        allowed_operations.sort();
        allowed_operations.dedup();
        if allowed.is_empty()
            || allowed_operations.is_empty()
            || allowed_operations.contains(&ChangeOperation::Unknown)
        {
            return Err(ScopeError::InvalidRules);
        }
        let digest = rules_digest(&allowed, &forbidden, &allowed_operations)?;
        Ok(Self {
            allowed,
            forbidden,
            allowed_operations,
            digest,
        })
    }

    /// Returns stable sorted allow patterns.
    #[must_use]
    pub fn allowed(&self) -> &[PathPattern] {
        &self.allowed
    }

    /// Returns stable sorted forbidden patterns.
    #[must_use]
    pub fn forbidden(&self) -> &[PathPattern] {
        &self.forbidden
    }

    /// Returns stable sorted allowed operations.
    #[must_use]
    pub fn allowed_operations(&self) -> &[ChangeOperation] {
        &self.allowed_operations
    }

    /// Returns the versioned canonical rule digest.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// Scope reports are evidence only and never authorization.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EvidenceLevel {
    /// The report records what the pure detector observed.
    DetectionOnly,
}

impl EvidenceLevel {
    const fn as_str(self) -> &'static str {
        match self {
            Self::DetectionOnly => "DETECTION_ONLY",
        }
    }
}

/// Stable detector outcome.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ScopeOutcome {
    /// Every observed side matched the configured detection rules.
    Pass,
    /// At least one observed side failed closed.
    Violation,
}

impl ScopeOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Violation => "VIOLATION",
        }
    }
}

/// Which side of a normalized change caused a violation.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ChangeSide {
    /// Base/source side of delete, rename, modify, or type change evidence.
    Source,
    /// Current/destination side of create, rename, modify, or type change evidence.
    Destination,
}

impl ChangeSide {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Source => "SOURCE",
            Self::Destination => "DESTINATION",
        }
    }
}

/// Stable fail-closed classification reason.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ViolationReason {
    /// A forbidden rule matched; it overrides allow and operation rules.
    ForbiddenPath,
    /// No allow rule matched the path.
    PathNotAllowed,
    /// The operation is absent from the exact allowed-operation set.
    OperationNotAllowed,
    /// Link, junction, directory, missing, or unknown path evidence is unsafe.
    UnsafePathKind,
}

impl ViolationReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ForbiddenPath => "FORBIDDEN_PATH",
            Self::PathNotAllowed => "PATH_NOT_ALLOWED",
            Self::OperationNotAllowed => "OPERATION_NOT_ALLOWED",
            Self::UnsafePathKind => "UNSAFE_PATH_KIND",
        }
    }
}

/// One stable path-side violation.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ScopeViolation {
    path: String,
    side: ChangeSide,
    reason: ViolationReason,
    operation: ChangeOperation,
    path_kind: PathKind,
}

impl ScopeViolation {
    /// Returns the violating repository-relative path.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the exact change side.
    #[must_use]
    pub const fn side(&self) -> ChangeSide {
        self.side
    }

    /// Returns the stable reason.
    #[must_use]
    pub const fn reason(&self) -> ViolationReason {
        self.reason
    }

    /// Returns the observed operation.
    #[must_use]
    pub const fn operation(&self) -> ChangeOperation {
        self.operation
    }

    /// Returns the observed physical/path kind.
    #[must_use]
    pub const fn path_kind(&self) -> PathKind {
        self.path_kind
    }
}

/// Stable detection-only scope report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeReport {
    outcome: ScopeOutcome,
    evidence_level: EvidenceLevel,
    violations: Vec<ScopeViolation>,
    rules_digest: String,
    evidence_digest: String,
    report_digest: String,
}

impl ScopeReport {
    /// Returns pass or violation.
    #[must_use]
    pub const fn outcome(&self) -> ScopeOutcome {
        self.outcome
    }

    /// Always returns detection-only.
    #[must_use]
    pub const fn evidence_level(&self) -> EvidenceLevel {
        self.evidence_level
    }

    /// Returns all stable sorted violations.
    #[must_use]
    pub fn violations(&self) -> &[ScopeViolation] {
        &self.violations
    }

    /// Returns the exact rule-set digest.
    #[must_use]
    pub fn rules_digest(&self) -> &str {
        &self.rules_digest
    }

    /// Returns the exact normalized-evidence digest.
    #[must_use]
    pub fn evidence_digest(&self) -> &str {
        &self.evidence_digest
    }

    /// Returns the complete report digest.
    #[must_use]
    pub fn report_digest(&self) -> &str {
        &self.report_digest
    }
}

fn hash(domain_name: &str, value: &CanonicalValue) -> Result<String, ScopeError> {
    let domain = HashDomain::new(domain_name, "1.1").map_err(|_| ScopeError::HashFailed)?;
    Ok(canonical_sha256(&domain, value)
        .map_err(|_| ScopeError::HashFailed)?
        .to_hex())
}

fn pattern_value(pattern: &PathPattern) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "kind".to_owned(),
            CanonicalValue::String(pattern.kind().as_str().to_owned()),
        ),
        (
            "value".to_owned(),
            CanonicalValue::String(pattern.value().to_owned()),
        ),
    ])
}

fn rules_digest(
    allowed: &[PathPattern],
    forbidden: &[PathPattern],
    allowed_operations: &[ChangeOperation],
) -> Result<String, ScopeError> {
    hash(
        "lattice.scope-check.rules",
        &CanonicalValue::Object(vec![
            (
                "allowed".to_owned(),
                CanonicalValue::Array(allowed.iter().map(pattern_value).collect()),
            ),
            (
                "forbidden".to_owned(),
                CanonicalValue::Array(forbidden.iter().map(pattern_value).collect()),
            ),
            (
                "allowed_operations".to_owned(),
                CanonicalValue::Array(
                    allowed_operations
                        .iter()
                        .map(|operation| CanonicalValue::String(operation.as_str().to_owned()))
                        .collect(),
                ),
            ),
        ]),
    )
}

fn violation_value(violation: &ScopeViolation) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "path".to_owned(),
            CanonicalValue::String(violation.path.clone()),
        ),
        (
            "side".to_owned(),
            CanonicalValue::String(violation.side.as_str().to_owned()),
        ),
        (
            "reason".to_owned(),
            CanonicalValue::String(violation.reason.as_str().to_owned()),
        ),
        (
            "operation".to_owned(),
            CanonicalValue::String(violation.operation.as_str().to_owned()),
        ),
        (
            "path_kind".to_owned(),
            CanonicalValue::String(violation.path_kind.as_str().to_owned()),
        ),
    ])
}

fn first_violation_reason(
    rules: &ScopeRules,
    path: &str,
    operation: ChangeOperation,
    kind: PathKind,
) -> Result<Option<ViolationReason>, ScopeError> {
    validate_repository_path(path)?;
    for pattern in &rules.forbidden {
        if pattern.matches_path(path)? {
            return Ok(Some(ViolationReason::ForbiddenPath));
        }
    }
    if kind != PathKind::Regular {
        return Ok(Some(ViolationReason::UnsafePathKind));
    }
    let mut allowed = false;
    for pattern in &rules.allowed {
        allowed |= pattern.matches_path(path)?;
    }
    if !allowed {
        return Ok(Some(ViolationReason::PathNotAllowed));
    }
    if operation == ChangeOperation::Unknown
        || rules.allowed_operations.binary_search(&operation).is_err()
    {
        return Ok(Some(ViolationReason::OperationNotAllowed));
    }
    Ok(None)
}

/// Purely evaluates normalized Workspace Git evidence against exact rules.
///
/// A passing result is detection evidence only. This function performs no
/// filesystem, Git, process, clock, environment, database, or network access.
///
/// # Errors
///
/// Malformed evidence or canonical hashing failure fails closed.
pub fn evaluate(rules: &ScopeRules, manifest: &ChangeManifest) -> Result<ScopeReport, ScopeError> {
    let mut violations = Vec::new();
    for change in manifest.changes() {
        for (side, path, kind) in [
            (
                ChangeSide::Source,
                change.source_path(),
                change.source_kind(),
            ),
            (
                ChangeSide::Destination,
                change.destination_path(),
                change.destination_kind(),
            ),
        ] {
            let (Some(path), Some(kind)) = (path, kind) else {
                continue;
            };
            if let Some(reason) = first_violation_reason(rules, path, change.operation(), kind)? {
                violations.push(ScopeViolation {
                    path: path.to_owned(),
                    side,
                    reason,
                    operation: change.operation(),
                    path_kind: kind,
                });
            }
        }
    }
    violations.sort();
    violations.dedup();
    let outcome = if violations.is_empty() {
        ScopeOutcome::Pass
    } else {
        ScopeOutcome::Violation
    };
    let evidence_level = EvidenceLevel::DetectionOnly;
    let evidence_digest = hash(
        "lattice.scope-check.evidence",
        &CanonicalValue::Object(vec![
            (
                "base_commit".to_owned(),
                CanonicalValue::String(manifest.base_commit().as_str().to_owned()),
            ),
            (
                "head_commit".to_owned(),
                CanonicalValue::String(manifest.head_commit().as_str().to_owned()),
            ),
            (
                "workspace_manifest_digest".to_owned(),
                CanonicalValue::String(manifest.digest().as_str().to_owned()),
            ),
        ]),
    )?;
    let report_digest = hash(
        "lattice.scope-check.report",
        &CanonicalValue::Object(vec![
            (
                "rules_digest".to_owned(),
                CanonicalValue::String(rules.digest.clone()),
            ),
            (
                "evidence_digest".to_owned(),
                CanonicalValue::String(evidence_digest.clone()),
            ),
            (
                "outcome".to_owned(),
                CanonicalValue::String(outcome.as_str().to_owned()),
            ),
            (
                "evidence_level".to_owned(),
                CanonicalValue::String(evidence_level.as_str().to_owned()),
            ),
            (
                "violations".to_owned(),
                CanonicalValue::Array(violations.iter().map(violation_value).collect()),
            ),
        ]),
    )?;
    Ok(ScopeReport {
        outcome,
        evidence_level,
        violations,
        rules_digest: rules.digest.clone(),
        evidence_digest,
        report_digest,
    })
}
