// The page in a plain browser (no `__ORAG_DESKTOP__` marker from the desktop
// app, whatever its fragment says): a language choice is stored, but the page
// never navigates anywhere for it, least of all to the app's `orag-lang:`
// report scheme.
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

test("a plain browser opened with a valid fragment never reports either", async () => {
  const page = installPage({ storage: memoryStorage(), hash: "#lang=en" });
  const fresh = await import("../i18n.js?fragment-no-app");
  const select = page.getElementById("language-select");
  fresh.initLanguage(select);
  assert.equal(fresh.getLanguage(), "en");
  select.value = "tr";
  select.dispatchEvent(new Event("change"));
  assert.equal(fresh.getLanguage(), "tr");
  assert.deepEqual(location.navigations, []);
});

test("an invalid fragment is not a desktop hand-over either", async () => {
  const page = installPage({ storage: memoryStorage(), hash: "#lang=de&at=5" });
  const fresh = await import("../i18n.js?invalid-fragment");
  fresh.initLanguage(page.getElementById("language-select"));
  fresh.setLanguage("en");
  assert.deepEqual(location.navigations, []);
});
