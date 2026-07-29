import { ledgerFailure } from "./errors.js";

const SENSITIVE_KEY =
  /(api[_-]?key|token|secret|password|credential|authorization|cookie|private[_-]?key)/i;
const SECRET_PATTERNS = [
  /\bBearer\s+[A-Za-z0-9._~+/=-]{8,}/gi,
  /\bsk-[A-Za-z0-9_-]{8,}/g,
  /-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----/g,
];

function sanitizeString(value) {
  let sanitized = value;
  for (const pattern of SECRET_PATTERNS) {
    sanitized = sanitized.replace(pattern, "[REDACTED]");
  }
  return sanitized;
}

export function sanitizeForAudit(value, seen = new WeakSet()) {
  if (value === null || typeof value === "boolean" || typeof value === "string") {
    return typeof value === "string" ? sanitizeString(value) : value;
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      ledgerFailure("AUDIT_PAYLOAD_NOT_JSON", "Audit payload numbers must be finite.");
    }
    return value;
  }
  if (Array.isArray(value)) {
    if (seen.has(value)) {
      ledgerFailure("AUDIT_PAYLOAD_NOT_JSON", "Audit payload cannot contain cycles.");
    }
    seen.add(value);
    const sanitized = value.map((entry) => sanitizeForAudit(entry, seen));
    seen.delete(value);
    return sanitized;
  }
  if (typeof value === "object") {
    if (
      Object.getPrototypeOf(value) !== Object.prototype ||
      seen.has(value)
    ) {
      ledgerFailure(
        "AUDIT_PAYLOAD_NOT_JSON",
        "Audit payload must contain only acyclic plain JSON objects.",
      );
    }
    seen.add(value);
    const sanitized = {};
    for (const [key, child] of Object.entries(value)) {
      sanitized[key] = SENSITIVE_KEY.test(key)
        ? "[REDACTED]"
        : sanitizeForAudit(child, seen);
    }
    seen.delete(value);
    return sanitized;
  }
  ledgerFailure(
    "AUDIT_PAYLOAD_NOT_JSON",
    `Audit payload does not support ${typeof value}.`,
  );
}

