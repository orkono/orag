// Setup mode (D-023): the configured models are not installed. One button
// downloads them from Hugging Face (step 76); a local folder of model packs
// still works offline. ORAG then restarts, by itself under `orag start`, or by
// the user otherwise.
import { $, el } from "./dom.js";
import { errorBox, getJson, request } from "./api.js";
import {
  DOWNLOAD_TEXT as T,
  cancelDownload,
  downloadButtonLabel,
  downloadStatus,
  downloadView,
  followDownload,
  startDownload,
  unavailableText,
} from "./download.js";

const HEALTH_POLL_MS = 1000;
const HEALTH_WAIT_MS = 60000;
const MODELS_POLL_MS = 2000;

let following = false;
let lastPhase = null;

function isWindows() {
  return /win/i.test(navigator.userAgentData?.platform || navigator.platform || "");
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function showStatus(...nodes) {
  $("setup-status").replaceChildren(...nodes);
}

function showImportStatus(...nodes) {
  $("setup-import-status").replaceChildren(...nodes);
}

function missingIds(models) {
  return [models.embedding, models.generation].filter((m) => !m.installed).map((m) => m.id);
}

/** Turns off what cannot work without models, with the reason. */
function lockWork() {
  document.body.classList.add("setup-mode");
  for (const id of ["ask-text", "ask-button", "upload-file"]) {
    const node = $(id);
    if (node) node.disabled = true;
  }
  const text = $("ask-text");
  if (text) text.placeholder = "Soru sormak için önce modelleri kurun.";
}

async function waitForRestart() {
  showStatus(el("p", "ORAG yeniden başlıyor…", "hint"));
  const deadline = Date.now() + HEALTH_WAIT_MS;
  await sleep(HEALTH_POLL_MS);
  while (Date.now() < deadline) {
    try {
      const models = await getJson("/v1/models");
      if (models.mode === "ready") {
        location.reload();
        return;
      }
    } catch {
      // Still restarting.
    }
    await sleep(HEALTH_POLL_MS);
  }
  showStatus(el("p", "ORAG bir dakika içinde geri gelmedi. Sunucu penceresindeki iletiyi kontrol edin.", "failed"));
}

async function waitForManualRestart() {
  showStatus(el("p", "Modeller kuruldu; ORAG'ı kapatıp yeniden başlatın. Yeniden başladığında bu sayfa kendiliğinden yenilenir.", "hint keep"));
  for (;;) {
    await sleep(MODELS_POLL_MS);
    try {
      if ((await getJson("/v1/models")).mode === "ready") {
        location.reload();
        return;
      }
    } catch {
      // Stopped for the restart; keep waiting.
    }
  }
}

/** After the models are in: the restart the server announced. */
async function afterInstall(restart) {
  if (restart === "automatic") return waitForRestart();
  if (restart === "manual") return waitForManualRestart();
  try {
    if ((await getJson("/v1/models")).mode === "ready") return location.reload();
  } catch {
    // Restarting after all.
  }
  return waitForManualRestart();
}

/* The download: the start button, then per-model progress. */

function progressRow(index) {
  const id = `setup-progress-${index}`;
  const label = el("label");
  label.htmlFor = id;
  const head = el("div", undefined, "setup-model-head");
  head.append(label, el("span", "", "setup-model-state"));
  const bar = el("progress");
  bar.id = id;
  const item = el("li", undefined, "setup-model");
  item.append(head, bar, el("span", "", "setup-amount"));
  return item;
}

function fillBar(bar, received, total) {
  bar.max = total || 1;
  bar.value = Math.min(received, bar.max);
}

function renderRows(models) {
  const list = $("setup-progress-models");
  if (list.children.length !== models.length) list.replaceChildren(...models.map((_, i) => progressRow(i)));
  models.forEach((model, i) => {
    const item = list.children[i];
    item.querySelector("label").textContent = model.id;
    item.querySelector(".setup-model-state").textContent = model.stateText;
    fillBar(item.querySelector("progress"), model.received, model.total);
    item.querySelector(".setup-amount").textContent = `${model.amount} · ${model.percent}`;
  });
}

/** The sentence for a phase, read out once when the phase changes. */
function announce(view) {
  if (view.phase === "running") showStatus(el("p", T.running, "hint"));
  else if (view.phase === "cancelled") showStatus(el("p", T.cancelled, "warning"));
  else if (view.phase === "done") showStatus(el("p", T.done, "hint"));
  else if (view.phase === "failed") showStatus(errorBox({ message: view.error.detail }, view.error.text));
}

/** Moves focus to the visible action when the focused button went away. */
function keepFocus() {
  const active = document.activeElement;
  if (active && active !== document.body && active.offsetParent !== null) return;
  for (const id of ["setup-download-cancel", "setup-download-again"]) {
    if (!$(id).hidden) return $(id).focus();
  }
}

function renderView(view) {
  if (view.phase === "idle") return showStart();
  $("setup-download").hidden = true;
  $("setup-progress").hidden = false;
  renderRows(view.models);
  fillBar($("setup-progress-bar"), view.overall.received, view.overall.total);
  $("setup-progress-amount").textContent = `${view.overall.amount} · ${view.overall.percent}`;
  $("setup-download-cancel").hidden = !view.busy;
  const again = $("setup-download-again");
  again.hidden = view.phase !== "failed" && view.phase !== "cancelled";
  again.textContent = view.phase === "cancelled" ? T.resume : T.retry;
  $("setup-import").disabled = view.busy;
  if (view.phase !== lastPhase) announce(view);
  lastPhase = view.phase;
  keepFocus();
}

function showStart() {
  $("setup-download").hidden = false;
  $("setup-progress").hidden = true;
  $("setup-import").disabled = false;
  lastPhase = "idle";
}

function setInFlight(busy) {
  for (const id of ["setup-download-start", "setup-download-cancel", "setup-download-again"]) $(id).disabled = busy;
}

/** No download here: say why and open the offline form. */
function offlineOnly(text, detail) {
  $("setup-download").hidden = true;
  $("setup-progress").hidden = true;
  const hint = $("setup-download-reason");
  hint.replaceChildren(text);
  if (detail) hint.append(el("span", detail, "detail"));
  hint.hidden = false;
  $("setup-offline").open = true;
}

/** A refused start: another tab may run the download already; follow it. */
async function startOrJoin() {
  try {
    return await startDownload();
  } catch (err) {
    if (err.code !== "conflict") throw err;
    const status = await downloadStatus();
    if (status.state === "running") return status;
    throw err;
  }
}

function showFailure(err, fallback) {
  if (err.code === "download_disabled" || err.code === "download_unavailable") {
    offlineOnly(T.byCode[err.code]);
    return;
  }
  showStatus(errorBox(err, fallback, T.byCode));
  if (!$("setup-progress").hidden) {
    $("setup-download-cancel").hidden = true;
    const again = $("setup-download-again");
    again.textContent = T.retry;
    again.hidden = false;
  }
  lastPhase = null;
  keepFocus();
}

/** Starts (or joins) a download and follows it to the end. */
async function follow(begin) {
  if (following) return;
  following = true;
  setInFlight(true);
  let fallback = T.startFailed;
  try {
    const first = await begin();
    setInFlight(false);
    fallback = T.pollFailed;
    const last = await followDownload(first, renderView);
    if (last.state === "done") await afterInstall(last.restart);
  } catch (err) {
    showFailure(err, fallback);
  } finally {
    following = false;
    setInFlight(false);
  }
}

async function cancel() {
  const button = $("setup-download-cancel");
  button.disabled = true;
  try {
    await cancelDownload(); // the running poll shows the cancelled state
  } catch (err) {
    showStatus(errorBox(err, T.cancelFailed, T.byCode));
    lastPhase = null;
    button.disabled = false;
  }
}

/** A download already running (a reload, a second tab) shows at once. */
async function joinRunning() {
  let status;
  try {
    status = await downloadStatus();
  } catch {
    return; // The start button stays.
  }
  if (status.state !== "idle") follow(async () => status);
}

function initDownload(download) {
  if (!download?.available) return offlineOnly(unavailableText(download), download?.reason);
  $("setup-download-start").textContent = downloadButtonLabel(download.bytes);
  $("setup-download-start").addEventListener("click", () => follow(startOrJoin));
  $("setup-download-again").addEventListener("click", () => follow(startOrJoin));
  $("setup-download-cancel").addEventListener("click", cancel);
  showStart();
  return joinRunning();
}

/* The offline import of model packs from a folder. */

async function importPacks(event) {
  event.preventDefault();
  const path = $("setup-path").value.trim();
  if (!path) return;
  const button = $("setup-import");
  button.disabled = true;
  showImportStatus(el("p", "İçe aktarılıyor… Büyük model dosyaları birkaç dakika sürebilir.", "hint"));
  try {
    const response = await request("/v1/models/import", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path }),
    });
    const result = await response.json();
    if (result.restart === "automatic" || result.restart === "manual") {
      showImportStatus();
      return afterInstall(result.restart);
    }
    const models = await getJson("/v1/models");
    const still = missingIds(models);
    $("setup-missing").textContent = still.join(", ");
    const imported = result.imported.length ? `İçe aktarıldı: ${result.imported.join(", ")}.` : "Bu klasördeki paketler zaten kurulu.";
    showImportStatus(el("p", `${imported} Hâlâ eksik: ${still.join(", ")}.`, "warning"));
  } catch (err) {
    showImportStatus(errorBox(err, "Model paketleri içe aktarılamadı.", {
      conflict: "Başka bir içe aktarma veya indirme sürüyor; bitmesini bekleyin.",
      invalid_input: "Bu klasörden model paketi içe aktarılamadı.",
    }));
  } finally {
    button.disabled = following;
  }
}

export async function initSetup() {
  let models;
  try {
    models = await getJson("/v1/models");
  } catch {
    return; // An older server or an unreachable one: the status bar says so.
  }
  if (models.mode !== "setup") return;
  $("setup-missing").textContent = missingIds(models).join(", ");
  $("setup-models-dir").textContent = models.models_dir;
  $("setup-path").placeholder = isWindows() ? "C:\\ORAG-paketleri" : "~/orag-packs";
  $("setup-form").addEventListener("submit", importPacks);
  $("setup-panel").hidden = false;
  lockWork();
  await initDownload(models.download);
}
