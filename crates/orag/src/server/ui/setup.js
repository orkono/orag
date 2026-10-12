// Setup mode (D-023): the configured models are not installed. The page
// imports the model packs from a local folder; ORAG then restarts, by itself
// under `orag start`, or by the user otherwise.
import { $, el } from "./dom.js";
import { errorBox, getJson, request } from "./api.js";

const HEALTH_POLL_MS = 1000;
const HEALTH_WAIT_MS = 60000;
const MODELS_POLL_MS = 2000;

function isWindows() {
  return /win/i.test(navigator.userAgentData?.platform || navigator.platform || "");
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function showStatus(...nodes) {
  $("setup-status").replaceChildren(...nodes);
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

async function importPacks(event) {
  event.preventDefault();
  const path = $("setup-path").value.trim();
  if (!path) return;
  const button = $("setup-import");
  button.disabled = true;
  showStatus(el("p", "İçe aktarılıyor… Büyük model dosyaları birkaç dakika sürebilir.", "hint"));
  try {
    const response = await request("/v1/models/import", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path }),
    });
    const result = await response.json();
    if (result.restart === "automatic") return waitForRestart();
    if (result.restart === "manual") return waitForManualRestart();
    const models = await getJson("/v1/models");
    const still = missingIds(models);
    $("setup-missing").textContent = still.join(", ");
    const imported = result.imported.length ? `İçe aktarıldı: ${result.imported.join(", ")}.` : "Bu klasördeki paketler zaten kurulu.";
    showStatus(el("p", `${imported} Hâlâ eksik: ${still.join(", ")}.`, "warning"));
  } catch (err) {
    showStatus(errorBox(err, "Model paketleri içe aktarılamadı.", {
      conflict: "Başka bir içe aktarma sürüyor; bitmesini bekleyin.",
      invalid_input: "Bu klasörden model paketi içe aktarılamadı.",
    }));
  } finally {
    button.disabled = false;
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
}
