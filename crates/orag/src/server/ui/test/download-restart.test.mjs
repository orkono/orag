// A download that ends in an automatic restart (P1 review finding): the poll
// that would have seen `done` fails (the server is restarting) or reaches the
// new process (state `idle`, mode `ready`). The page must reload, not offer a
// fresh download. In-memory fake `fetch`; the waits are injected.
import { test } from "node:test";
import assert from "node:assert/strict";
import { afterLostStatus, settleDownload, waitUntilReady } from "../download.js";

const DL = "/v1/models/download";
const MODELS = "/v1/models";

const running = (received) => ({
  state: "running",
  models: [{ id: "gen", state: "downloading", received, total: 100 }],
  error: null,
  restart: null,
});
const idle = { state: "idle", models: [], error: null, restart: null };
const failed = { state: "failed", models: [], error: { code: "network", message: "x" }, restart: null };
const ready = { mode: "ready" };
const setup = { mode: "setup" };

/** The connection refused: what `fetch` does while ORAG restarts. */
const REFUSED = "refused";
/** A 500 with the API's error body. */
const BROKEN = "broken";

/**
 * A fake ORAG answering from a script: each step is what one request gets
 * (`REFUSED`, `BROKEN`, or a body per path). The last step repeats.
 */
function scriptedServer(steps) {
  const calls = [];
  const json = (status, body) => new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
  globalThis.fetch = async (path, options = {}) => {
    calls.push(`${options.method || "GET"} ${path}`);
    const step = steps.length > 1 ? steps.shift() : steps[0];
    if (step === REFUSED) throw new TypeError("fetch failed: connection refused");
    if (step === BROKEN) return json(500, { error: { code: "internal", message: "boom" } });
    if (!(path in step)) return json(404, { error: { code: "not_found", message: path } });
    return json(200, step[path]);
  };
  return calls;
}

const noWait = { wait: async () => {} };

test("a lost status is reconciled with /v1/models", () => {
  const unreachable = Object.assign(new Error("x"), { code: "unreachable" });
  assert.equal(afterLostStatus({ status: idle, models: ready }), "reload");
  assert.equal(afterLostStatus({ statusError: unreachable, models: ready }), "reload");
  assert.equal(afterLostStatus({ statusError: unreachable, modelsError: unreachable }), "wait_restart");
  assert.equal(afterLostStatus({ status: idle, modelsError: unreachable }), "wait_restart");
  assert.equal(afterLostStatus({ status: idle, models: setup }), "show_start");
  assert.equal(afterLostStatus({ statusError: unreachable, models: setup }), "show_error");
  assert.equal(afterLostStatus({ status: running(50), models: setup }), "resume");
  assert.equal(afterLostStatus({ status: failed, models: setup }), "resume");
  assert.equal(afterLostStatus({ status: idle, models: { mode: "odd" } }), "show_start");
});

test("running -> connection refused -> new process (idle, ready): reload", async () => {
  const calls = scriptedServer([
    { [DL]: running(90) },
    REFUSED, // the poll: the old process is going down
    REFUSED, // /v1/models: still restarting
    REFUSED, // the first restart check
    { [DL]: idle, [MODELS]: ready }, // the new process
  ]);
  const phases = [];
  let restarting = 0;
  const outcome = await settleDownload(running(40), (view) => phases.push(view.phase), {
    ...noWait,
    onRestarting: () => {
      restarting += 1;
    },
  });
  assert.equal(outcome.action, "reload");
  assert.equal(restarting, 1);
  assert.ok(!phases.includes("idle"), `the start button was shown: ${phases}`);
  assert.deepEqual(calls, [`GET ${DL}`, `GET ${DL}`, `GET ${MODELS}`, `GET ${MODELS}`, `GET ${MODELS}`]);
});

test("running -> idle with mode ready (no error between): reload", async () => {
  scriptedServer([{ [DL]: running(90) }, { [DL]: idle, [MODELS]: ready }]);
  const phases = [];
  const outcome = await settleDownload(running(40), (view) => phases.push(view.phase), noWait);
  assert.equal(outcome.action, "reload");
  assert.deepEqual(phases, ["running", "running"]);
});

test("running -> idle while still in setup with models missing: the start button", async () => {
  scriptedServer([{ [DL]: idle, [MODELS]: setup }]);
  const outcome = await settleDownload(running(40), () => {}, noWait);
  assert.equal(outcome.action, "show_start");
});

test("one failed poll while the server stays in setup: following resumes", async () => {
  const done = { state: "done", models: [], error: null, restart: "manual" };
  const calls = scriptedServer([BROKEN, { [MODELS]: setup }, { [DL]: done }]);
  const outcome = await settleDownload(running(40), () => {}, noWait);
  assert.equal(outcome.action, "finished");
  assert.equal(outcome.status.state, "done");
  assert.deepEqual(calls, [`GET ${DL}`, `GET ${MODELS}`, `GET ${DL}`]);
});

test("a status that cannot be read while the server is in setup: the error", async () => {
  scriptedServer([BROKEN, { [MODELS]: setup }, BROKEN]);
  const outcome = await settleDownload(running(40), () => {}, noWait);
  assert.equal(outcome.action, "show_error");
  assert.equal(outcome.error.code, "internal");
});

test("a finished download is returned as it was", async () => {
  scriptedServer([{ [DL]: failed }]);
  const outcome = await settleDownload(running(40), () => {}, noWait);
  assert.equal(outcome.action, "finished");
  assert.equal(outcome.status.state, "failed");
});

test("a restart that never comes back gives up after the limit", async () => {
  scriptedServer([REFUSED]);
  let clock = 0;
  const outcome = await settleDownload(running(40), () => {}, {
    wait: async (ms) => {
      clock += ms;
    },
    now: () => clock,
  });
  assert.equal(outcome.action, "restart_timeout");
  assert.ok(clock >= 60000 && clock <= 62000, `gave up at ${clock} ms`);
});

test("restart wait: a new process still in setup keeps waiting; ready ends it", async () => {
  const calls = scriptedServer([REFUSED, { [MODELS]: setup }, { [MODELS]: ready }]);
  assert.equal(await waitUntilReady(noWait), true);
  assert.equal(calls.length, 3);
});

/*
 * A restart before the install completed (a crash, or the user restarted
 * ORAG): the poll and the first /v1/models fail, then the replacement server
 * answers in setup mode. The page must follow what it says about the
 * download, not wait for `ready` until it times out.
 */
const cancelledStatus = { state: "cancelled", models: [], error: { code: "cancelled", message: "x" }, restart: null };

/** The poll, /v1/models and the first restart check refused; then setup. */
function recovered(...after) {
  return scriptedServer([REFUSED, REFUSED, REFUSED, { [MODELS]: setup }, ...after]);
}

test("restart into setup with the download idle: the start controls", async () => {
  const calls = recovered({ [DL]: idle });
  const phases = [];
  const outcome = await settleDownload(running(40), (view) => phases.push(view.phase), noWait);
  assert.equal(outcome.action, "show_start");
  assert.deepEqual(calls, [`GET ${DL}`, `GET ${MODELS}`, `GET ${MODELS}`, `GET ${MODELS}`, `GET ${DL}`]);
  assert.ok(!phases.includes("idle"));
});

test("restart into setup with the download running again: following resumes", async () => {
  const done = { state: "done", models: [], error: null, restart: "manual" };
  recovered({ [DL]: running(10) }, { [DL]: done });
  const seen = [];
  const outcome = await settleDownload(running(40), (view) => seen.push(view.phase), noWait);
  assert.equal(outcome.action, "finished");
  assert.equal(outcome.status.state, "done");
  assert.deepEqual(seen, ["running", "running", "done"]);
});

test("restart into setup with the download cancelled: the cancelled state", async () => {
  recovered({ [DL]: cancelledStatus });
  const seen = [];
  const outcome = await settleDownload(running(40), (view) => seen.push(view.phase), noWait);
  assert.equal(outcome.action, "finished");
  assert.equal(outcome.status.state, "cancelled");
  assert.equal(seen.at(-1), "cancelled");
});

test("restart into setup where the status cannot be read: the error, not a timeout", async () => {
  recovered(BROKEN);
  const outcome = await settleDownload(running(40), () => {}, noWait);
  assert.equal(outcome.action, "show_error");
  assert.equal(outcome.error.code, "internal");
});
