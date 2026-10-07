// Calls to the ORAG API and readable errors for them.
import { el } from "./dom.js";

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

/** What each error code means for the user; the API message stays as detail. */
const MESSAGES = {
  unreachable: "Sunucuya ulaşılamıyor. orag serve çalışıyor mu?",
  reindex_required:
    "Bu koleksiyon başka bir modelle indekslenmiş. \"Koleksiyonu yönet\" altındaki " +
    "\"Yeniden indeksle\" ile belgeleri mevcut modelle yeniden işleyin.",
  unsupported_format: "Bu dosya türü desteklenmiyor. TXT, Markdown, DOCX veya PDF yükleyin.",
  too_large: "Dosya boyut sınırını aşıyor.",
  upload_timeout: "Dosya zamanında gönderilemedi; tekrar deneyin.",
  busy: "Sunucu şu an meşgul. Birkaç saniye sonra tekrar deneyin.",
  shutting_down: "Sunucu kapanıyor.",
  not_found: "Bulunamadı; başka bir yerden silinmiş olabilir.",
  internal: "Sunucuda bir hata oluştu; ayrıntılar sunucu günlüğünde.",
};

/**
 * An error as a short Turkish sentence with the technical detail below.
 * `byCode` gives sentences that depend on what was being done (a `conflict`
 * means different things for different actions); `fallback` covers the rest.
 */
export function errorBox(err, fallback, byCode = {}) {
  const box = el("div", undefined, "error-box");
  box.setAttribute("role", "alert");
  box.append(el("strong", byCode[err.code] || MESSAGES[err.code] || fallback));
  if (err.message) box.append(el("div", err.message, "detail"));
  return box;
}

let limits = null;

/** The server's document size limit (`max_document_mb`), read once. */
export async function serverLimits() {
  if (!limits) {
    const version = await getJson("/v1/version");
    const maxMb = version.config.max_document_mb;
    limits = { maxMb, maxBytes: maxMb * 1024 * 1024 };
  }
  return limits;
}
