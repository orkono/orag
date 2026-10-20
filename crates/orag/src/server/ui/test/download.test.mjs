// The in-app model download (step 76): byte and percent text, the status ->
// view model, error sentences, and the start -> poll -> done flow against an
// in-memory fake `fetch`.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  byteText,
  cancelDownload,
  downloadButtonLabel,
  downloadErrorText,
  unavailableText,
  downloadView,
  followDownload,
  percentText,
  startDownload,
} from "../download.js";
import { setLanguage } from "../i18n.js";

const GB = 1024 ** 3;
const MB = 1024 ** 2;

test("bytes use Turkish units and a decimal comma", () => {
  assert.equal(byteText(0), "0 bayt");
  assert.equal(byteText(512), "512 bayt");
  assert.equal(byteText(1536), "1,5 KB");
  assert.equal(byteText(610 * MB), "610 MB");
  assert.equal(byteText(1.2 * GB), "1,2 GB");
  assert.equal(byteText(3.24 * GB), "3,2 GB");
  assert.equal(byteText(2 * GB), "2 GB");
  assert.equal(byteText(1.5 * 1024 * GB), "1,5 TB");
});

test("bytes that are not a count read as unknown", () => {
  assert.equal(byteText(null), "?");
  assert.equal(byteText(undefined), "?");
  assert.equal(byteText(-1), "?");
  assert.equal(byteText(Number.NaN), "?");
});

test("percent is written the Turkish way and never claims 100 early", () => {
  assert.equal(percentText(0, 100), "%0");
  assert.equal(percentText(45, 100), "%45");
  assert.equal(percentText(999, 1000), "%99");
  assert.equal(percentText(100, 100), "%100");
  assert.equal(percentText(5, 0), "%0");
});

test("the start button names the total size when the server knows it", () => {
  assert.equal(downloadButtonLabel(3.2 * GB), "Modelleri indir (3,2 GB)");
  assert.equal(downloadButtonLabel(null), "Modelleri indir");
  assert.equal(downloadButtonLabel(undefined), "Modelleri indir");
});

test("a running status becomes per-model rows and an overall line", () => {
  const view = downloadView({
    state: "running",
    models: [
      { id: "embed", state: "installed", received: 0.6 * GB, total: 0.6 * GB },
      { id: "gen", state: "downloading", received: 1.2 * GB, total: 2.7 * GB },
    ],
    error: null,
    restart: null,
  });
  assert.equal(view.phase, "running");
  assert.equal(view.busy, true);
  assert.deepEqual(view.models[1], {
    id: "gen",
    state: "downloading",
    stateText: "İndiriliyor",
    received: 1.2 * GB,
    total: 2.7 * GB,
    amount: "1,2 GB / 2,7 GB",
    percent: "%44",
  });
  assert.equal(view.models[0].stateText, "Kuruldu");
  assert.equal(view.overall.amount, "1,8 GB / 3,3 GB");
  assert.equal(view.overall.percent, "%54");
  assert.equal(view.error, null);
});

test("a failed status carries a Turkish sentence and the server's detail", () => {
  const view = downloadView({
    state: "failed",
    models: [],
    error: { code: "disk", message: "need 3.4 GB in /models, 1.0 GB free" },
    restart: null,
  });
  assert.equal(view.phase, "failed");
  assert.equal(view.busy, false);
  assert.equal(view.error.text, downloadErrorText("disk"));
  assert.equal(view.error.detail, "need 3.4 GB in /models, 1.0 GB free");
});

test("error codes map to plain sentences; unknown codes get the fallback", () => {
  assert.match(downloadErrorText("network"), /İnternet bağlantı/);
  assert.match(downloadErrorText("network"), /proxy/);
  assert.match(downloadErrorText("checksum"), /bozuk/);
  assert.match(downloadErrorText("checksum"), /tekrar deneyin/i);
  assert.match(downloadErrorText("disk"), /boş alan/);
  assert.match(downloadErrorText("http"), /Hugging Face/);
  assert.equal(downloadErrorText("toString"), downloadErrorText("something_new"));
  assert.match(downloadErrorText(undefined), /indirilemedi/);
});

test("a missing or odd status is idle with no rows", () => {
  const view = downloadView(undefined);
  assert.equal(view.phase, "idle");
  assert.deepEqual(view.models, []);
  assert.equal(view.overall.percent, "%0");
  assert.equal(downloadView({ state: "weird" }).phase, "idle");
});

/** A fake server: a queue of GET statuses, and the calls it saw. */
function fakeServer(statuses, { post = { status: 202 } } = {}) {
  const calls = [];
  const json = (status, body) => new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
  globalThis.fetch = async (path, options = {}) => {
    const method = options.method || "GET";
    calls.push(`${method} ${path}`);
    if (path !== "/v1/models/download") return json(404, { error: { code: "not_found", message: "no" } });
    if (method === "POST") {
      if (post.status !== 202) return json(post.status, { error: { code: post.code, message: "nope" } });
      return json(202, statuses.length > 1 ? statuses.shift() : statuses[0]);
    }
    if (method === "DELETE") return json(200, { state: "cancelled", models: [], error: null, restart: null });
    return json(200, statuses.length > 1 ? statuses.shift() : statuses[0]);
  };
  return calls;
}

const running = (received) => ({
  state: "running",
  models: [{ id: "gen", state: "downloading", received, total: 100 }],
  error: null,
  restart: null,
});

test("click -> poll -> done: polls once a second while running", async () => {
  const done = { state: "done", models: [{ id: "gen", state: "installed", received: 100, total: 100 }], error: null, restart: "automatic" };
  const calls = fakeServer([running(0), running(40), running(90), done]);
  const waits = [];
  const seen = [];
  const first = await startDownload();
  const last = await followDownload(first, (view) => seen.push(view.models[0]?.percent ?? "-"), {
    wait: async (ms) => waits.push(ms),
  });
  assert.equal(last.state, "done");
  assert.equal(last.restart, "automatic");
  assert.deepEqual(calls, [
    "POST /v1/models/download",
    "GET /v1/models/download",
    "GET /v1/models/download",
    "GET /v1/models/download",
  ]);
  assert.deepEqual(waits, [1000, 1000, 1000]);
  assert.deepEqual(seen, ["%0", "%40", "%90", "%100"]);
});

test("a finished status is reported without polling", async () => {
  const calls = fakeServer([{ state: "failed", models: [], error: { code: "network", message: "x" }, restart: null }]);
  const seen = [];
  const last = await followDownload(await startDownload(), (view) => seen.push(view.phase), { wait: async () => {} });
  assert.equal(last.state, "failed");
  assert.deepEqual(seen, ["failed"]);
  assert.deepEqual(calls, ["POST /v1/models/download"]);
});

test("a refused start throws the API error with its code", async () => {
  fakeServer([running(0)], { post: { status: 409, code: "conflict" } });
  await assert.rejects(startDownload(), (err) => err.status === 409 && err.code === "conflict");
});

test("cancel sends DELETE and returns the status", async () => {
  const calls = fakeServer([running(10)]);
  const status = await cancelDownload();
  assert.equal(status.state, "cancelled");
  assert.deepEqual(calls, ["DELETE /v1/models/download"]);
});

test("an unavailable download is explained in Turkish by its code", () => {
  assert.match(unavailableText({ code: "download_disabled" }), /kapatılmış/);
  assert.match(unavailableText({ code: "download_unavailable" }), /indirilemiyor/);
  assert.match(unavailableText({ code: "conflict" }), /kurulu|indiremiyor/);
  assert.match(unavailableText({ code: "toString" }), /indiremiyor/);
  assert.match(unavailableText(undefined), /indiremiyor/);
});

test("the download view and its sentences follow the language", () => {
  setLanguage("en");
  try {
    assert.equal(downloadButtonLabel(3.1 * GB), "Download models (3.1 GB)");
    assert.equal(downloadButtonLabel(null), "Download models");
    const view = downloadView({
      state: "failed",
      models: [{ id: "gen", state: "downloading", received: 1.2 * GB, total: 2.7 * GB }],
      error: { code: "disk", message: "x" },
    });
    assert.equal(view.models[0].stateText, "Downloading");
    assert.equal(view.models[0].amount, "1.2 GB / 2.7 GB");
    assert.equal(view.models[0].percent, "44%");
    assert.equal(view.error.key, "download.error.disk");
    assert.match(view.error.text, /disk space/);
    assert.match(unavailableText({ code: "download_disabled" }), /turned off/);
  } finally {
    setLanguage("tr");
  }
  assert.equal(downloadButtonLabel(3.1 * GB), "Modelleri indir (3,1 GB)");
});
