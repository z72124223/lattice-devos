export const defaultControlOrigin = "http://127.0.0.1:4317";

export function requireText(value, label) {
  if (typeof value !== "string" || value.trim() === "") {
    throw new TypeError(`${label} is required`);
  }
  return value.trim();
}

export function normalizeControlOrigin(value) {
  const url = new URL(value || defaultControlOrigin);
  if (url.protocol !== "http:" || url.hostname !== "127.0.0.1") {
    throw new TypeError("Control origin must be an HTTP loopback endpoint");
  }
  if (url.username || url.password) {
    throw new TypeError("Control origin must not contain credentials");
  }
  return url.origin;
}

export async function requestJson(fetchImpl, url, options = {}, timeoutMs = 5_000) {
  const response = await fetchImpl(url, {
    ...options,
    redirect: "error",
    signal: options.signal ?? AbortSignal.timeout(timeoutMs),
  });
  let body;
  try {
    body = await response.json();
  } catch {
    throw new Error(`Control returned invalid JSON (HTTP ${response.status})`);
  }
  if (!response.ok) {
    const error = new Error(body.error || `Control returned HTTP ${response.status}`);
    if (typeof body.code === "string") error.code = body.code;
    error.status = response.status;
    throw error;
  }
  return { response, body };
}

export function resolveProject(projects, { projectId, projectName }) {
  if (!Array.isArray(projects)) throw new Error("Control state did not contain projects");
  if (projectId && projectName) throw new TypeError("use either project ID or project name, not both");
  if (projectId) {
    const project = projects.find((entry) => entry.id === projectId);
    if (!project) throw new Error("Control project ID was not found");
    return project;
  }
  const name = requireText(projectName, "project name");
  const matches = projects.filter((entry) => entry.name === name);
  if (matches.length !== 1) {
    throw new Error(`Control must contain exactly one project named ${JSON.stringify(name)}`);
  }
  return matches[0];
}
