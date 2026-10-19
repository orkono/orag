// The in-app model download (step 76): the `/v1/models/download` calls and
// the pure logic that turns a status body into what the setup panel shows.
// No DOM access, so `node --test` covers it (`test/download.test.mjs`).
import { request } from "./api.js";

const ENDPOINT = "/v1/models/download";
const POLL_MS = 1000;

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

/**
 * Shows `status`, then polls once a second while it is running and shows
 * each one; returns the final status. A failed poll throws (the caller
 * tells a restart from an error).
 */
export async function followDownload(status, onView, { wait, intervalMs = POLL_MS } = {}) {
  const pause = wait || ((ms) => new Promise((resolve) => setTimeout(resolve, ms)));
  let current = status;
  onView(downloadView(current));
  while (current?.state === "running") {
    await pause(intervalMs);
    current = await downloadStatus();
    onView(downloadView(current));
  }
  return current;
}
