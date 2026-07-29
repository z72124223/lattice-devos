export class DomainError extends Error {
  constructor(code, message, details = undefined) {
    super(message);
    this.name = "DomainError";
    this.code = code;
    this.details = details;
  }
}

export function domainFailure(code, message, details = undefined) {
  throw new DomainError(code, message, details);
}

