// A language change while the setup panel shows a download (step 77 review
// P2-1): the panel is drawn again as it is now, never from an older status.
import { test } from "node:test";
import assert from "node:assert/strict";
import { fakeFetch, installPage } from "./fake-dom.mjs";

const GB = 1024 ** 3;
const doc = installPage();
const running = {
  state: "running",
  models: [{ id: "gen", state: "downloading", received: 1.2 * GB, total: 2.7 * GB }],
  error: null,
  restart: null,
};
const statusFails = [500, { error: { code: "internal", message: "status store gone" } }];
fakeFetch({
  "GET /v1/models": () => [200, {
    mode: "setup",
    models_dir: "/m",
    embedding: { id: "embed", installed: false },
    generation: { id: "gen", installed: false },
    download: { available: true, bytes: 3.1 * GB, code: null, reason: null },
  }],
  // A reload finds the download running; the next poll (and the check
  // after it) cannot read the status any more.
  "GET /v1/models/download": (n) => (n === 1 ? [200, running] : statusFails),
});

const { setLanguage } = await import("../i18n.js");
const { initSetup } = await import("../setup.js");
const $ = (id) => doc.getElementById(id);

async function until(check, limitMs = 5000) {
  const end = Date.now() + limitMs;
  while (!check()) {
    if (Date.now() > end) throw new Error("timed out");
    await new Promise((r) => setTimeout(r, 20));
  }
}

test("a failed status poll stays a failure in the new language", async () => {
  await initSetup();
  await until(() => !$("setup-download-again").hidden);
  assert.equal($("setup-download-cancel").hidden, true);
  assert.equal($("setup-download-again").textContent, "Tekrar dene");
  assert.match($("setup-status").textContent, /Sunucuda bir hata oluştu/);
  assert.match($("setup-progress-amount").textContent, /1,2 GB \/ 2,7 GB/);

  setLanguage("en");
  assert.equal($("setup-download-again").hidden, false, "the retry button stays");
  assert.equal($("setup-download-again").textContent, "Try again");
  assert.equal($("setup-download-cancel").hidden, true, "nothing is being followed");
  assert.match($("setup-status").textContent, /Something went wrong on the server/);
  assert.doesNotMatch($("setup-status").textContent, /Downloading the models/);
  // The rows on screen are written again in the new language.
  assert.match($("setup-progress-amount").textContent, /1\.2 GB \/ 2\.7 GB/);
  assert.match($("setup-progress-models").textContent, /Downloading/);
  assert.equal($("setup-download-start").textContent, "Download models (3.1 GB)");
  setLanguage("tr");
});

test("a restart timeout notice stays in the new language", async () => {
  const { showRestartTimeout } = await import("../setup.js");
  showRestartTimeout();
  setLanguage("en");
  assert.match($("setup-status").textContent, /did not come back within a minute/);
  assert.equal($("setup-download-cancel").hidden, true);
  setLanguage("tr");
  assert.match($("setup-status").textContent, /bir dakika içinde geri gelmedi/);
});
