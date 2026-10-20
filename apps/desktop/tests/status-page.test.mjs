// The desktop status page's language (step 77 review): the app hands over a
// timed choice (window.oragLanguage), the newer of it and the page's own
// stored choice stands, and the page reports a choice the app does not know
// yet as a navigation to `orag-lang://choice?lang=<tr|en>&at=<ms>`, which the
// app cancels (src/window.rs `language_report`).
// Run with `node --test apps/desktop/tests/status-page.test.mjs`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const SOURCE = readFileSync(new URL("../ui/status.js", import.meta.url), "utf8");

class Element extends EventTarget {
  constructor() {
    super();
    this.textContent = "";
    this.hidden = false;
    this.className = "";
    this.value = "";
    this.attrs = {};
  }

  setAttribute(name, value) {
    this.attrs[name] = String(value);
  }
}

/** Loads status.js in a fresh page with `stored` in its localStorage. */
function loadPage(stored = {}) {
  const items = new Map(Object.entries(stored));
  const elements = new Map();
  const location = {
    pathname: "/index.html",
    search: "",
    hash: "",
    navigations: [],
    set href(url) {
      this.navigations.push(String(url));
    },
  };
  const window = {
    document: {
      documentElement: { lang: "" },
      getElementById: (id) => {
        if (!elements.has(id)) elements.set(id, new Element());
        return elements.get(id);
      },
    },
    localStorage: {
      getItem: (key) => (items.has(key) ? items.get(key) : null),
      setItem: (key, value) => items.set(key, String(value)),
      removeItem: (key) => items.delete(key),
    },
    location,
    history: {
      state: null,
      replaceState(_state, _title, url) {
        location.hash = String(url).slice(String(url).indexOf("#"));
      },
    },
  };
  window.window = window;
  vm.runInNewContext(SOURCE, window);
  const $ = (id) => window.document.getElementById(id);
  return { window, items, location, $ };
}

test("it opens in the stored choice, Turkish without one", () => {
  assert.equal(loadPage().$("status").textContent, "Hizmet başlatılıyor…");
  assert.equal(loadPage({ "orag.lang": "en" }).$("status").textContent, "Starting the service…");
});

test("a newer hand-over from the app wins and is stored", () => {
  const page = loadPage({ "orag.lang": "tr", "orag.lang.at": "100" });
  page.window.oragLanguage("en", 300);
  assert.equal(page.$("status").textContent, "Starting the service…");
  assert.equal(page.items.get("orag.lang"), "en");
  assert.equal(page.items.get("orag.lang.at"), "300");
  assert.deepEqual(page.location.navigations, [], "the app's own choice is not reported back");
});

test("a stale or default hand-over keeps a newer stored choice, and says so", () => {
  const page = loadPage({ "orag.lang": "en", "orag.lang.at": "200" });
  page.window.oragLanguage("tr", 100);
  assert.equal(page.$("status").textContent, "Starting the service…");
  assert.deepEqual(page.location.navigations, ["orag-lang://choice?lang=en&at=200"], "the app learns the newer choice");
  page.window.oragLanguage("tr", null);
  assert.equal(page.$("status").textContent, "Starting the service…");
  const untimed = loadPage({ "orag.lang": "en" });
  untimed.window.oragLanguage("tr", null);
  assert.equal(untimed.$("status").textContent, "Starting the service…");
  untimed.window.oragLanguage("de", 999);
  assert.equal(untimed.$("status").textContent, "Starting the service…", "unknown languages are ignored");
  assert.deepEqual(untimed.location.navigations, [], "an untimed choice is never reported");
});

test("a choice on the page is stored with its time and written into the address", () => {
  const page = loadPage();
  const before = Date.now();
  const select = page.$("language-select");
  select.value = "en";
  select.dispatchEvent(new Event("change"));
  const at = Number(page.items.get("orag.lang.at"));
  assert.ok(at >= before && at <= Date.now(), String(at));
  assert.deepEqual(page.location.navigations, [`orag-lang://choice?lang=en&at=${at}`]);
  assert.equal(page.location.hash, "", "nothing is written into the address");
  assert.equal(page.$("status").textContent, "Starting the service…");
});

test("a choice made after the clock moved back still beats the standing one", () => {
  const future = Date.now() + 3_600_000;
  const page = loadPage({ "orag.lang": "en", "orag.lang.at": String(future) });
  const select = page.$("language-select");
  select.value = "tr";
  select.dispatchEvent(new Event("change"));
  const at = Number(page.items.get("orag.lang.at"));
  assert.ok(at > future, `${at} must be after ${future}`);
  assert.deepEqual(page.location.navigations, [`orag-lang://choice?lang=tr&at=${at}`]);
});
