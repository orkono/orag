// Small DOM helpers. Document text and names are untrusted: they only ever
// reach the page through textContent or text nodes.
import { isMessage, setText } from "./i18n.js";

export const $ = (id) => document.getElementById(id);

export function el(tag, text, className) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}

/**
 * An element showing the message `key` that follows language changes.
 * `params` must not depend on the language (ids, counts, names).
 */
export function tEl(tag, key, params, className) {
  const node = el(tag, undefined, className);
  setText(node, key, params);
  return node;
}

/** `tEl` for a `{ key, params }` message, `el` for plain text. */
export function messageEl(tag, message, className) {
  return isMessage(message) ? tEl(tag, message.key, message.params, className) : el(tag, message, className);
}

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
