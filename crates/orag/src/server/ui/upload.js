// Uploads: drop files anywhere on the page or choose them; each gets a line
// that follows its indexing. Each line keeps how to draw itself, so it is
// drawn again in a new language.
import { $, el, tEl } from "./dom.js";
import { errorBox, getJson, request, serverLimits } from "./api.js";
import { displayName, refreshCollections, selectedCollection } from "./collections.js";
import { documentStatusView, followedIds, updateDocumentRow, warningNode } from "./documents.js";
import { formatSize } from "./format.js";
import { setText, t } from "./i18n.js";

const POLL_MS = 1000;
/** A finished upload's line leaves after this; the document list keeps it. */
const DONE_LINGER_MS = 5000;

/** Each line's file and its current `render()` → [badge label, detail nodes]. */
const lines = new WeakMap();

function paint(item) {
  const line = lines.get(item);
  if (!line) return;
  item.title = `${line.file.name} · ${formatSize(line.file.size)}`;
  const [label, details] = line.render();
  item.querySelector(".badge").textContent = label;
  item.querySelector(".upload-detail").replaceChildren(...details);
}

function uploadItem(file) {
  const item = el("li", undefined, "upload-item");
  const head = el("div", undefined, "upload-head");
  head.append(el("span", file.name, "upload-name"), el("span", undefined, "badge pending"));
  item.append(head, el("div", undefined, "progress"), el("div", undefined, "upload-detail"));
  lines.set(item, { file, render: () => [t("upload.waiting"), []] });
  paint(item);
  $("upload-list").prepend(item);
  return item;
}

/**
 * Sets a line's state: `render()` gives its badge label and details in the
 * current language; `ok` and `failed` mark it finished.
 */
function setState(item, render, badgeClass) {
  lines.get(item).render = render;
  paint(item);
  item.querySelector(".badge").className = `badge ${badgeClass}`;
  item.toggleAttribute("data-done", badgeClass === "ok" || badgeClass === "failed");
  // Failures and notes (a duplicate) stay until cleared; a plain success is
  // also in the document list.
  if (badgeClass === "ok" && !item.querySelector(".upload-detail .hint.keep")) {
    setTimeout(() => removeLine(item), DONE_LINGER_MS);
  }
  showClear();
}

function showClear() {
  $("upload-clear").hidden = $("upload-list").querySelector("[data-done]") === null;
}

function removeLine(item) {
  item.remove();
  showClear();
}

function fail(item, err, fallback) {
  setState(item, () => [t("documents.status.failed"), [errorBox(err, fallback)]], "failed");
}

/** Shows a document's state on its line; returns true once it is final. */
function renderProgress(item, doc, note) {
  const finished = doc.status === "ready" || doc.status === "failed";
  const badgeClass = documentStatusView(doc)[1];
  setState(item, () => [documentStatusView(doc)[0], progressDetails(doc, note)], badgeClass);
  return finished;
}

/** The notes under a line: duplicate, chunks, error and warnings. */
function progressDetails(doc, note) {
  const details = [];
  if (note) details.push(el("p", t(note), "hint keep"));
  if (doc.status === "ready") details.push(el("p", t("upload.indexed", { count: doc.chunk_count }), "hint"));
  if (doc.error) details.push(el("p", doc.error, "failed"));
  for (const warning of doc.warnings || []) details.push(warningNode("p", warning));
  return details;
}

/** Uploaded documents being indexed: document id → {item, collection, note, failures}. */
const tracked = new Map();
/** Documents polled per tick; the rest wait for the next one. */
const TRACK_LIMIT = 10;
/** Consecutive failed polls (about a minute) before a line gives up. */
const MAX_POLL_FAILURES = 60;
let ticking = false;

function track(item, collection, accepted) {
  const note = accepted.duplicate ? "upload.duplicate" : "";
  tracked.set(accepted.document_id, { item, collection, note, failures: 0 });
  followedIds.add(accepted.document_id);
  if (!ticking) {
    ticking = true;
    tick();
  }
}

function untrack(id) {
  tracked.delete(id);
  followedIds.delete(id);
}

/**
 * One loop follows every uploaded document: a batch of reads per tick, a
 * transient error is retried, and the collection counts refresh whenever a
 * document finishes.
 */
async function tick() {
  const batch = [...tracked].slice(0, TRACK_LIMIT);
  const results = await Promise.allSettled(
    batch.map(([id, { collection }]) => getJson(`/v1/collections/${collection.id}/documents/${id}`)),
  );
  let finished = false;
  results.forEach((result, i) => {
    const [id, entry] = batch[i];
    if (result.status === "fulfilled") {
      entry.failures = 0;
      updateDocumentRow(entry.collection, result.value);
      if (renderProgress(entry.item, result.value, entry.note)) {
        untrack(id);
        finished = true;
      }
    } else if (result.reason.status === 404) {
      // Deleted while indexing (the delete button): not an upload failure.
      setState(entry.item, () => [t("upload.removed"), []], "ok");
      untrack(id);
      finished = true;
    } else if (++entry.failures >= MAX_POLL_FAILURES) {
      fail(entry.item, result.reason, "upload.lostTrack");
      untrack(id);
    }
  });
  if (finished) await refreshCollections(); // new document counts
  if (tracked.size === 0) {
    ticking = false;
    return;
  }
  setTimeout(tick, POLL_MS);
}

/** Sends the files one after another (the server takes a few at a time). */
async function uploadFiles(files) {
  const collection = selectedCollection();
  if (files.length === 0) return;
  if (!collection) {
    const item = tEl("li", "upload.noCollection", undefined, "failed");
    item.toggleAttribute("data-done", true);
    $("upload-list").prepend(item);
    $("upload-clear").hidden = false;
    return;
  }
  const limits = await serverLimits().catch(() => null);
  for (const file of files) {
    const item = uploadItem(file);
    if (limits && file.size > limits.maxBytes) {
      const why = () => t("upload.tooLarge", { size: formatSize(file.size), max: limits.maxMb });
      setState(item, () => [t("documents.status.failed"), [el("p", why(), "failed")]], "failed");
      continue;
    }
    setState(item, () => [t("upload.sending"), []], "pending");
    try {
      const form = new FormData();
      form.append("file", file, file.name);
      const response = await request(`/v1/collections/${collection.id}/documents`, {
        method: "POST",
        body: form,
      });
      track(item, collection, await response.json());
      await refreshCollections(); // the new document counts at once
    } catch (err) {
      fail(item, err, "upload.failed");
    }
  }
}

/** Only file drags are taken over; dragging text into a field still works. */
const carriesFiles = (event) => event.dataTransfer && [...event.dataTransfer.types].includes("Files");

/** Files dragged over the window show an overlay; dropping anywhere uploads them. */
function initDropAnywhere() {
  const overlay = $("drop-overlay");
  let depth = 0; // dragenter/dragleave fire for every child element crossed
  window.addEventListener("dragenter", (event) => {
    if (!carriesFiles(event)) return;
    event.preventDefault();
    depth += 1;
    const current = selectedCollection();
    $("drop-target").textContent = current ? t("drop.target", { name: displayName(current) }) : "";
    overlay.hidden = false;
  });
  window.addEventListener("dragleave", (event) => {
    if (!carriesFiles(event)) return;
    depth = Math.max(0, depth - 1);
    if (depth === 0) overlay.hidden = true;
  });
  window.addEventListener("dragover", (event) => {
    if (carriesFiles(event)) event.preventDefault(); // allows the drop
  });
  window.addEventListener("drop", (event) => {
    if (!carriesFiles(event)) return;
    event.preventDefault(); // a dropped file must not replace the page
    depth = 0;
    overlay.hidden = true;
    uploadFiles([...event.dataTransfer.files]);
  });
}

export function initUpload() {
  const input = $("upload-file");
  input.addEventListener("change", () => {
    const files = [...input.files];
    input.value = ""; // the same file can be chosen again
    uploadFiles(files);
  });
  initDropAnywhere();
  $("upload-clear").addEventListener("click", () => {
    for (const item of $("upload-list").querySelectorAll("[data-done]")) item.remove();
    showClear();
  });
  window.addEventListener("orag:language", () => {
    for (const item of $("upload-list").children) paint(item);
  });
  document.addEventListener("orag:version", ({ detail }) => {
    setText($("upload-limit"), "upload.limit", { max: detail.config.max_document_mb });
  });
}
