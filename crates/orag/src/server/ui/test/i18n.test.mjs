// The page's two languages (step 77): the dictionaries match, `t()` falls
// back, the first language follows the stored choice only, and numbers
// follow the language.
import { test, afterEach } from "node:test";
import assert from "node:assert/strict";
import { MESSAGES, getLanguage, initialLanguage, setLanguage, t } from "../i18n.js";
import { byteText, formatSize, pageLabel, percentText, warningText } from "../format.js";

const GB = 1024 ** 3;

/** A Storage stand-in; `throws` makes every access fail like blocked storage. */
function fakeStorage(initial = {}, { throws = false } = {}) {
  const items = new Map(Object.entries(initial));
  const guard = () => {
    if (throws) throw new Error("SecurityError");
  };
  return {
    getItem: (key) => (guard(), items.has(key) ? items.get(key) : null),
    setItem: (key, value) => (guard(), items.set(key, String(value))),
    removeItem: (key) => (guard(), items.delete(key)),
    items,
  };
}

function withNavigator(language, languages) {
  Object.defineProperty(globalThis, "navigator", {
    value: { language, languages },
    configurable: true,
    writable: true,
  });
}

afterEach(() => {
  delete globalThis.localStorage;
  setLanguage("tr");
});

const placeholders = (text) => new Set([...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]));

test("tr and en have exactly the same keys", () => {
  const tr = Object.keys(MESSAGES.tr).sort();
  const en = Object.keys(MESSAGES.en).sort();
  assert.deepEqual(en, tr);
  assert.ok(tr.length > 100, `only ${tr.length} keys`);
});

test("every placeholder of a Turkish string exists in its English string", () => {
  for (const [key, text] of Object.entries(MESSAGES.tr)) {
    assert.deepEqual(placeholders(MESSAGES.en[key]), placeholders(text), key);
  }
});

test("every string is a non-empty string, except prefixes the word order leaves empty", () => {
  for (const lang of ["tr", "en"]) {
    for (const [key, text] of Object.entries(MESSAGES[lang])) {
      assert.equal(typeof text, "string", `${lang} ${key}`);
      if (!key.endsWith(".before") && !key.endsWith(".after")) assert.ok(text.trim(), `${lang} ${key}`);
    }
  }
});

test("English strings carry no Turkish letters", () => {
  for (const [key, text] of Object.entries(MESSAGES.en)) {
    assert.doesNotMatch(text, /[çğıöşüÇĞİÖŞÜ]/, key);
  }
});

test("t() fills placeholders and falls back to Turkish, then to the key", () => {
  setLanguage("en");
  assert.equal(t("collections.created", { name: "HR" }), 'Collection "HR" created.');
  assert.equal(t("no.such.key"), "no.such.key");
  MESSAGES.tr["test.only.tr"] = "yalnız Türkçe {x}";
  try {
    assert.equal(t("test.only.tr", { x: 1 }), "yalnız Türkçe 1");
  } finally {
    delete MESSAGES.tr["test.only.tr"];
  }
  // A placeholder without a value stays visible instead of reading "undefined".
  assert.equal(t("collections.created"), 'Collection "{name}" created.');
});

test("a count of one picks the singular form when there is one", () => {
  setLanguage("en");
  assert.equal(t("collections.count", { count: 1 }), "1 document");
  assert.equal(t("collections.count", { count: 3 }), "3 documents");
  setLanguage("tr");
  assert.equal(t("collections.count", { count: 1 }), "1 belge");
  assert.equal(t("collections.count", { count: 3 }), "3 belge");
});

test("setLanguage switches, ignores unknown languages and remembers the choice", () => {
  const storage = fakeStorage();
  globalThis.localStorage = storage;
  assert.equal(setLanguage("en"), "en");
  assert.equal(getLanguage(), "en");
  assert.equal(storage.items.get("orag.lang"), "en");
  assert.equal(t("download.button"), "Download models");
  assert.equal(setLanguage("de"), "en");
  assert.equal(getLanguage(), "en");
  setLanguage("tr");
  assert.equal(t("download.button"), "Modelleri indir");
});

test("blocked storage does not stop a language change", () => {
  globalThis.localStorage = fakeStorage({}, { throws: true });
  assert.equal(setLanguage("en"), "en");
  assert.equal(getLanguage(), "en");
});

test("the page always opens in Turkish unless English was chosen", () => {
  withNavigator("en-US", ["en-US", "en"]);
  assert.equal(initialLanguage(), "tr", "no storage at all");
  globalThis.localStorage = fakeStorage();
  assert.equal(initialLanguage(), "tr", "nothing stored, English browser");
  globalThis.localStorage = fakeStorage({ "orag.lang": "en" });
  assert.equal(initialLanguage(), "en");
  globalThis.localStorage = fakeStorage({ "orag.lang": "tr" });
  withNavigator("en-GB", ["en-GB"]);
  assert.equal(initialLanguage(), "tr");
  globalThis.localStorage = fakeStorage({ "orag.lang": "fr" });
  assert.equal(initialLanguage(), "tr", "an invalid stored value");
  globalThis.localStorage = fakeStorage({ "orag.lang": "en" }, { throws: true });
  assert.equal(initialLanguage(), "tr", "blocked storage");
});

test("byte counts follow the language: decimal comma in Turkish, point in English", () => {
  setLanguage("tr");
  assert.equal(byteText(3.1 * GB), "3,1 GB");
  assert.equal(byteText(512), "512 bayt");
  assert.equal(formatSize(1536), "1,5 KB");
  assert.equal(percentText(45, 100), "%45");
  setLanguage("en");
  assert.equal(byteText(3.1 * GB), "3.1 GB");
  assert.equal(byteText(512), "512 bytes");
  assert.equal(byteText(null), "?");
  assert.equal(formatSize(1536), "1.5 KB");
  assert.equal(formatSize(2 * 1024 * 1024), "2 MB");
  assert.equal(percentText(45, 100), "45%");
});

test("page labels and warnings follow the language", () => {
  setLanguage("en");
  assert.equal(pageLabel([3, 3]), "p. 3");
  assert.equal(pageLabel([12, 13]), "pp. 12–13");
  assert.equal(
    warningText("ocr_required: page(s) 3-5, 9 have no extractable text (scanned?) and were not indexed"),
    "Scanned pages could not be read (pp. 3–5, 9)",
  );
  assert.equal(warningText("something_new: x"), "something_new: x");
});
