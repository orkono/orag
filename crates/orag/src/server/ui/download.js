// The in-app model download (step 76): the `/v1/models/download` calls and
// the pure logic that turns a status body into what the setup panel shows.
// No DOM access, so `node --test` covers it (`test/download.test.mjs`).
import { getJson, request } from "./api.js";

const ENDPOINT = "/v1/models/download";
const MODELS_ENDPOINT = "/v1/models";
const POLL_MS = 1000;
const RESTART_POLL_MS = 1000;
const RESTART_LIMIT_MS = 60000;

/** Every user-visible sentence of the download, in one place. */
export const DOWNLOAD_TEXT = {
  button: "Modelleri indir",
  running: "Modeller indiriliyor… Bu sayfayı kapatsanız da indirme sürer.",
  cancelled: "İndirme durduruldu. Kaldığı yerden devam edebilirsiniz.",
  done: "Modeller indirildi ve doğrulandı.",
  retry: "Tekrar dene",
  resume: "Devam et",
  installed: "Modeller zaten kurulu; indirilecek bir şey yok.",
  unavailable: "Bu sunucu modelleri kendisi indiremiyor; model paketlerini aşağıdan çevrimdışı kurun.",
  startFailed: "İndirme başlatılamadı.",
  cancelFailed: "İndirme durdurulamadı.",
  pollFailed: "İndirmenin durumu okunamadı.",
  byCode: {
    conflict: "Başka bir indirme veya içe aktarma sürüyor; bitmesini bekleyin.",
    download_disabled: "Model indirme bu kurulumda kapatılmış (config.toml: models.download). Çevrimdışı kurulumu kullanın.",
    download_unavailable: "Bu modeller ORAG içinden indirilemiyor. Çevrimdışı kurulumu kullanın.",
  },
};

const STATE_TEXT = new Map([
  ["waiting", "Sırada"],
  ["downloading", "İndiriliyor"],
  ["verifying", "Doğrulanıyor"],
  ["installed", "Kuruldu"],
  ["failed", "Başarısız"],
]);

/** What a failed download's `error.code` means for the user. */
const ERROR_TEXT = new Map([
  ["network", "Hugging Face'e bağlanılamadı. İnternet bağlantınızı ve varsa proxy (vekil sunucu) ayarını kontrol edip tekrar deneyin."],
  ["http", "Hugging Face isteği kabul etmedi. Biraz sonra tekrar deneyin."],
  ["checksum", "İndirilen dosya bozuk çıktı ve silindi. Tekrar deneyin."],
  ["disk", "Diskte yeterli boş alan yok. Yer açıp tekrar deneyin."],
  ["cancelled", "İndirme durduruldu."],
  ["internal", "Sunucuda bir hata oluştu; ayrıntılar sunucu günlüğünde."],
]);
const ERROR_FALLBACK = "Modeller indirilemedi. Tekrar deneyin.";

const PHASES = new Set(["idle", "running", "done", "failed", "cancelled"]);
const UNITS = ["bayt", "KB", "MB", "GB", "TB"];

const isCount = (value) => typeof value === "number" && Number.isFinite(value) && value >= 0;

/** A byte count with Turkish units and a decimal comma: `3,2 GB`. */
export function byteText(bytes) {
  if (!isCount(bytes)) return "?";
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const rounded = unit === 0 ? Math.round(value) : Math.round(value * 10) / 10;
  return `${String(rounded).replace(".", ",")} ${UNITS[unit]}`;
}

/** `%45`; rounded down so a file is at `%100` only when it is all there. */
export function percentText(received, total) {
  if (!isCount(received) || !isCount(total) || total === 0) return "%0";
  return `%${Math.floor((Math.min(received, total) / total) * 100)}`;
}

export function downloadButtonLabel(bytes) {
  return isCount(bytes) ? `${DOWNLOAD_TEXT.button} (${byteText(bytes)})` : DOWNLOAD_TEXT.button;
}

/** Why there is no download, in Turkish; the server's English reason is detail. */
export function unavailableText(download) {
  const code = download?.code;
  if (code === "conflict") return DOWNLOAD_TEXT.installed;
  return (Object.hasOwn(DOWNLOAD_TEXT.byCode, code) && DOWNLOAD_TEXT.byCode[code]) || DOWNLOAD_TEXT.unavailable;
}

export function downloadErrorText(code) {
  return ERROR_TEXT.get(code) || ERROR_FALLBACK;
}

function amount(received, total) {
  return { amount: `${byteText(received)} / ${byteText(total)}`, percent: percentText(received, total) };
}

function modelRow(model) {
  const received = isCount(model.received) ? model.received : 0;
  const total = isCount(model.total) ? model.total : 0;
  return {
    id: String(model.id),
    state: model.state,
    stateText: STATE_TEXT.get(model.state) || "",
    received,
    total,
    ...amount(received, total),
  };
}

/** A status body as the panel's view: phase, rows, overall line, error. */
export function downloadView(status) {
  const phase = PHASES.has(status?.state) ? status.state : "idle";
  const models = Array.isArray(status?.models) ? status.models.map(modelRow) : [];
  const received = models.reduce((sum, m) => sum + m.received, 0);
  const total = models.reduce((sum, m) => sum + m.total, 0);
  const error = phase === "failed"
    ? { code: status.error?.code ?? null, text: downloadErrorText(status.error?.code), detail: status.error?.message || "" }
    : null;
  return {
    phase,
    busy: phase === "running",
    models,
    overall: { received, total, ...amount(received, total) },
    error,
    restart: status?.restart ?? null,
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
 * Polls `/v1/models` until it answers `ready` (true) or `limitMs` passes
 * (false). Connection errors mean the restart is still under way.
 */
export async function waitUntilReady({ wait = sleep, now = Date.now, pollMs = RESTART_POLL_MS, limitMs = RESTART_LIMIT_MS } = {}) {
  const deadline = now() + limitMs;
  await wait(pollMs);
  while (now() < deadline) {
    const models = await attempt(() => getJson(MODELS_ENDPOINT));
    if (models.value?.mode === "ready") return true;
    await wait(pollMs);
  }
  return false;
}

/**
 * Follows a download to its end, also when the end is a restart that took
 * the status with it. Returns `{action}`: `finished` (with `status`, the
 * caller handles done/failed/cancelled), `reload`, `restart_timeout`,
 * `show_start`, or `show_error` (with `error`). `onRestarting` is called
 * once a restart is detected, before waiting for it.
 */
export async function settleDownload(first, onView, { wait = sleep, now = Date.now, intervalMs = POLL_MS, onRestarting = () => {} } = {}) {
  let current = first;
  for (;;) {
    const followed = await attempt(() => followDownload(current, onView, { wait, intervalMs }));
    const last = followed.value;
    if (!followed.error && last?.state !== "idle") return { action: "finished", status: last };
    const facts = await reconcile(followed.error ? { statusError: followed.error } : { status: last });
    const decision = afterLostStatus(facts);
    if (decision === "resume") {
      current = facts.status;
      continue;
    }
    if (decision === "wait_restart") {
      onRestarting();
      return { action: (await waitUntilReady({ wait, now })) ? "reload" : "restart_timeout" };
    }
    return decision === "show_error" ? { action: decision, error: facts.statusError } : { action: decision };
  }
}
