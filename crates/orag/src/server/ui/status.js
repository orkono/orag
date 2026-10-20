// The version and models in the sidebar footer, and the connection banner:
// while ORAG is unreachable the page says so and resumes when it is back.
import { $, el } from "./dom.js";
import { versionInfo } from "./api.js";
import { toast } from "./dialog.js";
import { setAttrText } from "./i18n.js";

const HEALTH_MS = 10000;
const HEALTH_DOWN_MS = 2000;

let down = false;

function showVersion(info) {
  const models = el("div", `${info.config.generation_model} · ${info.config.embedding_model}`, "app-models");
  setAttrText(models, "title", "status.models");
  $("app-info").replaceChildren(el("div", `ORAG ${info.version}`), models);
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
  versionInfo().catch(() => {}); // shown after the next reconnect otherwise
  watchHealth();
}
