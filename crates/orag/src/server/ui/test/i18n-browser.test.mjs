// The page in a plain browser (no `#lang=` hand-over from the desktop app):
// a language choice is stored, but the page never navigates anywhere for it,
// least of all to the app's `orag-lang:` report scheme.
import { test } from "node:test";
import assert from "node:assert/strict";
import { installPage, memoryStorage } from "./fake-dom.mjs";

const storage = memoryStorage({ "orag.lang": "en", "orag.lang.at": "500" });
const doc = installPage({ storage });
const i18n = await import("../i18n.js?plain-browser");

test("a plain browser never reports a language choice", () => {
  const select = doc.getElementById("language-select");
  i18n.initLanguage(select);
  assert.equal(i18n.getLanguage(), "en");
  select.value = "tr";
  select.dispatchEvent(new Event("change"));
  i18n.setLanguage("en");
  assert.equal(i18n.getLanguage(), "en");
  assert.equal(storage.items.get("orag.lang"), "en");
  assert.deepEqual(location.navigations, []);
  assert.equal(location.hash, "", "and the address never gets a fragment");
});

test("an invalid fragment is not a desktop hand-over either", async () => {
  const page = installPage({ storage: memoryStorage(), hash: "#lang=de&at=5" });
  const fresh = await import("../i18n.js?invalid-fragment");
  fresh.initLanguage(page.getElementById("language-select"));
  fresh.setLanguage("en");
  assert.deepEqual(location.navigations, []);
});
