// Small DOM helpers. Document text and names are untrusted: they only ever
// reach the page through textContent or text nodes.

export const $ = (id) => document.getElementById(id);

export function el(tag, text, className) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}

export function formatSize(bytes) {
  const units = ["bayt", "KB", "MB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toLocaleString("tr-TR", { maximumFractionDigits: 1 })} ${units[unit]}`;
}

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
