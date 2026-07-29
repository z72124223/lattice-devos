import { deepFreeze, sha256Canonical } from "../domain/canonical-json.js";

function decision(allowed, reasonCode, evidence) {
  return deepFreeze({
    allowed,
    reason_code: reasonCode,
    evidence,
  });
}

function deny(reasonCode, evidence = {}) {
  return decision(false, reasonCode, evidence);
}

function nonEmpty(value) {
  return typeof value === "string" && value.trim().length > 0;
}

function nonceWasUsed(usedNonces, nonce) {
  if (usedNonces instanceof Set) {
    return usedNonces.has(nonce);
  }
  return Array.isArray(usedNonces) && usedNonces.includes(nonce);
}

export function createMergeApprovalSubject({
  task_id,
  task_revision,
  reviewed_commit,
  diff_hash,
}) {
  return sha256Canonical({
    task_id,
    task_revision,
    reviewed_commit,
    diff_hash,
  });
}

export async function verifyApproval({
  kind,
  expectedState,
  state,
  spec,
  approval,
  subjectHash,
  usedNonces = new Set(),
  approvalVerifier,
  now,
}) {
  if (state !== expectedState) {
    return deny("APPROVAL_STATE_DENIED", { state, expected_state: expectedState });
  }
  if (
    approval === null ||
    typeof approval !== "object" ||
    Array.isArray(approval)
  ) {
    return deny("APPROVAL_MISSING");
  }
  if (approval.kind !== kind) {
    return deny("APPROVAL_KIND_MISMATCH", {
      expected_kind: kind,
      actual_kind: approval.kind,
    });
  }
  if (approval.task_id !== spec?.task_id) {
    return deny("APPROVAL_TASK_MISMATCH", {
      expected_task_id: spec?.task_id,
      actual_task_id: approval.task_id,
    });
  }
  if (approval.task_revision !== spec?.revision) {
    return deny("APPROVAL_REVISION_MISMATCH", {
      expected_revision: spec?.revision,
      actual_revision: approval.task_revision,
    });
  }
  if (approval.subject_hash !== subjectHash) {
    return deny("APPROVAL_SUBJECT_MISMATCH", {
      expected_subject_hash: subjectHash,
      actual_subject_hash: approval.subject_hash,
    });
  }
  if (approval.authority !== "HUMAN_OWNER") {
    return deny("APPROVAL_AUTHORITY_DENIED", {
      authority: approval.authority,
    });
  }
  if (
    !nonEmpty(approval.approval_id) ||
    !nonEmpty(approval.approver_id) ||
    !nonEmpty(approval.nonce) ||
    !nonEmpty(approval.channel)
  ) {
    return deny("APPROVAL_INVALID");
  }
  const issuedAt = Date.parse(approval.issued_at);
  const expiresAt = Date.parse(approval.expires_at);
  const nowTime = now instanceof Date ? now.getTime() : new Date(now).getTime();
  if (
    !Number.isFinite(issuedAt) ||
    !Number.isFinite(expiresAt) ||
    expiresAt <= issuedAt
  ) {
    return deny("APPROVAL_INVALID_TIME");
  }
  if (issuedAt > nowTime) {
    return deny("APPROVAL_NOT_YET_VALID");
  }
  if (expiresAt <= nowTime) {
    return deny("APPROVAL_EXPIRED");
  }
  if (nonceWasUsed(usedNonces, approval.nonce)) {
    return deny("APPROVAL_REPLAYED", { nonce: approval.nonce });
  }

  let verified;
  try {
    verified = await approvalVerifier(approval);
  } catch {
    return deny("APPROVAL_IDENTITY_UNVERIFIED");
  }
  if (verified?.verified !== true) {
    return deny("APPROVAL_IDENTITY_UNVERIFIED");
  }
  if (
    nonEmpty(verified.owner_id) &&
    verified.owner_id !== approval.approver_id
  ) {
    return deny("APPROVER_ID_MISMATCH");
  }
  return decision(true, `${kind.toUpperCase()}_APPROVAL_VALID`, {
    approval_id: approval.approval_id,
    kind,
    approver_id: approval.approver_id,
    subject_hash: subjectHash,
  });
}

