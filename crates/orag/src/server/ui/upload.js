// Uploads: drop or choose files; each gets a line that follows its indexing.
import { $, el, formatSize } from "./dom.js";
import { errorBox, getJson, request, serverLimits } from "./api.js";
import { refreshCollections, selectedCollection } from "./collections.js";
import { documentStatusView, followedIds, updateDocumentRow } from "./documents.js";

const POLL_MS = 1000;

function uploadItem(file, collection) {
  const item = el("li", undefined, "upload-item");
  const head = el("div", undefined, "upload-head");
  head.append(el("strong", file.name), el("span", "Bekliyor", "badge pending"));
  item.append(
    head,
    el("div", `${collection.name} · ${formatSize(file.size)}`, "hint"),
    el("div", undefined, "upload-detail"),
  );
  $("upload-list").prepend(item);
  return item;
}

/** Sets a line's badge and details; `ok` and `failed` mark it finished. */
function setState(item, label, badgeClass, details = []) {
  const badge = item.querySelector(".badge");
  badge.textContent = label;
  badge.className = `badge ${badgeClass}`;
  item.querySelector(".upload-detail").replaceChildren(...details);
  item.toggleAttribute("data-done", badgeClass === "ok" || badgeClass === "failed");
  $("upload-clear").hidden = $("upload-list").querySelector("[data-done]") === null;
}

function fail(item, err, fallback) {
  setState(item, "Başarısız", "failed", [errorBox(err, fallback)]);
}

/** Shows a document's state on its line; returns true once it is final. */
function renderProgress(item, doc, note) {
  const finished = doc.status === "ready" || doc.status === "failed";
  const [label, badgeClass] = documentStatusView(doc);
  const details = [];
  if (note) details.push(el("p", note, "hint"));
  if (doc.status === "ready") details.push(el("p", `${doc.chunk_count} parça indekslendi.`, "hint"));
  if (doc.error) details.push(el("p", doc.error, "failed"));
  if (doc.warnings && doc.warnings.length > 0) {
    const list = el("ul", undefined, "warnings");
    for (const warning of doc.warnings) list.append(el("li", warning, "warning"));
    details.push(list);
  }
  setState(item, label, badgeClass, details);
  return finished;
}

/** Uploaded documents being indexed: document id → {item, collection, note, failures}. */
const tracked = new Map();
/** Documents polled per tick; the rest wait for the next one. */
const TRACK_LIMIT = 10;
/** Consecutive failed polls (about a minute) before a line gives up. */
const MAX_POLL_FAILURES = 60;
let ticking = false;

function track(item, collection, accepted) {
  const note = accepted.duplicate ? "Bu dosya bu koleksiyonda zaten vardı." : "";
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
      // Deleted while indexing (the Sil button): not an upload failure.
      setState(entry.item, "Silindi", "ok");
      untrack(id);
      finished = true;
    } else if (++entry.failures >= MAX_POLL_FAILURES) {
      fail(entry.item, result.reason, "Durum izlenemedi; Belgeler listesine bakın.");
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
    const item = el("li", "Önce bir koleksiyon seçin.", "failed");
    item.toggleAttribute("data-done", true);
    $("upload-list").prepend(item);
    $("upload-clear").hidden = false;
    return;
  }
  const limits = await serverLimits().catch(() => null);
  for (const file of files) {
    const item = uploadItem(file, collection);
    if (limits && file.size > limits.maxBytes) {
      const why = `Dosya ${formatSize(file.size)}; sınır ${limits.maxMb} MB (config.toml: max_document_mb).`;
      setState(item, "Başarısız", "failed", [el("p", why, "failed")]);
      continue;
    }
    setState(item, "Gönderiliyor…", "pending");
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
      fail(item, err, "Dosya yüklenemedi.");
    }
  }
}

/** Only file drags are taken over; dragging text into a field still works. */
const carriesFiles = (event) => event.dataTransfer && [...event.dataTransfer.types].includes("Files");

export function initUpload() {
  const input = $("upload-file");
  const zone = $("dropzone");
  input.addEventListener("change", () => {
    const files = [...input.files];
    input.value = ""; // the same file can be chosen again
    uploadFiles(files);
  });
  for (const type of ["dragenter", "dragover"]) {
    zone.addEventListener(type, (event) => {
      if (!carriesFiles(event)) return;
      event.preventDefault();
      zone.classList.add("dragging");
    });
  }
  zone.addEventListener("dragleave", () => zone.classList.remove("dragging"));
  zone.addEventListener("drop", (event) => {
    if (!carriesFiles(event)) return;
    event.preventDefault();
    zone.classList.remove("dragging");
    uploadFiles([...event.dataTransfer.files]);
  });
  // A file dropped next to the zone must not replace the page.
  for (const type of ["dragover", "drop"]) {
    window.addEventListener(type, (event) => {
      if (carriesFiles(event)) event.preventDefault();
    });
  }
  $("upload-clear").addEventListener("click", () => {
    for (const item of $("upload-list").querySelectorAll("[data-done]")) item.remove();
    $("upload-clear").hidden = true;
  });
  document.addEventListener("orag:collections", ({ detail: current }) => {
    $("upload-target").textContent = current ? current.name : "";
  });
  serverLimits()
    .then(({ maxMb }) => {
      $("upload-limit").textContent = `En fazla ${maxMb} MB.`;
    })
    .catch(() => {});
}
