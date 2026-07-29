export class LedgerError extends Error {
  constructor(code, message, details = undefined) {
    super(message);
    this.name = "LedgerError";
    this.code = code;
    this.details = details;
  }
}

export function ledgerFailure(code, message, details = undefined) {
  throw new LedgerError(code, message, details);
}

