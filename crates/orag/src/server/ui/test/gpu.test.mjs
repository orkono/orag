// The GPU line of the sidebar footer and the last answer's time (v0.6 step 87).
import { test } from "node:test";
import assert from "node:assert/strict";
import { gpuLine, answerTimeText } from "../gpu.js";
import { setLanguage } from "../i18n.js";

const base = { layers_on_gpu: null, layers_total: null, device: null, reason: null, retry: false };

test("a model on the graphics card names it", () => {
  setLanguage("tr");
  const line = gpuLine({ ...base, backend: "vulkan", in_use: true, full: true, device: "NVIDIA GeForce RTX 3060" });
  assert.deepEqual(line, { text: "Ekran kartı: NVIDIA GeForce RTX 3060 · kullanılıyor", retry: false });
  const partial = gpuLine({
    ...base, backend: "vulkan", in_use: true, full: false, device: "AMD Radeon RX 6600", layers_on_gpu: 20, layers_total: 33,
  });
  assert.equal(partial.text, "Ekran kartı: AMD Radeon RX 6600 · kısmen kullanılıyor (20/33 katman)");
});

test("a fallback says why and offers the retry only when allowed", () => {
  setLanguage("tr");
  const fallback = gpuLine({ ...base, backend: "cpu", in_use: false, full: false, reason: "crashed-twice", retry: true });
  assert.deepEqual(fallback, {
    text: "Ekran kartı kullanılmıyor: iki kez durdu, işlemci kullanılıyor",
    retry: true,
  });
  const noDriver = gpuLine({ ...base, backend: "cpu", in_use: false, full: false, reason: "loader-missing", retry: true });
  assert.match(noDriver.text, /Vulkan sürücüsü yok/);
  const unknown = gpuLine({ ...base, backend: "cpu", in_use: false, full: false, reason: "exit-3", retry: false });
  assert.equal(unknown.text, "Ekran kartı kullanılmıyor (exit-3)");
  assert.equal(unknown.retry, false);
});

test("the CPU by choice and Metal show no GPU line", () => {
  assert.equal(gpuLine({ ...base, backend: "cpu", in_use: false, full: false }), null);
  assert.equal(gpuLine({ ...base, backend: "metal", in_use: true, full: true }), null);
  assert.equal(gpuLine(undefined), null);
});

test("English texts", () => {
  setLanguage("en");
  assert.equal(
    gpuLine({ ...base, backend: "vulkan", in_use: true, full: true, device: "Intel Arc A770" }).text,
    "Graphics card: Intel Arc A770 · in use",
  );
  assert.equal(
    gpuLine({ ...base, backend: "cpu", in_use: false, full: false, reason: "no-eligible-device", retry: true }).text,
    "Graphics card not used: no suitable card, using the processor",
  );
  assert.equal(answerTimeText(3240, "vulkan"), "Last answer: 3.2 s · graphics card");
  setLanguage("tr");
  assert.equal(answerTimeText(108000, "cpu"), "Son yanıt: 108 sn · işlemci");
  assert.equal(answerTimeText(900, "metal"), "Son yanıt: 0,9 sn · Apple GPU");
  assert.equal(answerTimeText(undefined, "cpu"), null);
});
