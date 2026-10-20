// The in-app model download (step 76): the `/v1/models/download` calls and
// the pure logic that turns a status body into what the setup panel shows.
// No DOM access, so `node --test` covers it (`test/download.test.mjs`).
import { getJson, request } from "./api.js";
import { byteText, percentText } from "./format.js";
import { t } from "./i18n.js";

export { byteText, percentText };

const ENDPOINT = "/v1/models/download";
const MODELS_ENDPOINT = "/v1/models";
const POLL_MS = 1000;
const RESTART_POLL_MS = 1000;
const RESTART_LIMIT_MS = 60000;

/** The keys of the sentences for refused requests, by error code. */
export const DOWNLOAD_ERRORS = {
  conflict: "download.conflict",
  download_disabled: "download.disabled",
  download_unavailable: "download.notDownloadable",
};

const STATES = new Set(["waiting", "downloading", "verifying", "installed", "failed"]);
/** The failed-download codes that have their own sentence. */
const ERROR_CODES = new Set(["network", "http", "checksum", "disk", "cancelled", "internal"]);
const PHASES = new Set(["idle", "running", "done", "failed", "cancelled"]);

const isCount = (value) => typeof value === "number" && Number.isFinite(value) && value >= 0;

export function downloadButtonLabel(bytes) {
  return isCount(bytes) ? t("download.buttonSize", { size: byteText(bytes) }) : t("download.button");
}

/** The key of why there is no download; the server's English reason is detail. */
export function unavailableKey(download) {
  const code = download?.code;
  if (code === "conflict") return "download.installed";
  return Object.hasOwn(DOWNLOAD_ERRORS, code) ? DOWNLOAD_ERRORS[code] : "download.unavailable";
}

export const unavailableText = (download) => t(unavailableKey(download));

/** The key of what a failed download's `error.code` means for the user. */
export function downloadErrorKey(code) {
  return ERROR_CODES.has(code) ? `download.error.${code}` : "download.error.fallback";
}

export const downloadErrorText = (code) => t(downloadErrorKey(code));

function amount(received, total) {
  return { amount: `${byteText(received)} / ${byteText(total)}`, percent: percentText(received, total) };
}

function modelRow(model) {
  const received = isCount(model.received) ? model.received : 0;
  const total = isCount(model.total) ? model.total : 0;
  return {
    id: String(model.id),
    state: model.state,
    stateText: STATES.has(model.state) ? t(`download.state.${model.state}`) : "",
    received,
    total,
    ...amount(received, total),
  };
}

function failure(error) {
  const code = error?.code ?? null;
  return { code, key: downloadErrorKey(code), text: downloadErrorText(code), detail: error?.message || "" };
}

/**
 * A status body as the panel's view, in the current language: phase, rows,
 * overall line, error, and the status itself (to show it again after a
 * language change).
 */
export function downloadView(status) {
  const phase = PHASES.has(status?.state) ? status.state : "idle";
  const models = Array.isArray(status?.models) ? status.models.map(modelRow) : [];
  const received = models.reduce((sum, m) => sum + m.received, 0);
  const total = models.reduce((sum, m) => sum + m.total, 0);
  return {
    phase,
    busy: phase === "running",
    models,
    overall: { received, total, ...amount(received, total) },
    error: phase === "failed" ? failure(status.error) : null,
    restart: status?.restart ?? null,
    status,
  };
}

async function call(method) {
  const options = { method, headers: { Accept: "application/json" } };
  return (await request(ENDPOINT, options)).json();
}

/** Starts (or resumes) the download; 202 returns the first status. */
export const startDownload = () => call("POST");
export const downloadStatus = () => call("GET");
/** Cancels; the `.part` files stay so the next start resumes. */
export const cancelDownload = () => call("DELETE");

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * Shows `status`, then polls once a second while it is running and shows
 * each one; returns the final status. A failed poll throws, and an `idle`
 * after `running` is returned unshown: the server restarted, or lost the
 * run. `settleDownload` tells which.
 */
export async function followDownload(status, onView, { wait = sleep, intervalMs = POLL_MS } = {}) {
  let current = status;
  onView(downloadView(current));
  while (current?.state === "running") {
    await wait(intervalMs);
    current = await downloadStatus();
    if (current?.state === "idle") return current;
    onView(downloadView(current));
  }
  return current;
}

/**
 * What to do when the status of a followed download was lost: its poll
 * failed (`statusError`) or answered `idle`. `models` (or `modelsError`) is
 * the answer of `/v1/models` asked right after; `status` (or `statusError`)
 * the download status read when that answer was `setup`.
 * - `reload`: the models are in (a new process answered `ready`);
 * - `wait_restart`: no answer, so a restart is in progress;
 * - `resume`: the download is still known (running or finished), follow it;
 * - `show_start` / `show_error`: still in setup, nothing running.
 */
export function afterLostStatus({ status, statusError, models, modelsError } = {}) {
  if (modelsError || !models) return "wait_restart";
  if (models.mode === "ready") return "reload";
  if (statusError || !status) return "show_error";
  return status.state === "idle" || !PHASES.has(status.state) ? "show_start" : "resume";
}

async function attempt(read) {
  try {
    return { value: await read() };
  } catch (error) {
    return { error };
  }
}

/** Asks `/v1/models` and, while still in setup, the download status again. */
async function reconcile(lost) {
  const models = await attempt(() => getJson(MODELS_ENDPOINT));
  const facts = { ...lost, models: models.value, modelsError: models.error };
  if (models.error || models.value?.mode === "ready" || !lost.statusError) return facts;
  const again = await attempt(downloadStatus);
  return { ...facts, status: again.value, statusError: again.error };
}

/**
 * Polls `/v1/models` until an answer passes `accept` (returned) or
 * `limitMs` passes (null). Connection errors mean the restart is still
 * under way.
 */
async function waitForServer(accept, { wait = sleep, now = Date.now, pollMs = RESTART_POLL_MS, limitMs = RESTART_LIMIT_MS } = {}) {
  const deadline = now() + limitMs;
  await wait(pollMs);
  while (now() < deadline) {
    const models = await attempt(() => getJson(MODELS_ENDPOINT));
    if (models.value && accept(models.value)) return models.value;
    await wait(pollMs);
  }
  return null;
}

/** Waits for `/v1/models` to answer `ready` (true) or the limit (false). */
export async function waitUntilReady(options) {
  return (await waitForServer((models) => models.mode === "ready", options)) !== null;
}

const isBack = (models) => models.mode === "ready" || models.mode === "setup";

/**
 * Waits out a restart. A server back in setup mode (it restarted before
 * the install completed) is asked for the download status again, so the
 * page follows it instead of waiting for `ready`. Null on timeout.
 */
async function factsAfterRestart(timing) {
  const models = await waitForServer(isBack, timing);
  if (!models) return null;
  if (models.mode === "ready") return { models };
  const status = await attempt(downloadStatus);
  return { models, status: status.value, statusError: status.error };
}

/**
 * Follows a download to its end, also when the end is a restart that took
 * the status with it. Returns `{action}`: `finished` (with `status`, the
 * caller handles done/failed/cancelled), `reload`, `restart_timeout` (the
 * server did not come back), `show_start`, or `show_error` (with `error`).
 * `onRestarting` is called each time a restart is detected, before waiting
 * for it; a server back in setup mode is followed again.
 */
export async function settleDownload(first, onView, { wait = sleep, now = Date.now, intervalMs = POLL_MS, onRestarting = () => {} } = {}) {
  let current = first;
  for (;;) {
    const followed = await attempt(() => followDownload(current, onView, { wait, intervalMs }));
    const last = followed.value;
    if (!followed.error && last?.state !== "idle") return { action: "finished", status: last };
    let facts = await reconcile(followed.error ? { statusError: followed.error } : { status: last });
    if (afterLostStatus(facts) === "wait_restart") {
      onRestarting();
      facts = await factsAfterRestart({ wait, now });
      if (!facts) return { action: "restart_timeout" };
    }
    const decision = afterLostStatus(facts);
    if (decision === "resume") {
      current = facts.status;
      continue;
    }
    return decision === "show_error" ? { action: decision, error: facts.statusError } : { action: decision };
  }
}
