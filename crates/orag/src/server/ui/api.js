// Calls to the ORAG API and readable errors for them.
import { el, messageEl, tEl } from "./dom.js";
import { t } from "./i18n.js";

/**
 * An Error for a failed response, with the API's `{error:{code,message}}`
 * kept as technical detail, and `status` and `code` to act on.
 */
export async function apiError(response) {
  let detail = response.statusText;
  let code = null;
  try {
    const body = await response.json();
    if (body && body.error) {
      code = body.error.code;
      detail = `${body.error.message} (${code})`;
    }
  } catch {
    // Not JSON: keep the status text.
  }
  const err = new Error(`HTTP ${response.status}: ${detail}`);
  err.status = response.status;
  err.code = code;
  err.headers = response.headers;
  return err;
}

/** `fetch` that throws for a failed response and names an unreachable server. */
export async function request(path, options = {}) {
  let response;
  try {
    response = await fetch(path, options);
  } catch (err) {
    // Aborted on purpose (with or without a reason): not a network failure.
    if (err.name === "AbortError" || options.signal?.aborted) throw err;
    const unreachable = new Error(err.message);
    unreachable.code = "unreachable";
    throw unreachable;
  }
  if (!response.ok) throw await apiError(response);
  return response;
}

export async function getJson(path) {
  return (await request(path, { headers: { Accept: "application/json" } })).json();
}

/** The error codes with a sentence of their own (`error.<code>` in i18n.js). */
const ERROR_CODES = new Set([
  "unreachable",
  "reindex_required",
  "unsupported_format",
  "too_large",
  "upload_timeout",
  "busy",
  "shutting_down",
  "not_found",
  "internal",
  "models_missing",
]);

/**
 * The key of an error's short sentence; the API message stays as detail.
 * `byCode` gives keys that depend on what was being done (a `conflict`
 * means different things for different actions); `fallback` (a key) covers
 * the rest.
 */
export function errorSentenceKey(err, fallback, byCode = {}) {
  if (Object.hasOwn(byCode, err.code)) return byCode[err.code];
  return ERROR_CODES.has(err.code) ? `error.${err.code}` : fallback;
}

export function errorSentence(err, fallback, byCode = {}) {
  return t(errorSentenceKey(err, fallback, byCode));
}

/**
 * An Error the page raises itself: its sentence travels as `messageKey`
 * (and `messageParams`), so wherever it is shown it is translated and
 * follows a language change; `message` is that sentence now, for logs.
 */
export function clientError(key, params, code) {
  const err = new Error(t(key, params));
  err.messageKey = key;
  err.messageParams = params;
  if (code) err.code = code;
  return err;
}

/**
 * An error's detail line: the page's own sentence as a message, a server's
 * or the browser's words as they are; "" for none.
 */
export function errorDetail(err) {
  if (err.messageKey) return { key: err.messageKey, params: err.messageParams };
  return err.message || "";
}

/** The sentence (kept translated) with the technical detail below. */
export function errorBox(err, fallback, byCode = {}) {
  const box = el("div", undefined, "error-box");
  box.setAttribute("role", "alert");
  const key = errorSentenceKey(err, fallback, byCode);
  box.append(tEl("strong", key));
  const detail = errorDetail(err);
  // The page's own sentence as the heading already says it all.
  const repeated = err.messageKey === key && !err.messageParams;
  if (detail && !repeated) box.append(messageEl("div", detail, "detail"));
  return box;
}

let version = null;

/**
 * `/v1/version` (version, models, limits), read once and again on `refresh`
 * (after a reconnect the server may run with another config). Every refresh
 * announces the new value with `orag:version`.
 */
export async function versionInfo(refresh = false) {
  if (!version || refresh) {
    version = await getJson("/v1/version");
    document.dispatchEvent(new CustomEvent("orag:version", { detail: version }));
  }
  return version;
}

/** The server's document size limit (`max_document_mb`). */
export async function serverLimits() {
  const maxMb = (await versionInfo()).config.max_document_mb;
  return { maxMb, maxBytes: maxMb * 1024 * 1024 };
}
