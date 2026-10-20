// A small DOM for `node --test`: just enough of Element, Document and window
// for the page modules, built from the real index.html, so tests can drive
// the page glue (setup.js, dialog.js, api.js) without a browser.
import { readFileSync } from "node:fs";

const VOID = new Set(["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"]);
const PROPERTY_ATTRS = { id: "id", className: "class", title: "title", placeholder: "placeholder", htmlFor: "for", type: "type", value: "value" };

class FakeText {
  constructor(text) {
    this.data = text;
    this.parentNode = null;
  }

  get textContent() {
    return this.data;
  }
}

const kebab = (name) => name.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);

function datasetOf(element) {
  return new Proxy({}, {
    get: (_, name) => element.getAttribute(`data-${kebab(String(name))}`) ?? undefined,
    set: (_, name, value) => {
      element.setAttribute(`data-${kebab(String(name))}`, String(value));
      return true;
    },
    deleteProperty: (_, name) => {
      element.removeAttribute(`data-${kebab(String(name))}`);
      return true;
    },
  });
}

/** One compound selector: tag, `.class`, `#id`, `[attr]` and `[attr="v"]`. */
function compile(selector) {
  const parts = selector.trim().match(/^[a-zA-Z][\w-]*|\.[\w-]+|#[\w-]+|\[[^\]]+\]/g) ?? [];
  if (parts.join("") !== selector.trim()) throw new Error(`fake DOM cannot match "${selector}"`);
  return (el) => parts.every((part) => {
    if (part.startsWith(".")) return el.classList.contains(part.slice(1));
    if (part.startsWith("#")) return el.id === part.slice(1);
    if (part.startsWith("[")) {
      const m = /^\[([\w-]+)(?:="([^"]*)")?\]$/.exec(part);
      return el.hasAttribute(m[1]) && (m[2] === undefined || el.getAttribute(m[1]) === m[2]);
    }
    return el.tagName === part.toUpperCase();
  });
}

const matcher = (selectors) => {
  const tests = selectors.split(",").map(compile);
  return (el) => tests.some((test) => test(el));
};

export class FakeElement extends EventTarget {
  constructor(tag, doc) {
    super();
    this.tagName = tag.toUpperCase();
    this.ownerDocument = doc;
    this.namespaceURI = "http://www.w3.org/1999/xhtml";
    this.childNodes = [];
    this.parentNode = null;
    this.attrs = new Map();
    this.dataset = datasetOf(this);
    this.style = {};
    this.disabled = false;
    this.open = false;
  }

  get hidden() {
    return this.attrs.has("hidden");
  }

  set hidden(value) {
    if (value) this.attrs.set("hidden", "");
    else this.attrs.delete("hidden");
  }

  get children() {
    return this.childNodes.filter((n) => n instanceof FakeElement);
  }

  get textContent() {
    return this.childNodes.map((n) => n.textContent).join("");
  }

  set textContent(value) {
    this.replaceChildren(...(value === "" || value == null ? [] : [String(value)]));
  }

  get classList() {
    const names = () => (this.getAttribute("class") || "").split(/\s+/).filter(Boolean);
    const write = (list) => this.setAttribute("class", list.join(" "));
    return {
      contains: (name) => names().includes(name),
      add: (...add) => write([...new Set([...names(), ...add])]),
      remove: (...remove) => write(names().filter((n) => !remove.includes(n))),
      toggle: (name, force) => {
        const on = force ?? !names().includes(name);
        write(on ? [...new Set([...names(), name])] : names().filter((n) => n !== name));
        return on;
      },
    };
  }

  /** Visible when it and every ancestor are shown (like a laid-out page). */
  get offsetParent() {
    for (let node = this; node; node = node.parentNode) if (node.hidden) return null;
    return this.ownerDocument.body;
  }

  getAttribute(name) {
    return this.attrs.has(name) ? this.attrs.get(name) : null;
  }

  setAttribute(name, value) {
    this.attrs.set(name, String(value));
  }

  removeAttribute(name) {
    this.attrs.delete(name);
  }

  hasAttribute(name) {
    return this.attrs.has(name);
  }

  toggleAttribute(name, force) {
    const on = force ?? !this.attrs.has(name);
    if (on) this.attrs.set(name, "");
    else this.attrs.delete(name);
    return on;
  }

  #adopt(nodes) {
    return nodes.map((node) => {
      const child = typeof node === "string" ? new FakeText(node) : node;
      child.parentNode?.childNodes && child.parentNode.removeChild(child);
      child.parentNode = this;
      return child;
    });
  }

  removeChild(child) {
    this.childNodes = this.childNodes.filter((n) => n !== child);
    child.parentNode = null;
  }

  append(...nodes) {
    this.childNodes.push(...this.#adopt(nodes));
  }

  prepend(...nodes) {
    this.childNodes.unshift(...this.#adopt(nodes));
  }

  replaceChildren(...nodes) {
    for (const child of this.childNodes) child.parentNode = null;
    this.childNodes = [];
    this.append(...nodes);
  }

  remove() {
    this.parentNode?.removeChild(this);
  }

  *descendants() {
    for (const child of this.children) {
      yield child;
      yield* child.descendants();
    }
  }

  matches(selectors) {
    return matcher(selectors)(this);
  }

  querySelectorAll(selectors) {
    const test = matcher(selectors);
    return [...this.descendants()].filter(test);
  }

  querySelector(selectors) {
    return this.querySelectorAll(selectors)[0] ?? null;
  }

  closest(selectors) {
    const test = matcher(selectors);
    for (let node = this; node instanceof FakeElement; node = node.parentNode) if (test(node)) return node;
    return null;
  }

  focus() {
    this.ownerDocument.activeElement = this;
  }

  click() {
    this.dispatchEvent(new Event("click"));
  }

  scrollIntoView() {}

  showModal() {
    this.open = true;
  }

  close() {
    this.open = false;
    this.dispatchEvent(new Event("close"));
  }
}

for (const [prop, attr] of Object.entries(PROPERTY_ATTRS)) {
  Object.defineProperty(FakeElement.prototype, prop, {
    get() {
      return this.getAttribute(attr) ?? "";
    },
    set(value) {
      this.setAttribute(attr, value);
    },
  });
}

class FakeDocument extends EventTarget {
  constructor() {
    super();
    this.documentElement = new FakeElement("html", this);
    this.body = new FakeElement("body", this);
    this.documentElement.append(this.body);
    this.activeElement = this.body;
    this.hidden = false;
  }

  createElement(tag) {
    return new FakeElement(tag, this);
  }

  createElementNS(_ns, tag) {
    const node = new FakeElement(tag, this);
    node.namespaceURI = _ns;
    return node;
  }

  createTextNode(text) {
    return new FakeText(text);
  }

  getElementById(id) {
    return this.documentElement.querySelector(`#${id}`);
  }

  querySelectorAll(selectors) {
    return this.documentElement.querySelectorAll(selectors);
  }

  querySelector(selectors) {
    return this.documentElement.querySelector(selectors);
  }
}

function attributes(raw) {
  return [...raw.matchAll(/([\w:-]+)(?:\s*=\s*"([^"]*)")?/g)].map((m) => [m[1], m[2] ?? ""]);
}

/** Builds the body of index.html into `doc`. */
function build(doc, html) {
  const body = html.slice(html.indexOf("<body>") + 6, html.indexOf("</body>"));
  const stack = [doc.body];
  for (const m of body.matchAll(/<!--[\s\S]*?-->|<(\/?)([a-zA-Z][\w-]*)([^>]*)>|([^<]+)/g)) {
    if (m[0].startsWith("<!--")) continue;
    const parent = stack[stack.length - 1];
    if (m[4] !== undefined) parent.append(m[4]);
    else if (m[1]) stack.pop();
    else {
      const element = doc.createElement(m[2]);
      for (const [name, value] of attributes(m[3])) element.setAttribute(name, value);
      parent.append(element);
      if (!VOID.has(m[2].toLowerCase())) stack.push(element);
    }
  }
}

/** A Storage stand-in. */
export function memoryStorage(initial = {}) {
  const items = new Map(Object.entries(initial));
  return {
    getItem: (key) => (items.has(key) ? items.get(key) : null),
    setItem: (key, value) => items.set(key, String(value)),
    removeItem: (key) => items.delete(key),
    items,
  };
}

/**
 * Installs `document`, `window`, `localStorage` and `location` built from the
 * page's index.html; returns the document.
 */
export function installPage({ storage = memoryStorage(), hash = "" } = {}) {
  const doc = new FakeDocument();
  build(doc, readFileSync(new URL("../index.html", import.meta.url), "utf8"));
  const win = new EventTarget();
  const location = {
    hash,
    pathname: "/",
    search: "",
    /** Every address the page navigated to (`location.href = …`), in order. */
    navigations: [],
    get href() {
      return `http://127.0.0.1:42000${this.pathname}${this.search}${this.hash}`;
    },
    set href(url) {
      this.navigations.push(String(url));
    },
    reload() {},
  };
  const history = {
    replaceState(_state, _title, url) {
      const at = String(url).indexOf("#");
      location.hash = at === -1 ? "" : String(url).slice(at);
    },
  };
  Object.assign(win, { location, history, document: doc });
  globalThis.document = doc;
  globalThis.window = win;
  globalThis.localStorage = storage;
  globalThis.location = location;
  globalThis.history = history;
  return doc;
}

/** `fetch` answering from `routes`: `"METHOD /path"` → function returning [status, body]. */
export function fakeFetch(routes) {
  const calls = [];
  globalThis.fetch = async (path, options = {}) => {
    const key = `${options.method || "GET"} ${path}`;
    calls.push(key);
    const route = routes[key];
    const [status, body] = route ? route(calls.filter((c) => c === key).length) : [404, { error: { code: "not_found", message: "no" } }];
    return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
  };
  return calls;
}
