// Thin fetch wrapper: JSON in, JSON out, explicit errors, CSRF on mutations.

let csrfToken = null;

export function setCsrf(token) {
  csrfToken = token || null;
}

export function getCsrf() {
  return csrfToken;
}

export class ApiError extends Error {
  constructor(message, status, code) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
  }
}

async function request(method, path, options = {}) {
  const headers = new Headers(options.headers || {});
  const init = { method, headers, credentials: "same-origin" };

  if (options.body instanceof Blob || options.body instanceof ArrayBuffer) {
    init.body = options.body;
  } else if (options.body !== undefined) {
    headers.set("content-type", "application/json");
    init.body = JSON.stringify(options.body);
  }
  if (method !== "GET" && method !== "HEAD" && csrfToken) {
    headers.set("x-csrf-token", csrfToken);
  }

  let response;
  try {
    response = await fetch(path, init);
  } catch (err) {
    throw new ApiError("Cannot reach WinPanel. Is the panel still running?", 0, "network");
  }

  if (response.status === 401) {
    window.dispatchEvent(new CustomEvent("winpanel:unauthorized"));
  }

  const contentType = response.headers.get("content-type") || "";
  const isJson = contentType.includes("application/json");
  const payload = isJson ? await response.json().catch(() => null) : null;

  if (!response.ok) {
    const message =
      (payload && (payload.message || payload.error)) ||
      `${response.status} ${response.statusText}`;
    throw new ApiError(message, response.status, payload?.error || "error");
  }
  return payload;
}

export const api = {
  get: (path) => request("GET", path),
  post: (path, body) => request("POST", path, { body: body ?? {} }),
  put: (path, body) => request("PUT", path, { body: body ?? {} }),
  patch: (path, body) => request("PATCH", path, { body: body ?? {} }),
  del: (path, body) => request("DELETE", path, { body }),
  upload: (path, file) => request("PUT", path, { body: file }),
};

export function downloadUrl(path) {
  const anchor = document.createElement("a");
  anchor.href = path;
  anchor.rel = "noopener";
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
}

export function errorMessage(err) {
  if (err instanceof ApiError) return err.message;
  if (err instanceof Error) return err.message;
  return String(err);
}
