// The desktop app hands the language to the page in the URL fragment
// (`#lang=en`, step 77 review P2-2) and reads it back from there: the page
// adopts a valid value, stores it, and keeps the fragment in step.
import { test } from "node:test";
import assert from "node:assert/strict";
import { installPage, memoryStorage } from "./fake-dom.mjs";

const storage = memoryStorage({ "orag.lang": "tr" });
const doc = installPage({ storage, hash: "#lang=en" });
const i18n = await import("../i18n.js");

test("a valid fragment wins over the stored choice and is stored", () => {
  assert.equal(i18n.initialLanguage(), "en");
  i18n.initLanguage(doc.getElementById("language-select"));
  assert.equal(i18n.getLanguage(), "en");
  assert.equal(storage.items.get("orag.lang"), "en");
  assert.equal(doc.documentElement.lang, "en");
  assert.equal(doc.getElementById("language-select").value, "en");
  assert.equal(doc.getElementById("setup-title").textContent, "Install models");
});

test("a page opened with the fragment keeps it in step with the menu", () => {
  i18n.setLanguage("tr");
  assert.equal(location.hash, "#lang=tr");
  i18n.setLanguage("en");
  assert.equal(location.hash, "#lang=en");
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
