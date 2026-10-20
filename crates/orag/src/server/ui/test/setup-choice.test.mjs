// The setup panel with a choice of answer model (step 82), driven through
// the real index.html in the fake DOM: the card and the list are drawn,
// a pick changes the start button's size, a language switch draws both
// again, and the press sends the picked generator.
import { test } from "node:test";
import assert from "node:assert/strict";
import { installPage } from "./fake-dom.mjs";

const GB = 1024 ** 3;
const doc = installPage();
const $ = (id) => doc.getElementById(id);

const option = (id, size, bytes, need, verdict, metal) => ({
  id,
  size,
  label_key: `setup.tier.${size}`,
  bytes,
  installed: false,
  need_ram_bytes: need,
  speed: { metal_p50_answer_s: metal, cpu_p50_answer_s: null },
  verdict,
  warnings: [],
});

const models = {
  mode: "setup",
  models_dir: "/m",
  embedding: { id: "qwen3-embedding-0.6b-q8_0", installed: false },
  generation: { id: "qwen3.5-4b-q4_k_m", installed: false },
  machine: {
    os: "macos",
    arch: "aarch64",
    total_ram_bytes: 16 * GB,
    available_ram_bytes: 10 * GB,
    logical_cores: 12,
    physical_cores: 12,
    cpu_model: "Apple M4 Pro",
    cpu_features: "not_applicable",
    apple_silicon: true,
    metal: true,
    free_disk_bytes: 200 * GB,
    gpu: null,
  },
  download: {
    available: true,
    code: null,
    reason: null,
    bytes: 3 * GB,
    embedding_bytes: 0.5 * GB,
    options: [
      option("qwen3.5-4b-q4_k_m", "default", 2.5 * GB, 6.5 * GB, "fits", 0.9),
      option("qwen3.5-9b-q4_k_m", "large", 5.5 * GB, 9 * GB, "recommended", 2.5),
    ],
    selected: "qwen3.5-9b-q4_k_m",
    warnings: [],
    support: { status: "supported", code: null, missing_cpu_features: [] },
  },
};

const idle = { state: "idle", models: [], error: null, restart: null };
const posts = [];
globalThis.fetch = async (path, options = {}) => {
  const method = options.method || "GET";
  const json = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
  if (method === "GET" && path === "/v1/models") return json(models);
  if (method === "GET" && path === "/v1/models/download") return json(idle);
  if (method === "POST" && path === "/v1/models/download") {
    posts.push(JSON.parse(options.body));
    return json({ error: { code: "model_not_eligible", message: "too_big_disk" } }, 409);
  }
  return json({ error: { code: "not_found", message: path } }, 404);
};

const { setLanguage } = await import("../i18n.js");
const { initSetup } = await import("../setup.js");

const radios = () => $("setup-tier-list").querySelectorAll("input");
const checked = () => radios().find((r) => r.checked)?.value ?? null;
const badges = () => $("setup-tier-list").querySelectorAll(".setup-badge").map((b) => b.textContent);

async function until(check, limitMs = 5000) {
  const end = Date.now() + limitMs;
  while (!check()) {
    if (Date.now() > end) throw new Error("timed out");
    await new Promise((r) => setTimeout(r, 10));
  }
}

test("the card, the list and the start button describe this machine's choice", async () => {
  await initSetup();
  assert.equal($("setup-machine").hidden, false);
  const facts = $("setup-machine-facts").textContent;
  for (const part of ["Bellek", "16 GB (şu an boş: 10 GB)", "Apple M4 Pro", "Apple GPU (Metal)", "Boş disk", "200 GB"]) {
    assert.ok(facts.includes(part), `${part} in ${facts}`);
  }
  assert.equal($("setup-tiers").hidden, false);
  assert.equal(radios().length, 2);
  assert.ok(radios().every((r) => !r.disabled));
  assert.equal(checked(), "qwen3.5-9b-q4_k_m", "the server's selection");
  assert.deepEqual(badges(), ["Uygun", "Önerilen"]);
  assert.match($("setup-tier-list").textContent, /Apple GPU'da yaklaşık 2,5 sn/);
  assert.equal($("setup-download").hidden, false);
  assert.equal($("setup-download-start").textContent, "Modelleri indir (6 GB)");
  assert.equal($("setup-unsupported").hidden, true);
});

test("a pick changes the size on the button", () => {
  const small = radios().find((r) => r.value === "qwen3.5-4b-q4_k_m");
  small.checked = true;
  small.dispatchEvent(new Event("change"));
  assert.equal(checked(), "qwen3.5-4b-q4_k_m");
  assert.equal($("setup-download-start").textContent, "Modelleri indir (3 GB)");
  assert.equal(doc.activeElement.value, "qwen3.5-4b-q4_k_m", "focus stays on the picked radio");
});

test("a language switch draws the card and the list again, keeping the pick", () => {
  setLanguage("en");
  assert.match($("setup-machine-facts").textContent, /Memory16 GB \(10 GB free now\)/);
  assert.equal($("setup-machine-title").textContent, "This computer");
  assert.deepEqual(badges(), ["Fits", "Recommended"]);
  assert.equal(checked(), "qwen3.5-4b-q4_k_m");
  assert.equal($("setup-download-start").textContent, "Download models (3 GB)");
  setLanguage("tr");
  assert.deepEqual(badges(), ["Uygun", "Önerilen"]);
});

test("the press sends the picked generator; a refusal is shown and the choice stays", async () => {
  $("setup-download-start").click();
  await until(() => posts.length === 1 && $("setup-status").textContent !== "");
  assert.deepEqual(posts[0], { generation_model: "qwen3.5-4b-q4_k_m" });
  assert.match($("setup-status").textContent, /artık uymuyor/);
  await until(() => !$("setup-download-start").disabled);
  assert.equal($("setup-tiers").hidden, false);
  assert.ok(radios().every((r) => !r.disabled), "the list is usable again");
  assert.equal(checked(), "qwen3.5-4b-q4_k_m");
});
