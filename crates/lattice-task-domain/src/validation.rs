use std::collections::BTreeSet;

use lattice_cjson::normalize_nfc;
use lattice_contracts::ProjectId;
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

use crate::TaskDomainError;

pub(crate) fn normalize_text(value: &str, field: &'static str) -> Result<String, TaskDomainError> {
    let normalized = normalize_nfc(value.trim());
    if normalized.is_empty() || normalized.contains('\0') {
        return Err(TaskDomainError::InvalidTaskSpec {
            field,
            reason: "must be a non-empty NUL-free string",
        });
    }
    Ok(normalized)
}

pub(crate) fn normalize_text_list(
    values: Vec<String>,
    field: &'static str,
    minimum: usize,
    sort: bool,
) -> Result<Vec<String>, TaskDomainError> {
    if values.len() < minimum {
        return Err(TaskDomainError::InvalidTaskSpec {
            field,
            reason: "does not contain enough values",
        });
    }
    let mut normalized = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in values {
        let value = normalize_text(&value, field)?;
        if !seen.insert(value.clone()) {
            return Err(TaskDomainError::DuplicateTaskFieldValue { field, value });
        }
        normalized.push(value);
    }
    if sort {
        normalized.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    }
    Ok(normalized)
}

pub(crate) fn validate_task_id(value: &str) -> Result<(), TaskDomainError> {
    let suffix = value
        .strip_prefix("TASK-")
        .ok_or_else(|| TaskDomainError::InvalidTaskId {
            value: value.to_owned(),
        })?;
    let valid = (3..=64).contains(&suffix.len())
        && suffix.bytes().all(|byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
        && suffix
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit());
    if valid {
        Ok(())
    } else {
        Err(TaskDomainError::InvalidTaskId {
            value: value.to_owned(),
        })
    }
}

pub(crate) fn normalize_project_id(value: &str) -> Result<String, TaskDomainError> {
    let value = normalize_text(value, "project_id")?.to_ascii_lowercase();
    ProjectId::new(value.clone())
        .map(|project_id| project_id.as_str().to_owned())
        .map_err(|_| TaskDomainError::InvalidProjectId { value })
}

pub(crate) fn normalize_git_object_id(value: &str) -> Result<String, TaskDomainError> {
    let normalized = value.trim().to_ascii_lowercase();
    let valid = matches!(normalized.len(), 40 | 64)
        && normalized
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    if valid {
        Ok(normalized)
    } else {
        Err(TaskDomainError::InvalidGitObjectId {
            value: value.to_owned(),
        })
    }
}

pub(crate) fn normalize_base_ref(value: &str) -> Result<String, TaskDomainError> {
    let value = normalize_text(value, "base_ref")?;
    let unsafe_character = value.chars().any(|character| {
        character <= '\u{1f}'
            || character == '\u{7f}'
            || character.is_whitespace()
            || "~^:?*[\\".contains(character)
    });
    let unsafe_component = value.split('/').any(|component| {
        component.starts_with('.') || component.to_ascii_lowercase().ends_with(".lock")
    });
    if value.starts_with('-')
        || value.starts_with('/')
        || value.ends_with('/')
        || value.ends_with('.')
        || value.contains("..")
        || value.contains("@{")
        || value.contains("//")
        || value == "@"
        || unsafe_character
        || unsafe_component
    {
        return Err(TaskDomainError::InvalidTaskSpec {
            field: "base_ref",
            reason: "unsafe Git ref",
        });
    }
    Ok(value)
}

pub(crate) fn canonical_unsigned(
    value: &str,
    field: &'static str,
    positive: bool,
) -> Result<u64, TaskDomainError> {
    let canonical = !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'));
    let parsed = value.parse::<u64>().ok();
    if !canonical || parsed.is_none() || (positive && parsed == Some(0)) {
        return Err(TaskDomainError::InvalidCanonicalInteger {
            field,
            value: value.to_owned(),
        });
    }
    Ok(parsed.expect("checked Some"))
}

pub(crate) fn canonical_decimal(value: &str, field: &'static str) -> Result<(), TaskDomainError> {
    let (integer, fraction) = match value.split_once('.') {
        Some((integer, fraction)) => (integer, Some(fraction)),
        None => (value, None),
    };
    let integer_valid = !integer.is_empty()
        && integer.bytes().all(|byte| byte.is_ascii_digit())
        && (integer == "0" || !integer.starts_with('0'));
    let fraction_valid = fraction.is_none_or(|fraction| {
        !fraction.is_empty()
            && fraction.bytes().all(|byte| byte.is_ascii_digit())
            && !fraction.ends_with('0')
    });
    if value.len() > crate::MAX_CANONICAL_DECIMAL_BYTES
        || integer.len() > crate::MAX_CANONICAL_DECIMAL_INTEGER_DIGITS
        || fraction.is_some_and(|fraction| fraction.len() > crate::MAX_CANONICAL_DECIMAL_SCALE)
        || !integer_valid
        || !fraction_valid
    {
        return Err(TaskDomainError::InvalidCanonicalDecimal {
            field,
            value: value.to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn canonical_utc_timestamp(value: &str) -> Result<String, TaskDomainError> {
    if !has_strict_utc_rfc3339_shape(value) {
        return Err(TaskDomainError::InvalidUtcTimestamp {
            value: value.to_owned(),
        });
    }
    let parsed = OffsetDateTime::parse(value, &Rfc3339).map_err(|_| {
        TaskDomainError::InvalidUtcTimestamp {
            value: value.to_owned(),
        }
    })?;
    if parsed.offset() != UtcOffset::UTC {
        return Err(TaskDomainError::InvalidUtcTimestamp {
            value: value.to_owned(),
        });
    }
    parsed
        .format(&Rfc3339)
        .map_err(|_| TaskDomainError::InvalidUtcTimestamp {
            value: value.to_owned(),
        })
}

fn has_strict_utc_rfc3339_shape(value: &str) -> bool {
    const DIGIT_POSITIONS: [usize; 14] = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18];
    let bytes = value.as_bytes();
    if !(20..=30).contains(&bytes.len())
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[bytes.len() - 1] != b'Z'
    {
        return false;
    }
    if !DIGIT_POSITIONS
        .iter()
        .all(|index| bytes[*index].is_ascii_digit())
    {
        return false;
    }
    if bytes[17] > b'5' {
        return false;
    }
    if bytes.len() == 20 {
        return true;
    }
    bytes.len() >= 22
        && bytes[19] == b'.'
        && bytes[20..bytes.len() - 1].iter().all(u8::is_ascii_digit)
}

pub(crate) fn normalize_scope_path(
    value: &str,
    field: &'static str,
    allow_git: bool,
) -> Result<String, TaskDomainError> {
    if value.trim() != value {
        return Err(TaskDomainError::InvalidScopePath {
            field,
            path: value.to_owned(),
        });
    }
    let path = normalize_text(value, field)?;
    let bytes = path.as_bytes();
    let drive_path = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    let segments = path.split('/').collect::<Vec<_>>();
    let unsafe_segment = segments.iter().any(|segment| {
        segment.is_empty()
            || matches!(*segment, "." | "..")
            || segment.ends_with(['.', ' '])
            || is_windows_reserved_component(segment)
            || (!allow_git && segment.eq_ignore_ascii_case(".git"))
    });
    let unsafe_character = path
        .chars()
        .any(|character| character <= '\u{1f}' || character == '\u{7f}' || character == ':');
    if path.contains('\\')
        || path.starts_with('/')
        || drive_path
        || unsafe_segment
        || unsafe_character
    {
        return Err(TaskDomainError::InvalidScopePath { field, path });
    }
    Ok(path)
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
    let numbered_device = upper.as_bytes();
    if numbered_device.len() == 4
        && matches!(&numbered_device[..3], b"COM" | b"LPT")
        && matches!(numbered_device[3], b'1'..=b'9')
    {
        return true;
    }
    matches!(
        upper.as_str(),
        "COM¹" | "COM²" | "COM³" | "LPT¹" | "LPT²" | "LPT³"
    )
}
