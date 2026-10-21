// The version, models, graphics card and last answer time in the sidebar
// footer, and the connection banner: while ORAG is unreachable the page says
// so and resumes when it is back (also after "try the graphics card again",
// which restarts the service).
import { $, el } from "./dom.js";
import { request, versionInfo } from "./api.js";
import { toast } from "./dialog.js";
import { setAttrText, setText } from "./i18n.js";
import { answerTimeText, gpuLine } from "./gpu.js";

const HEALTH_MS = 10000;
const HEALTH_DOWN_MS = 2000;

let down = false;
/** The last `/v1/version` answer and answer time, redrawn on a language change. */
let shown = null;
let lastAnswerMs = null;

function showVersion(info) {
  shown = info;
  const models = el("div", `${info.config.generation_model} · ${info.config.embedding_model}`, "app-models");
  setAttrText(models, "title", "status.models");
  const rows = [el("div", `ORAG ${info.version}`), models];
  const gpu = gpuLine(info.gpu);
  if (gpu) {
    const line = el("div", gpu.text, "app-gpu");
    line.title = gpu.text;
    rows.push(line);
    if (gpu.retry) rows.push(retryButton());
  }
  const time = answerTimeText(lastAnswerMs, info.gpu?.backend === "cpu" || !info.gpu?.in_use ? "cpu" : info.gpu.backend);
  if (time) rows.push(el("div", time, "app-answer-time"));
  $("app-info").replaceChildren(...rows);
}

/** Ends the CPU service; the desktop app clears its fallback and tries the GPU. */
function retryButton() {
  const button = el("button", undefined, "button ghost small");
  button.type = "button";
  setText(button, "gpu.retry");
  button.addEventListener("click", async () => {
    button.disabled = true;
    try {
      await request("/v1/system/gpu-retry", { method: "POST" });
    } catch {
      button.disabled = false;
      toast({ key: "gpu.retryFailed" });
    }
  });
  return button;
}

async function reachable() {
  try {
    // A server that accepts but never answers counts as down too.
    const signal = AbortSignal.timeout(HEALTH_DOWN_MS * 2);
    return (await fetch("/v1/health", { cache: "no-store", signal })).ok;
  } catch {
    return false;
  }
}

async function watchHealth() {
  const ok = await reachable();
  if (!ok && !down) {
    down = true;
    $("offline-banner").hidden = false;
  } else if (ok && down) {
    down = false;
    $("offline-banner").hidden = true;
    toast({ key: "status.reconnected" });
    versionInfo(true).catch(() => {});
    document.dispatchEvent(new Event("orag:reconnected"));
  }
  setTimeout(watchHealth, down ? HEALTH_DOWN_MS : HEALTH_MS);
}

export function initStatus() {
  document.addEventListener("orag:version", ({ detail }) => showVersion(detail));
  document.addEventListener("orag:answered", ({ detail }) => {
    lastAnswerMs = detail.generationMs;
    if (shown) showVersion(shown);
  });
  window.addEventListener("orag:language", () => {
    if (shown) showVersion(shown);
  });
  versionInfo().catch(() => {}); // shown after the next reconnect otherwise
  watchHealth();
}
