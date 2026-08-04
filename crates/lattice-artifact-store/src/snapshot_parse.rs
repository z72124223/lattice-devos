//! Closed-schema helpers shared by aggregate metadata restore paths.

use lattice_cjson::CanonicalValue;
use lattice_contracts::{
    ArtifactGeneration, ArtifactObjectIdentity, ArtifactObjectKey, ContentDigest, ProjectId, TaskId,
};

use crate::{ArtifactLimitKind, ArtifactStoreLimits};

/// Internal parse failure. Public replay deliberately maps every malformed or
/// unsupported raw shape to one non-secret fail-closed error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SnapshotParseError;

pub(crate) type SnapshotParseResult<T> = Result<T, SnapshotParseError>;

/// Exact ordered canonical object view.
///
/// Requiring the serializer's frozen field order additionally rejects raw
/// reorder, duplicate, missing, and unknown fields before any owner is built.
pub(crate) struct StrictSnapshotObject<'a> {
    entries: &'a [(String, CanonicalValue)],
}

impl<'a> StrictSnapshotObject<'a> {
    pub(crate) fn new(value: &'a CanonicalValue, expected: &[&str]) -> SnapshotParseResult<Self> {
        let CanonicalValue::Object(entries) = value else {
            return Err(SnapshotParseError);
        };
        if entries.len() != expected.len()
            || entries
                .iter()
                .zip(expected)
                .any(|((actual, _), expected)| actual != expected)
        {
            return Err(SnapshotParseError);
        }
        Ok(Self { entries })
    }

    pub(crate) fn get(&self, name: &str) -> SnapshotParseResult<&'a CanonicalValue> {
        self.entries
            .iter()
            .find_map(|(field, value)| (field == name).then_some(value))
            .ok_or(SnapshotParseError)
    }

    pub(crate) fn string(&self, name: &str) -> SnapshotParseResult<&'a str> {
        match self.get(name)? {
            CanonicalValue::String(value) => Ok(value),
            _ => Err(SnapshotParseError),
        }
    }

    pub(crate) fn array(&self, name: &str) -> SnapshotParseResult<&'a [CanonicalValue]> {
        match self.get(name)? {
            CanonicalValue::Array(values) => Ok(values),
            _ => Err(SnapshotParseError),
        }
    }
}

pub(crate) fn parse_u64(value: &str) -> SnapshotParseResult<u64> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(SnapshotParseError);
    }
    value.parse().map_err(|_| SnapshotParseError)
}

pub(crate) fn parse_i64(value: &str) -> SnapshotParseResult<i64> {
    if value.is_empty()
        || value.starts_with('-')
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(SnapshotParseError);
    }
    value.parse().map_err(|_| SnapshotParseError)
}

pub(crate) fn parse_digest(value: &str) -> SnapshotParseResult<ContentDigest> {
    ContentDigest::from_sha256(value.to_owned()).map_err(|_| SnapshotParseError)
}

pub(crate) fn parse_optional_digest(
    value: &CanonicalValue,
) -> SnapshotParseResult<Option<ContentDigest>> {
    match value {
        CanonicalValue::Null => Ok(None),
        CanonicalValue::String(value) => parse_digest(value).map(Some),
        _ => Err(SnapshotParseError),
    }
}

pub(crate) fn parse_project_id(value: &str) -> SnapshotParseResult<ProjectId> {
    ProjectId::new(value.to_owned()).map_err(|_| SnapshotParseError)
}

pub(crate) fn parse_task_id(value: &str) -> SnapshotParseResult<TaskId> {
    TaskId::new(value.to_owned()).map_err(|_| SnapshotParseError)
}

pub(crate) fn parse_object_identity(
    value: &CanonicalValue,
) -> SnapshotParseResult<ArtifactObjectIdentity> {
    let object = StrictSnapshotObject::new(
        value,
        &["project_id", "algorithm", "content_digest", "generation"],
    )?;
    if object.string("algorithm")? != "sha256" {
        return Err(SnapshotParseError);
    }
    let project_id = parse_project_id(object.string("project_id")?)?;
    let content_digest = parse_digest(object.string("content_digest")?)?;
    let generation = ArtifactGeneration::new(parse_u64(object.string("generation")?)?)
        .map_err(|_| SnapshotParseError)?;
    Ok(ArtifactObjectIdentity::new(
        ArtifactObjectKey::new(project_id, content_digest),
        generation,
    ))
}

pub(crate) fn parse_limits(value: &CanonicalValue) -> SnapshotParseResult<ArtifactStoreLimits> {
    let CanonicalValue::Object(fields) = value else {
        return Err(SnapshotParseError);
    };
    if fields.len() != ArtifactLimitKind::ALL.len() {
        return Err(SnapshotParseError);
    }
    let mut limits = ArtifactStoreLimits::hard_maximums();
    for ((name, value), kind) in fields.iter().zip(ArtifactLimitKind::ALL) {
        let CanonicalValue::String(value) = value else {
            return Err(SnapshotParseError);
        };
        if name != kind.as_str() {
            return Err(SnapshotParseError);
        }
        limits = limits
            .tighten(kind, parse_u64(value)?)
            .map_err(|_| SnapshotParseError)?;
    }
    Ok(limits)
}
