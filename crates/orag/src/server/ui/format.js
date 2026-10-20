// Pure text helpers of the page: no DOM access, so `node --test` covers them
// (`test/format.test.mjs`, `test/i18n.test.mjs`). Text and number formats
// follow the page's language (i18n.js).
import { locale, t } from "./i18n.js";

const isCount = (value) => typeof value === "number" && Number.isFinite(value) && value >= 0;

/** The value in the largest unit below 1024 of `units` and that unit's index. */
function scaled(bytes, units) {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return [value, unit];
}

const units = () => t("format.units").split(",");

/** A byte count with one decimal in the language's style: `3,2 GB` / `3.2 GB`. */
export function byteText(bytes) {
  if (!isCount(bytes)) return "?";
  const names = units();
  const [value, unit] = scaled(bytes, names);
  const rounded = unit === 0 ? Math.round(value) : Math.round(value * 10) / 10;
  return `${String(rounded).replace(".", t("format.decimal"))} ${names[unit]}`;
}

/** A file size (up to MB) with the language's number format. */
export function formatSize(bytes) {
  const names = units().slice(0, 3);
  const [value, unit] = scaled(bytes, names);
  return `${value.toLocaleString(locale(), { maximumFractionDigits: 1 })} ${names[unit]}`;
}

/** `%45` / `45%`; rounded down so a file is at 100 only when it is all there. */
export function percentText(received, total) {
  const value = !isCount(received) || !isCount(total) || total === 0
    ? 0
    : Math.floor((Math.min(received, total) / total) * 100);
  return t("format.percent", { value });
}

/** `s. 12` or `s. 12–13` for a source's PDF pages; empty without pages. */
export function pageLabel(pages) {
  if (!Array.isArray(pages) || pages.length !== 2) return "";
  const [first, last] = pages;
  return first === last ? t("format.page", { page: first }) : t("format.pages", { pages: `${first}–${last}` });
}

/** The label keys of the warning codes (`code: details` on the API). */
const WARNING_KEYS = new Map([
  ["ocr_required", "warning.ocr_required"],
  ["extraction_failed", "warning.extraction_failed"],
  ["suspicious_layout", "warning.suspicious_layout"],
  ["styles_unreadable", "warning.styles_unreadable"],
  ["no_text", "warning.no_text"],
]);

/** The keys of the `suspicious_layout` reasons. */
const LAYOUT_REASONS = new Map([
  ["unmapped characters", "warning.reason.unmapped"],
  ["words split into single letters", "warning.reason.split"],
]);

/**
 * A warning as its label plus the pages and reason it names, so they stay
 * visible without hovering; an unknown code keeps the server's text.
 */
export function warningText(warning) {
  const code = warning.split(":", 1)[0].trim();
  const key = WARNING_KEYS.get(code);
  if (!key) return warning;
  const pages = /page\(s\) ([\d, -]+?)(?: have| could| may|$)/.exec(warning);
  const reason = /garbled \(([^)]+)\)/.exec(warning);
  const details = [];
  if (pages) details.push(t("format.pages", { pages: pages[1].trim().replaceAll("-", "–") }));
  if (code === "suspicious_layout" && reason && LAYOUT_REASONS.has(reason[1])) {
    details.push(t(LAYOUT_REASONS.get(reason[1])));
  }
  return details.length ? `${t(key)} (${details.join("; ")})` : t(key);
}

/** Why an answer stopped early (`finish_reason`). */
const FINISH_KEYS = new Map([
  ["length", "answer.length"],
  ["repetition", "answer.repetition"],
]);

/** The key of the notice for `finish_reason`, or null when it did not stop early. */
export const finishNoticeKey = (reason) => FINISH_KEYS.get(reason) ?? null;

export function finishNotice(reason) {
  const key = finishNoticeKey(reason);
  return key ? t(key) : "";
}

/** The keys of the notices under a finished answer (a `done` event). */
export function answerNoticeKeys(done) {
  if (done.abstained) return ["answer.abstained"];
  return [finishNoticeKey(done.finish_reason), done.uncited ? "answer.uncited" : null].filter(Boolean);
}

/** The notice under a finished answer as one sentence, or "" for none. */
export function answerNotice(done) {
  return answerNoticeKeys(done).map((key) => t(key)).join(" ");
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
