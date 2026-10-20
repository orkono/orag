// The desktop app hands the language to the page in the URL fragment
// (`#lang=en`, step 77 review P2-2) and reads it back from there: the page
// adopts a valid value, stores it, and keeps the fragment in step.
import { test } from "node:test";
import assert from "node:assert/strict";
import { installPage, memoryStorage } from "./fake-dom.mjs";

const storage = memoryStorage({ "orag.lang": "tr" });
const doc = installPage({ storage, hash: "#lang=en&at=1000" });
const i18n = await import("../i18n.js");

test("a timed fragment wins over an older stored choice and is stored", () => {
  assert.equal(i18n.initialLanguage(), "en");
  i18n.initLanguage(doc.getElementById("language-select"));
  assert.equal(i18n.getLanguage(), "en");
  assert.equal(storage.items.get("orag.lang"), "en");
  assert.equal(storage.items.get("orag.lang.at"), "1000");
  assert.equal(location.hash, "#lang=en&at=1000");
  assert.equal(doc.documentElement.lang, "en");
  assert.equal(doc.getElementById("language-select").value, "en");
  assert.equal(doc.getElementById("setup-title").textContent, "Install models");
});

test("a page opened with the fragment keeps it in step with the menu", () => {
  i18n.setLanguage("tr");
  assert.match(location.hash, /^#lang=tr&at=\d+$/);
  i18n.setLanguage("en");
  assert.match(location.hash, /^#lang=en&at=\d+$/);
});

test("the fragment carries when the choice was made", () => {
  assert.deepEqual(i18n.choiceFromHash("#lang=en&at=1700000000000"), { language: "en", at: 1700000000000 });
  assert.deepEqual(i18n.choiceFromHash("#lang=tr"), { language: "tr", at: null });
  assert.deepEqual(i18n.choiceFromHash("#lang=tr&at=soon"), { language: "tr", at: null });
  assert.deepEqual(i18n.choiceFromHash("#lang=tr&at=-5"), { language: "tr", at: null });
  assert.equal(i18n.choiceFromHash("#at=5"), null);
});

test("a later choice wins; a hand-over without a time never overrides a stored choice", () => {
  const choose = (handed, stored) => i18n.chooseLanguage(handed, stored);
  const c = (language, at = null) => ({ language, at });
  // The desktop's stale value must not downgrade a newer choice made on the page.
  assert.deepEqual(choose(c("tr", 100), c("en", 200)), c("en", 200));
  assert.deepEqual(choose(c("en", 300), c("tr", 200)), c("en", 300));
  assert.deepEqual(choose(c("en", 200), c("tr", 200)), c("en", 200), "a tie goes to the hand-over");
  // The desktop's default (no choice seen yet) keeps a stored one, timed or not.
  assert.deepEqual(choose(c("tr"), c("en", 200)), c("en", 200));
  assert.deepEqual(choose(c("tr"), c("en")), c("en"));
  // A timed choice beats an untimed one (stored by an earlier version).
  assert.deepEqual(choose(c("tr", 5), c("en")), c("tr", 5));
  assert.deepEqual(choose(c("en"), null), c("en"));
  assert.deepEqual(choose(null, c("en", 9)), c("en", 9));
  assert.deepEqual(choose(null, null), c("tr"));
});

test("a choice on the page is stored and written with its time", () => {
  const before = Date.now();
  i18n.setLanguage("en");
  const at = Number(storage.items.get("orag.lang.at"));
  assert.ok(at >= before && at <= Date.now(), String(at));
  assert.equal(location.hash, `#lang=en&at=${at}`);
});

test("the fragment is read strictly", () => {
  for (const [hash, expected] of [
    ["#lang=tr", "tr"],
    ["#lang=en", "en"],
    ["#x=1&lang=en", "en"],
    ["#lang=de", null],
    ["#lang=EN", null],
    ["#language=en", null],
    ["", null],
  ]) {
    assert.equal(i18n.languageFromHash(hash), expected, hash);
  }
  location.hash = "#lang=de";
  storage.items.set("orag.lang", "en");
  assert.equal(i18n.initialLanguage(), "en", "an invalid fragment falls back to the stored choice");
  storage.items.delete("orag.lang");
  assert.equal(i18n.initialLanguage(), "tr", "and then to Turkish");
});

// Last: it installs a second page (new globals).
test("a stale fragment does not downgrade a newer stored choice, and is corrected", async () => {
  const newer = memoryStorage({ "orag.lang": "en", "orag.lang.at": "2000" });
  const page = installPage({ storage: newer, hash: "#lang=tr&at=1500" });
  const fresh = await import("../i18n.js?stale-fragment"); // a second page load
  fresh.initLanguage(page.getElementById("language-select"));
  assert.equal(fresh.getLanguage(), "en");
  assert.equal(page.getElementById("setup-title").textContent, "Install models");
  assert.equal(newer.items.get("orag.lang"), "en");
  // The address now tells the desktop app the page's (newer) choice.
  assert.equal(location.hash, "#lang=en&at=2000");
});

test("a choice made after the clock moved back still beats the standing one", async () => {
  const future = Date.now() + 3_600_000; // chosen while the clock was an hour fast
  const storage = memoryStorage({ "orag.lang": "en", "orag.lang.at": String(future) });
  const page = installPage({ storage, hash: `#lang=en&at=${future}` });
  const fresh = await import("../i18n.js?clock-back");
  const select = page.getElementById("language-select");
  fresh.initLanguage(select);
  select.value = "tr";
  select.dispatchEvent(new Event("change"));
  const at = Number(storage.items.get("orag.lang.at"));
  assert.ok(at > future, `${at} must be after ${future}`);
  assert.equal(location.hash, `#lang=tr&at=${at}`);
});
