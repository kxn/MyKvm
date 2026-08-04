// JSON API 封装：非 2xx 统一抛出 ApiError，携带 {error, detail}。

export class ApiError extends Error {
  constructor(status, error, detail, body) {
    super(error || `HTTP ${status}`);
    this.name = "ApiError";
    this.status = status;
    this.detail = detail;
    this.body = body;
  }
}

export async function apiFetch(path, options = {}) {
  const headers = new Headers(options.headers);
  if (options.body !== undefined && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }
  const response = await fetch(path, { ...options, headers });
  const text = await response.text();
  let body = null;
  if (text.length > 0) {
    try {
      body = JSON.parse(text);
    } catch {
      body = null;
    }
  }
  if (!response.ok) {
    throw new ApiError(
      response.status,
      body?.error ?? undefined,
      body?.detail ?? undefined,
      body,
    );
  }
  return body;
}

export function getJson(path) {
  return apiFetch(path);
}

export function postJson(path, payload) {
  return apiFetch(path, {
    method: "POST",
    body: JSON.stringify(payload ?? {}),
  });
}

export function errorText(error) {
  if (error instanceof ApiError) {
    return error.detail ? `${error.message}：${error.detail}` : error.message;
  }
  if (error && typeof error.message === "string") {
    return error.message;
  }
  return String(error);
}
