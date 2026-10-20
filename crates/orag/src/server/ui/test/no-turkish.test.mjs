// No user-visible Turkish outside the dictionary (step 77): every script
// but i18n.js is free of Turkish string literals, and index.html keeps
// Turkish only as the initial text of elements that i18n.js fills.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { MESSAGES } from "../i18n.js";

const UI_DIR = new URL("../", import.meta.url);
const TURKISH = /[çğıöşüÇĞİÖŞÜ]/;

/** Characters after which a `/` starts a regular expression, not a division. */
const REGEX_AFTER = new Set([..."(,=:[!&|?{};+-*%<>~^"]);

/**
 * The string literals of a script (quoted strings and template text), with
 * their line numbers; comments and regular expressions are skipped.
 */
export function stringLiterals(source) {
  const literals = [];
  let i = 0;
  let line = 1;
  let last = ""; // the last significant character outside literals
  const advance = (n = 1) => {
    for (let k = 0; k < n; k += 1) if (source[i + k] === "\n") line += 1;
    i += n;
  };
  const readQuoted = (quote) => {
    const startLine = line;
    advance();
    let text = "";
    while (i < source.length && source[i] !== quote) {
      if (source[i] === "\\") {
        text += source.slice(i, i + 2);
        advance(2);
      } else if (quote === "`" && source[i] === "$" && source[i + 1] === "{") {
        advance(2);
        let depth = 1;
        while (i < source.length && depth > 0) {
          if (source[i] === "{") depth += 1;
          else if (source[i] === "}") depth -= 1;
          else if (source[i] === '"' || source[i] === "'" || source[i] === "`") {
            readQuoted(source[i]);
            continue;
          }
          advance();
        }
      } else {
        text += source[i];
        advance();
      }
    }
    advance();
    literals.push({ text, line: startLine });
  };
  const skipRegex = () => {
    advance();
    let inClass = false;
    while (i < source.length && (inClass || source[i] !== "/")) {
      if (source[i] === "\\") advance();
      else if (source[i] === "[") inClass = true;
      else if (source[i] === "]") inClass = false;
      advance();
    }
    advance();
    while (/[a-z]/i.test(source[i] ?? "")) advance();
  };
  while (i < source.length) {
    const c = source[i];
    if (c === "/" && source[i + 1] === "/") {
      while (i < source.length && source[i] !== "\n") advance();
    } else if (c === "/" && source[i + 1] === "*") {
      const end = source.indexOf("*/", i + 2);
      advance((end === -1 ? source.length : end + 2) - i);
    } else if (c === '"' || c === "'" || c === "`") {
      readQuoted(c);
      last = "a";
    } else if (c === "/" && (last === "" || REGEX_AFTER.has(last) || /\breturn\s*$/.test(source.slice(0, i)))) {
      skipRegex();
      last = "a";
    } else {
      if (!/\s/.test(c)) last = c;
      advance();
    }
  }
  return literals;
}

const scripts = readdirSync(UI_DIR).filter((name) => name.endsWith(".js") && name !== "i18n.js");

test("the scanner sees literals and skips comments and regular expressions", () => {
  const found = stringLiterals(
    '// "yorum ş"\nconst a = "bir"; /* "iki ğ" */ const r = /["ç]/g;\nconst b = `x ${"üç"} y`;\nconst c = a / 2 / "dört";',
  ).map((l) => l.text);
  assert.deepEqual(found, ["bir", "üç", "x  y", "dört"]);
});

test("no script outside i18n.js has a Turkish string literal", () => {
  assert.ok(scripts.length >= 15, scripts.join(", "));
  const offenders = [];
  for (const name of scripts) {
    const source = readFileSync(new URL(name, UI_DIR), "utf8");
    for (const { text, line } of stringLiterals(source)) {
      if (TURKISH.test(text)) offenders.push(`${name}:${line}: ${text}`);
    }
  }
  assert.deepEqual(offenders, []);
});

test("every message key a script names is in the dictionary", () => {
  const prefixes = new Set(Object.keys(MESSAGES.tr).map((key) => key.split(".")[0]));
  const missing = [];
  for (const name of scripts) {
    const source = readFileSync(new URL(name, UI_DIR), "utf8");
    for (const { text, line } of stringLiterals(source)) {
      if (!/^[a-z]+(\.[a-zA-Z_]+)+$/.test(text) || !prefixes.has(text.split(".")[0])) continue;
      if (/\.(js|toml|md|txt)$/.test(text)) continue; // file names
      if (!Object.hasOwn(MESSAGES.tr, text)) missing.push(`${name}:${line}: ${text}`);
    }
  }
  assert.deepEqual(missing, []);
});

const VOID = new Set(["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"]);
const TRANSLATED_ATTRS = ["placeholder", "title", "aria-label"];
const html = readFileSync(new URL("index.html", UI_DIR), "utf8");

function attributes(raw) {
  const attrs = new Map();
  for (const m of raw.matchAll(/([\w:-]+)(?:\s*=\s*"([^"]*)")?/g)) attrs.set(m[1], m[2] ?? "");
  return attrs;
}

/** index.html as open/close/text tokens with each text's enclosing element. */
function walkHtml(onElement, onText, onClose = () => {}) {
  const stack = [];
  for (const m of html.matchAll(/<!--[\s\S]*?-->|<(\/?)([a-zA-Z][\w-]*)([^>]*)>|([^<]+)/g)) {
    if (m[0].startsWith("<!--") || m[0].startsWith("<!")) continue;
    if (m[4] !== undefined) {
      onText(m[4], stack[stack.length - 1]);
    } else if (m[1]) {
      stack.pop();
      onClose();
    } else {
      const element = { tag: m[2].toLowerCase(), attrs: attributes(m[3]), children: 0 };
      if (stack.length) stack[stack.length - 1].children += 1;
      onElement(element);
      if (!VOID.has(element.tag) && !m[3].trim().endsWith("/")) stack.push(element);
    }
  }
}

const collapse = (text) => text.replace(/\s+/g, " ").trim();

test("index.html has Turkish only where i18n.js replaces it", () => {
  const offenders = [];
  const keyed = [];
  walkHtml(
    (element) => {
      for (const attr of TRANSLATED_ATTRS) {
        const value = element.attrs.get(attr);
        if (value && TURKISH.test(value) && !element.attrs.has(`data-i18n-${attr}`)) {
          offenders.push(`<${element.tag} ${attr}="${value}">`);
        }
      }
      if (element.attrs.has("data-i18n")) keyed.push(element);
    },
    (text, parent) => {
      if (TURKISH.test(text) && !parent?.attrs.has("data-i18n")) offenders.push(collapse(text));
    },
  );
  assert.deepEqual(offenders, []);
  // textContent replaces children: a translated element holds text only.
  for (const element of keyed) assert.equal(element.children, 0, element.attrs.get("data-i18n"));
  assert.ok(keyed.length > 50, `${keyed.length} translated elements`);
});

test("index.html keys exist and its Turkish text is the dictionary's", () => {
  const problems = [];
  let current = null;
  let text = "";
  const finish = () => {
    if (current && collapse(text) !== collapse(MESSAGES.tr[current] ?? "")) {
      problems.push(`${current}: "${collapse(text)}" vs "${MESSAGES.tr[current]}"`);
    }
    current = null;
    text = "";
  };
  walkHtml(
    (element) => {
      finish();
      for (const [name, value] of element.attrs) {
        if (!name.startsWith("data-i18n")) continue;
        if (!Object.hasOwn(MESSAGES.tr, value)) problems.push(`${name}="${value}" is not a key`);
        const attr = name.slice("data-i18n-".length);
        if (TRANSLATED_ATTRS.includes(attr) && collapse(element.attrs.get(attr) ?? "") !== collapse(MESSAGES.tr[value] ?? "")) {
          problems.push(`${attr} of ${value} differs from the dictionary`);
        }
      }
      if (element.attrs.has("data-i18n")) current = element.attrs.get("data-i18n");
    },
    (part) => {
      if (current) text += part;
    },
    finish,
  );
  finish();
  assert.deepEqual(problems, []);
});

test("the language menu offers TR and EN with a translated label", () => {
  const select = /<select id="language-select"[^>]*>([\s\S]*?)<\/select>/.exec(html);
  assert.ok(select, "a #language-select in the page");
  assert.match(select[0], /data-i18n-aria-label="language\.label"/);
  assert.match(select[1], /<option value="tr">TR<\/option>/);
  assert.match(select[1], /<option value="en">EN<\/option>/);
});
