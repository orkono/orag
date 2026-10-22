// The GPU bench's entry on the page (v0.6 step 88): a link in the help
// panel and the one-button view at `?bench=1`, only in the desktop app on
// a pre-release, and never with Metal (macOS has no Vulkan path).
import { test } from "node:test";
import assert from "node:assert/strict";
import { benchLinkShown, benchRequested, BENCH_RUN_URL } from "../bench.js";

const info = (version, backend) => ({ version, gpu: { backend } });

test("the link shows only in the desktop app, on a pre-release, without Metal", () => {
  assert.equal(benchLinkShown(info("0.6.0-alpha.10", "vulkan"), true), true);
  assert.equal(benchLinkShown(info("0.6.0-alpha.10", "cpu"), true), true, "a CPU fallback still shows it");
  assert.equal(benchLinkShown(info("0.6.0-alpha.10", "metal"), true), false);
  assert.equal(benchLinkShown(info("0.6.0", "vulkan"), true), false, "not in a stable release");
  assert.equal(benchLinkShown(info("0.6.0-alpha.10", "vulkan"), false), false, "not in a browser");
  assert.equal(benchLinkShown(undefined, true), false);
  assert.equal(benchLinkShown({ version: "0.6.0-rc.1" }, true), false, "no gpu state: no link");
});

test("the bench view opens only from ?bench=1 in the desktop app", () => {
  assert.equal(benchRequested("?bench=1", true), true);
  assert.equal(benchRequested("?x=2&bench=1", true), true);
  assert.equal(benchRequested("?bench=1", false), false);
  assert.equal(benchRequested("?bench=0", true), false);
  assert.equal(benchRequested("", true), false);
  assert.equal(BENCH_RUN_URL, "orag-bench://run");
});
