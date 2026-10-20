// A machine ORAG cannot run on (step 82): the setup panel shows this
// computer and why, offers no download button, and opens the offline form.
import { test } from "node:test";
import assert from "node:assert/strict";
import { fakeFetch, installPage } from "./fake-dom.mjs";

const GB = 1024 ** 3;
const doc = installPage();
const $ = (id) => doc.getElementById(id);

const calls = fakeFetch({
  "GET /v1/models": () => [200, {
    mode: "setup",
    models_dir: "C:\\Users\\u\\.orag\\models",
    embedding: { id: "qwen3-embedding-0.6b-q8_0", installed: false },
    generation: { id: "qwen3.5-4b-q4_k_m", installed: false },
    machine: {
      os: "windows",
      arch: "x86_64",
      total_ram_bytes: 32 * GB,
      available_ram_bytes: 20 * GB,
      logical_cores: 8,
      physical_cores: 4,
      cpu_model: "Intel(R) Core(TM) i7-2600",
      cpu_features: { avx2: false, fma: false, f16c: true, bmi2: false },
      apple_silicon: false,
      metal: false,
      free_disk_bytes: 100 * GB,
      gpu: { name: "AMD Radeon HD 6950", memory_bytes: 2 * GB },
    },
    download: {
      available: true,
      code: null,
      reason: null,
      bytes: 3 * GB,
      embedding_bytes: 0.6 * GB,
      options: [{
        id: "qwen3.5-4b-q4_k_m",
        size: "default",
        label_key: "setup.tier.default",
        bytes: 2.5 * GB,
        installed: false,
        min_ram_bytes: 7 * GB,
        recommended_ram_bytes: 8 * GB,
        speed: { metal_p50_answer_s: 0.9, cpu_p50_answer_s: null },
        verdict: "fits",
        warnings: [],
      }],
      selected: null,
      warnings: [],
      support: { status: "unsupported", code: "cpu_unsupported", missing_cpu_features: ["avx2", "fma", "bmi2"] },
    },
  }],
});

const { setLanguage } = await import("../i18n.js");
const { initSetup } = await import("../setup.js");

test("an unsupported processor gets the message and no download button", async () => {
  await initSetup();
  assert.equal($("setup-panel").hidden, false);
  assert.equal($("setup-machine").hidden, false);
  assert.match($("setup-machine-facts").textContent, /Intel\(R\) Core\(TM\) i7-2600/);
  assert.match($("setup-machine-facts").textContent, /4 fiziksel, 8 mantıksal/);
  assert.match($("setup-machine-facts").textContent, /AMD Radeon HD 6950 \(2 GB\)/);
  assert.equal($("setup-unsupported").hidden, false);
  assert.match($("setup-unsupported").textContent, /\(AVX2, FMA, BMI2\) desteklemiyor/);
  assert.equal($("setup-download").hidden, true, "no download button");
  assert.equal($("setup-tiers").hidden, true);
  assert.equal($("setup-offline").open, true);
  assert.ok(!calls.includes("GET /v1/models/download"), "nothing to follow");
  setLanguage("en");
  assert.match($("setup-unsupported").textContent, /lacks instructions ORAG needs \(AVX2, FMA, BMI2\)/);
  assert.match($("setup-machine-facts").textContent, /4 physical, 8 logical/);
  assert.equal($("setup-download").hidden, true);
  setLanguage("tr");
});
