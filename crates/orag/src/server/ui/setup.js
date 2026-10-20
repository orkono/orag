// Setup mode (D-023): the configured models are not installed. One button
// downloads them from Hugging Face (step 76), with the answer model the user
// picked from the ones that fit this computer (step 82); a local folder of
// model packs still works offline. ORAG then restarts, by itself under
// `orag start`, or by the user otherwise.
import { $, el, tEl } from "./dom.js";
import { errorBox, getJson, request } from "./api.js";
import {
  DOWNLOAD_ERRORS,
  cancelDownload,
  downloadButtonLabel,
  downloadStatus,
  downloadView,
  settleDownload,
  startDownload,
  unavailableKey,
  waitUntilReady,
} from "./download.js";
import { setAttrText, setText } from "./i18n.js";
import { chosenGenerator, draw as drawChoice, initChoice } from "./setup-choice.js";
import { choiceTotal } from "./tiers.js";

const MODELS_POLL_MS = 2000;

let following = false;
let lastPhase = null;
/** The last view shown and the size on the start button, drawn again in a new language. */
let lastView = null;
let downloadBytes = null;
/** The `/v1/models` answer, when it offers a choice of answer model. */
let choiceModels = null;

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
  if (text) setAttrText(text, "placeholder", "setup.askLocked");
}

function showRestarting() {
  showStatus(tEl("p", "setup.restarting", undefined, "hint"));
  lastPhase = null; // a download followed again after the restart is announced anew
}

export function showRestartTimeout() {
  showStatus(tEl("p", "setup.restartTimeout", undefined, "failed"));
}

async function waitForRestart() {
  showRestarting();
  if (await waitUntilReady()) location.reload();
  else showRestartTimeout();
}

async function waitForManualRestart() {
  showStatus(tEl("p", "setup.manualRestart", undefined, "hint keep"));
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
  if (view.phase === "running") showStatus(tEl("p", "download.running", undefined, "hint"));
  else if (view.phase === "cancelled") showStatus(tEl("p", "download.cancelled", undefined, "warning"));
  else if (view.phase === "done") showStatus(tEl("p", "download.done", undefined, "hint"));
  else if (view.phase === "failed") showStatus(errorBox({ message: view.error.detail }, view.error.key));
}

/** Moves focus to the visible action when the focused button went away. */
function keepFocus() {
  const active = document.activeElement;
  if (active && active !== document.body && active.offsetParent !== null) return;
  for (const id of ["setup-download-cancel", "setup-download-again"]) {
    if (!$(id).hidden) return $(id).focus();
  }
}

/** The progress rows and the overall line: the only language-dependent
 * parts of the view without a `data-i18n` key. */
function drawProgress(view) {
  renderRows(view.models);
  fillBar($("setup-progress-bar"), view.overall.received, view.overall.total);
  $("setup-progress-amount").textContent = `${view.overall.amount} · ${view.overall.percent}`;
}

/** The card and the list again (a new language, a new pick); the list hides while a download runs. */
function redrawChoice(listShown) {
  if (!choiceModels) return;
  drawChoice();
  if (!listShown) $("setup-tiers").hidden = true;
}

/** The start button: the size of what the press downloads, off without a pick. */
function labelStart() {
  const button = $("setup-download-start");
  if (!choiceModels) {
    button.textContent = downloadButtonLabel(downloadBytes);
    return;
  }
  const chosen = chosenGenerator();
  button.textContent = downloadButtonLabel(chosen ? choiceTotal(choiceModels.download, chosen) : null);
  button.disabled = following || !chosen;
}

function renderView(view) {
  lastView = view.phase === "idle" ? null : view;
  if (view.phase === "idle") return showStart();
  $("setup-download").hidden = true;
  redrawChoice(false);
  $("setup-progress").hidden = false;
  drawProgress(view);
  $("setup-download-cancel").hidden = !view.busy;
  const again = $("setup-download-again");
  again.hidden = view.phase !== "failed" && view.phase !== "cancelled";
  setText(again, view.phase === "cancelled" ? "download.resume" : "download.retry");
  $("setup-import").disabled = view.busy;
  if (view.phase !== lastPhase) announce(view);
  lastPhase = view.phase;
  keepFocus();
}

/**
 * A language change: only the text of what is on screen is written again.
 * The sentences, the error, the retry button and every notice carry
 * `data-i18n` and follow by themselves; which of them is shown (the current
 * state of the panel) is left exactly as it is. The progress numbers come
 * from the last status drawn, which is also what they show now.
 */
function relabel() {
  const progressShown = !$("setup-progress").hidden;
  redrawChoice(!progressShown);
  labelStart();
  if (lastView && progressShown) drawProgress(downloadView(lastView.status));
}

function showStart() {
  lastView = null;
  redrawChoice(true);
  labelStart();
  $("setup-download").hidden = false;
  $("setup-progress").hidden = true;
  $("setup-import").disabled = false;
  lastPhase = "idle";
}

function setInFlight(busy) {
  for (const id of ["setup-download-start", "setup-download-cancel", "setup-download-again"]) $(id).disabled = busy;
  for (const input of $("setup-tier-list").querySelectorAll("input")) if (busy) input.disabled = true;
  if (!busy && choiceModels && !$("setup-download").hidden) {
    redrawChoice(true);
    labelStart();
  }
}

/** No download here: say why (the key `reason`) and open the offline form. */
function offlineOnly(reason, detail) {
  $("setup-download").hidden = true;
  $("setup-progress").hidden = true;
  const hint = $("setup-download-reason");
  hint.replaceChildren(tEl("span", reason));
  if (detail) hint.append(el("span", detail, "detail"));
  hint.hidden = false;
  $("setup-offline").open = true;
}

/** A refused start: another tab may run the download already; follow it. */
async function startOrJoin() {
  try {
    return await startDownload(choiceModels ? chosenGenerator() : undefined);
  } catch (err) {
    if (err.code !== "conflict") throw err;
    const status = await downloadStatus();
    if (status.state === "running") return status;
    throw err;
  }
}

function showFailure(err, fallback) {
  if (err.code === "download_disabled" || err.code === "download_unavailable") {
    offlineOnly(DOWNLOAD_ERRORS[err.code]);
    return;
  }
  showStatus(errorBox(err, fallback, DOWNLOAD_ERRORS));
  if (!$("setup-progress").hidden) {
    $("setup-download-cancel").hidden = true;
    const again = $("setup-download-again");
    setText(again, "download.retry");
    again.hidden = false;
  }
  lastPhase = null;
  keepFocus();
}

/**
 * Acts on how a followed download ended. A restart can take the `done`
 * status with it; `settleDownload` then answers `reload` or a timeout.
 */
async function settled(outcome, fallback) {
  if (outcome.action === "finished") {
    if (outcome.status.state === "done") await afterInstall(outcome.status.restart);
  } else if (outcome.action === "reload") location.reload();
  else if (outcome.action === "restart_timeout") showRestartTimeout();
  else if (outcome.action === "show_start") {
    showStatus();
    showStart();
  } else showFailure(outcome.error || {}, fallback);
}

/** Starts (or joins) a download and follows it to the end. */
async function follow(begin) {
  if (following) return;
  following = true;
  setInFlight(true);
  let fallback = "download.startFailed";
  try {
    const first = await begin();
    setInFlight(false);
    fallback = "download.pollFailed";
    const outcome = await settleDownload(first, renderView, { onRestarting: showRestarting });
    await settled(outcome, fallback);
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
    showStatus(errorBox(err, "download.cancelFailed", DOWNLOAD_ERRORS));
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

/** A machine no answer model fits: say why; the offline form stays. */
function unsupportedOnly() {
  $("setup-download").hidden = true;
  $("setup-progress").hidden = true;
  $("setup-offline").open = true;
}

function initDownload(models) {
  const download = models.download;
  if (!download?.available) return offlineOnly(unavailableKey(download), download?.reason);
  const choice = initChoice(models, labelStart);
  choiceModels = choice ? models : null;
  if (choice?.unsupported) return unsupportedOnly();
  downloadBytes = download.bytes;
  labelStart();
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
  showImportStatus(tEl("p", "setup.importing", undefined, "hint"));
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
    const imported = result.imported.length
      ? tEl("span", "setup.imported", { ids: result.imported.join(", ") })
      : tEl("span", "setup.alreadyInstalled");
    const line = el("p", undefined, "warning");
    line.append(imported, " ", tEl("span", "setup.stillMissing", { ids: still.join(", ") }));
    showImportStatus(line);
  } catch (err) {
    showImportStatus(errorBox(err, "setup.importFailed", {
      conflict: "setup.importConflict",
      invalid_input: "setup.importInvalid",
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
  setAttrText($("setup-path"), "placeholder", isWindows() ? "setup.pathWindows" : "setup.pathUnix");
  $("setup-form").addEventListener("submit", importPacks);
  window.addEventListener("orag:language", relabel);
  $("setup-panel").hidden = false;
  lockWork();
  await initDownload(models);
}
