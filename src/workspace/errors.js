export class WorkspaceError extends Error {
  constructor(code, message, details = undefined) {
    super(message);
    this.name = "WorkspaceError";
    this.code = code;
    this.details = details;
  }
}

export function workspaceFailure(code, message, details = undefined) {
  throw new WorkspaceError(code, message, details);
}

