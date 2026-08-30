//! Exact local process-death observation for managed Writer recovery.
//!
//! The existing Writer identity predates an OS creation-time certificate.  A
//! replacement process therefore cannot safely classify a present PID as the
//! predecessor or as PID reuse.  It may, however, prove that the predecessor
//! is no longer alive when two complete Windows process snapshots both omit
//! the exact retained PID.  Every other result fails closed.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::thread;
use std::time::Duration;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::ContentDigest;

const PROCESS_ABSENCE_SAMPLE_DOMAIN: &str = "lattice.managed-process-snapshot";
const PROCESS_ABSENCE_SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

/// Closed failure reasons for the local process observer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ManagedProcessObservationError {
    #[cfg(not(windows))]
    UnsupportedPlatform,
    InvalidProcessId,
    CurrentProcess,
    SnapshotUnavailable,
    SnapshotIncomplete,
    ProcessStillPresent,
    InconsistentObservation,
    DigestFailure,
}

impl ManagedProcessObservationError {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            #[cfg(not(windows))]
            Self::UnsupportedPlatform => "LATTICE_MANAGED_WRITER_PROCESS_OBSERVER_UNSUPPORTED",
            Self::InvalidProcessId => "LATTICE_MANAGED_WRITER_PROCESS_ID_REJECTED",
            Self::CurrentProcess => "LATTICE_MANAGED_WRITER_PROCESS_STILL_ACTIVE",
            Self::SnapshotUnavailable => "LATTICE_MANAGED_WRITER_PROCESS_SNAPSHOT_UNAVAILABLE",
            Self::SnapshotIncomplete => "LATTICE_MANAGED_WRITER_PROCESS_SNAPSHOT_INCOMPLETE",
            Self::ProcessStillPresent => "LATTICE_MANAGED_WRITER_PROCESS_STILL_ACTIVE",
            Self::InconsistentObservation => {
                "LATTICE_MANAGED_WRITER_PROCESS_OBSERVATION_INCONSISTENT"
            }
            Self::DigestFailure => "LATTICE_MANAGED_WRITER_PROCESS_EVIDENCE_REJECTED",
        }
    }
}

impl fmt::Display for ManagedProcessObservationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for ManagedProcessObservationError {}

/// Secret-free proof that two complete snapshots omitted one exact PID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedProcessAbsence {
    holder_process_id: u64,
    first_snapshot_digest: ContentDigest,
    second_snapshot_digest: ContentDigest,
}

impl VerifiedProcessAbsence {
    pub(crate) const fn holder_process_id(&self) -> u64 {
        self.holder_process_id
    }

    pub(crate) const fn first_snapshot_digest(&self) -> &ContentDigest {
        &self.first_snapshot_digest
    }

    pub(crate) const fn second_snapshot_digest(&self) -> &ContentDigest {
        &self.second_snapshot_digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompleteProcessSnapshot {
    process_ids: BTreeSet<u32>,
    digest: ContentDigest,
}

/// Proves that an exact retained PID is absent from two complete local OS
/// snapshots. A present/reused PID is deliberately not classified until a
/// future Writer identity version persists an OS creation-time certificate.
pub(crate) fn verify_process_absent(
    holder_process_id: u64,
) -> Result<VerifiedProcessAbsence, ManagedProcessObservationError> {
    let holder_process_id = u32::try_from(holder_process_id)
        .ok()
        .filter(|value| *value != 0)
        .ok_or(ManagedProcessObservationError::InvalidProcessId)?;
    if holder_process_id == std::process::id() {
        return Err(ManagedProcessObservationError::CurrentProcess);
    }
    let first = complete_process_snapshot()?;
    thread::sleep(PROCESS_ABSENCE_SAMPLE_INTERVAL);
    let second = complete_process_snapshot()?;
    verify_two_snapshot_absence(holder_process_id, first, second)
}

fn verify_two_snapshot_absence(
    holder_process_id: u32,
    first: CompleteProcessSnapshot,
    second: CompleteProcessSnapshot,
) -> Result<VerifiedProcessAbsence, ManagedProcessObservationError> {
    let first_present = first.process_ids.contains(&holder_process_id);
    let second_present = second.process_ids.contains(&holder_process_id);
    match (first_present, second_present) {
        (false, false) => Ok(VerifiedProcessAbsence {
            holder_process_id: u64::from(holder_process_id),
            first_snapshot_digest: first.digest,
            second_snapshot_digest: second.digest,
        }),
        (true, true) => Err(ManagedProcessObservationError::ProcessStillPresent),
        _ => Err(ManagedProcessObservationError::InconsistentObservation),
    }
}

fn snapshot_digest(
    process_ids: &BTreeSet<u32>,
) -> Result<ContentDigest, ManagedProcessObservationError> {
    let domain = HashDomain::new(PROCESS_ABSENCE_SAMPLE_DOMAIN, "1.0")
        .map_err(|_| ManagedProcessObservationError::DigestFailure)?;
    let value = CanonicalValue::Object(vec![
        (
            "classification".to_owned(),
            CanonicalValue::String("COMPLETE_PROCESS_ID_SET".to_owned()),
        ),
        (
            "process_ids".to_owned(),
            CanonicalValue::Array(
                process_ids
                    .iter()
                    .map(|value| CanonicalValue::String(value.to_string()))
                    .collect(),
            ),
        ),
    ]);
    canonical_sha256(&domain, &value)
        .map_err(|_| ManagedProcessObservationError::DigestFailure)
        .and_then(|digest| {
            ContentDigest::from_sha256(digest.to_hex())
                .map_err(|_| ManagedProcessObservationError::DigestFailure)
        })
}

#[cfg(windows)]
fn complete_process_snapshot() -> Result<CompleteProcessSnapshot, ManagedProcessObservationError> {
    windows_process_snapshot()
}

#[cfg(not(windows))]
fn complete_process_snapshot() -> Result<CompleteProcessSnapshot, ManagedProcessObservationError> {
    Err(ManagedProcessObservationError::UnsupportedPlatform)
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn windows_process_snapshot() -> Result<CompleteProcessSnapshot, ManagedProcessObservationError> {
    use std::mem::size_of;

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_NO_MORE_FILES, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };

    struct SnapshotHandle(HANDLE);

    impl Drop for SnapshotHandle {
        fn drop(&mut self) {
            if self.0 != INVALID_HANDLE_VALUE {
                // SAFETY: this type exclusively owns the valid snapshot
                // handle returned by CreateToolhelp32Snapshot.
                let _closed = unsafe { CloseHandle(self.0) };
            }
        }
    }

    // SAFETY: flags and process id follow the documented process-snapshot
    // contract; the returned handle is immediately owned by SnapshotHandle.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if raw == INVALID_HANDLE_VALUE {
        return Err(ManagedProcessObservationError::SnapshotUnavailable);
    }
    let handle = SnapshotHandle(raw);
    let mut entry = PROCESSENTRY32W {
        dwSize: u32::try_from(size_of::<PROCESSENTRY32W>())
            .map_err(|_| ManagedProcessObservationError::SnapshotUnavailable)?,
        ..PROCESSENTRY32W::default()
    };
    // SAFETY: entry points to a correctly-sized writable PROCESSENTRY32W for
    // the lifetime of the snapshot enumeration.
    if unsafe { Process32FirstW(handle.0, &raw mut entry) } == 0 {
        // SAFETY: GetLastError has no preconditions and is read immediately
        // after the failed enumeration call.
        return if unsafe { GetLastError() } == ERROR_NO_MORE_FILES {
            Err(ManagedProcessObservationError::SnapshotIncomplete)
        } else {
            Err(ManagedProcessObservationError::SnapshotUnavailable)
        };
    }
    let mut process_ids = BTreeSet::new();
    process_ids.insert(entry.th32ProcessID);
    loop {
        // SAFETY: entry remains correctly-sized and writable; handle remains
        // valid and exclusively owned until this function returns.
        if unsafe { Process32NextW(handle.0, &raw mut entry) } != 0 {
            process_ids.insert(entry.th32ProcessID);
            continue;
        }
        // SAFETY: GetLastError is sampled immediately after Process32NextW.
        if unsafe { GetLastError() } != ERROR_NO_MORE_FILES {
            return Err(ManagedProcessObservationError::SnapshotIncomplete);
        }
        break;
    }
    let digest = snapshot_digest(&process_ids)?;
    Ok(CompleteProcessSnapshot {
        process_ids,
        digest,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CompleteProcessSnapshot, ManagedProcessObservationError, snapshot_digest,
        verify_two_snapshot_absence,
    };
    use std::collections::BTreeSet;

    fn snapshot(values: &[u32]) -> CompleteProcessSnapshot {
        let process_ids = values.iter().copied().collect::<BTreeSet<_>>();
        let digest = snapshot_digest(&process_ids).expect("snapshot digest");
        CompleteProcessSnapshot {
            process_ids,
            digest,
        }
    }

    #[test]
    fn two_complete_absent_snapshots_are_required() {
        let first = snapshot(&[1, 9, 44]);
        let second = snapshot(&[1, 10, 45]);
        let expected_first = first.digest.clone();
        let expected_second = second.digest.clone();
        let proof = verify_two_snapshot_absence(7, first, second).expect("exact absence");
        assert_eq!(proof.holder_process_id(), 7);
        assert_eq!(proof.first_snapshot_digest(), &expected_first);
        assert_eq!(proof.second_snapshot_digest(), &expected_second);
    }

    #[test]
    fn present_pid_fails_closed() {
        assert_eq!(
            verify_two_snapshot_absence(7, snapshot(&[1, 7]), snapshot(&[2, 7])),
            Err(ManagedProcessObservationError::ProcessStillPresent)
        );
    }

    #[test]
    fn changing_pid_observation_is_inconsistent() {
        assert_eq!(
            verify_two_snapshot_absence(7, snapshot(&[1]), snapshot(&[2, 7])),
            Err(ManagedProcessObservationError::InconsistentObservation)
        );
        assert_eq!(
            verify_two_snapshot_absence(7, snapshot(&[1, 7]), snapshot(&[2])),
            Err(ManagedProcessObservationError::InconsistentObservation)
        );
    }

    #[test]
    fn snapshot_digest_binds_the_complete_sorted_pid_set() {
        let ordered = snapshot(&[1, 7, 9]);
        let reordered = snapshot(&[9, 1, 7]);
        let substituted = snapshot(&[1, 8, 9]);
        assert_eq!(ordered.digest, reordered.digest);
        assert_ne!(ordered.digest, substituted.digest);
    }
}
