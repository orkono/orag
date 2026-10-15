// Pure text helpers of the page: no DOM access, so `node --test` covers them
// (`test/format.test.mjs`).

/** `s. 12` or `s. 12–13` for a source's PDF pages; empty without pages. */
export function pageLabel(pages) {
  if (!Array.isArray(pages) || pages.length !== 2) return "";
  const [first, last] = pages;
  return first === last ? `s. ${first}` : `s. ${first}–${last}`;
}

/** Turkish labels for the warning codes (`code: details` on the API). */
const WARNING_LABELS = new Map([
  ["ocr_required", "Taranmış sayfalar okunamadı"],
  ["extraction_failed", "Bazı sayfalar okunamadı"],
  ["suspicious_layout", "Bazı sayfaların metni bozuk olabilir"],
  ["styles_unreadable", "Başlık stilleri okunamadı"],
  ["no_text", "Metin bulunamadı"],
]);

/** Turkish names of the `suspicious_layout` reasons. */
const LAYOUT_REASONS = new Map([
  ["unmapped characters", "eşleşmeyen karakterler"],
  ["words split into single letters", "harflere bölünmüş kelimeler"],
]);

/**
 * A warning as its Turkish label plus the pages and reason it names, so they
 * stay visible without hovering; an unknown code keeps the server's text.
 */
export function warningText(warning) {
  const code = warning.split(":", 1)[0].trim();
  const label = WARNING_LABELS.get(code);
  if (!label) return warning;
  const pages = /page\(s\) ([\d, -]+?)(?: have| could| may|$)/.exec(warning);
  const reason = /garbled \(([^)]+)\)/.exec(warning);
  const details = [];
  if (pages) details.push(`s. ${pages[1].trim().replaceAll("-", "–")}`);
  if (code === "suspicious_layout" && reason && LAYOUT_REASONS.has(reason[1])) {
    details.push(LAYOUT_REASONS.get(reason[1]));
  }
  return details.length ? `${label} (${details.join("; ")})` : label;
}

/** Why an answer stopped early (`finish_reason`), or "" when it did not. */
const FINISH_NOTICES = new Map([
  ["length", "Yanıt uzunluk sınırında kesildi."],
  ["repetition", "Yanıt kendini tekrar ettiği için durduruldu; eksik olabilir."],
]);

export function finishNotice(reason) {
  return FINISH_NOTICES.get(reason) || "";
}

const ABSTAINED = "Belgelerde bu sorunun yanıtı bulunamadı.";
const UNCITED = "Bu yanıt hiçbir kaynağa atıf yapmıyor; belgelerden doğrulayın.";

/** The notice under a finished answer (a `done` event), or "" for none. */
export function answerNotice(done) {
  if (done.abstained) return ABSTAINED;
  return [finishNotice(done.finish_reason), done.uncited ? UNCITED : ""].filter(Boolean).join(" ");
}

const DEFAULT_RETRY_MS = 5000;
const MIN_RETRY_MS = 1000;
const MAX_RETRY_MS = 30000;

/** The wait a `429 busy` asks for (`Retry-After`, seconds), within bounds. */
export function retryAfterMs(headers) {
  const seconds = Number.parseInt(headers.get("Retry-After") ?? "", 10);
  if (!Number.isFinite(seconds)) return DEFAULT_RETRY_MS;
  return Math.min(Math.max(seconds * 1000, MIN_RETRY_MS), MAX_RETRY_MS);
}
