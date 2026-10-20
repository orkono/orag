// A configured generator outside the download catalog (review P2-3): the
// setup panel still offers the tiers, with the default selected, says the
// configured model cannot be downloaded and will be replaced, and the press
// sends the picked tier.
import { test } from "node:test";
import assert from "node:assert/strict";
import { installPage } from "./fake-dom.mjs";

const GB = 1024 ** 3;
const doc = installPage();
const $ = (id) => doc.getElementById(id);

const option = (id, size, bytes, need, verdict) => ({
  id,
  size,
  current: false,
  label_key: `setup.tier.${size}`,
  bytes,
  download_bytes: bytes,
  installed: false,
  min_ram_bytes: need,
  recommended_ram_bytes: need + GB,
  speed: { metal_p50_answer_s: null, cpu_p50_answer_s: null },
  verdict,
  warnings: [],
});

const models = {
  mode: "setup",
  models_dir: "/m",
  embedding: { id: "qwen3-embedding-0.6b-q8_0", installed: false },
  generation: { id: "my-old-custom-model", installed: false },
  machine: { total_ram_bytes: 16 * GB, available_ram_bytes: 12 * GB, metal: true, free_disk_bytes: 100 * GB },
  download: {
    available: true,
    code: null,
    reason: null,
    bytes: 3 * GB,
    configured_downloadable: false,
    embedding_bytes: 0.5 * GB,
    options: [
      option("qwen3.5-4b-q4_k_m", "default", 2.5 * GB, 6.5 * GB, "recommended"),
      option("qwen3.5-9b-q4_k_m", "large", 5.5 * GB, 9 * GB, "fits"),
    ],
    selected: "qwen3.5-4b-q4_k_m",
    warnings: [],
    support: { status: "supported", code: null, missing_cpu_features: [] },
  },
};

const posts = [];
globalThis.fetch = async (path, options = {}) => {
  const method = options.method || "GET";
  const json = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
  if (method === "GET" && path === "/v1/models") return json(models);
  if (method === "GET" && path === "/v1/models/download") return json({ state: "idle", models: [], error: null, restart: null });
  if (method === "POST" && path === "/v1/models/download") {
    posts.push(JSON.parse(options.body));
    return json({ error: { code: "conflict", message: "busy" } }, 409);
  }
  return json({ error: { code: "not_found", message: path } }, 404);
};

const { setLanguage } = await import("../i18n.js");
const { initSetup } = await import("../setup.js");

async function until(check, limitMs = 5000) {
  const end = Date.now() + limitMs;
  while (!check()) {
    if (Date.now() > end) throw new Error("timed out");
    await new Promise((r) => setTimeout(r, 10));
  }
}

test("the tiers are offered with a note that the configured model will be replaced", async () => {
  await initSetup();
  assert.equal($("setup-download").hidden, false, "the download button is offered");
  assert.equal($("setup-download-reason").hidden, true, "no offline-only reason");
  assert.equal($("setup-tiers").hidden, false);
  const checked = $("setup-tier-list").querySelectorAll("input").find((r) => r.checked);
  assert.equal(checked.value, "qwen3.5-4b-q4_k_m");
  const note = $("setup-choice-note");
  assert.equal(note.hidden, false);
  assert.match(note.textContent, /my-old-custom-model/);
  assert.match(note.textContent, /indirilemiyor/);
  assert.equal($("setup-missing").textContent, "qwen3-embedding-0.6b-q8_0, qwen3.5-4b-q4_k_m");
  setLanguage("en");
  assert.match($("setup-choice-note").textContent, /cannot be downloaded.*replaces it/);
  setLanguage("tr");
});

test("the press sends the picked tier", async () => {
  $("setup-download-start").click();
  await until(() => posts.length === 1);
  assert.deepEqual(posts[0], { generation_model: "qwen3.5-4b-q4_k_m" });
});
