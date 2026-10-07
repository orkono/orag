// The document list of the selected collection: paged, deletable, and kept
// up to date while documents are indexed.
import { $, el, formatSize } from "./dom.js";
import { errorBox, getJson, request } from "./api.js";
import { refreshCollections, selectedCollection } from "./collections.js";

const PAGE_SIZE = 50;
const POLL_MS = 2000;
/** Unfinished rows refreshed per tick. */
const WATCH_LIMIT = 10;

const DOCUMENT_STATUS = {
  queued: "Sırada",
  indexing: "İndeksleniyor",
  ready: "Hazır",
  failed: "Başarısız",
};

/** A document's display name: JSON text uploads may have no file name. */
export function documentName(doc) {
  return doc.filename || doc.title || `Belge #${doc.id}`;
}

/** Label and badge class of a document status, shared with the upload list. */
export function documentStatusView(doc) {
  const className = doc.status === "ready" ? "ok" : doc.status === "failed" ? "failed" : "pending";
  return [DOCUMENT_STATUS[doc.status] || doc.status, className];
}

/** Cursor of the next page, or null when the list is complete. */
let documentsAfter = null;
/** Only the newest document load may change the table. */
let documentsSeq = 0;
/** The collection the table shows. */
let documentsOf = null;
/** The document each row shows; rows are updated in place. */
const rowDocs = new WeakMap();
/** Documents deleted from this page: a late update must not bring them back. */
const deletedIds = new Set();
/** Documents an upload already polls; the table watcher skips them. */
export const followedIds = new Set();

function documentRow(collection, doc) {
  const row = el("tr");
  row.dataset.id = String(doc.id);
  const remove = el("button", "Sil", "small");
  remove.type = "button";
  remove.title = "Belgeyi koleksiyondan sil";
  remove.addEventListener("click", () => deleteDocument(collection, row, remove));
  const action = el("td");
  action.append(remove);
  const status = el("td");
  status.append(el("span", "", "badge"));
  row.append(el("td", "", "name"), status, el("td", "", "number"), el("td", "", "number"), action);
  fillDocumentRow(row, doc);
  return row;
}

/** Writes a document into its row, keeping the row and its button. */
function fillDocumentRow(row, doc) {
  rowDocs.set(row, doc);
  row.toggleAttribute("data-unfinished", doc.status === "queued" || doc.status === "indexing");
  const [name, status, chunks, size] = row.cells;
  name.replaceChildren(documentName(doc));
  for (const warning of doc.warnings || []) name.append(el("div", warning, "warning"));
  if (doc.error) name.append(el("div", doc.error, "failed"));
  const [label, badgeClass] = documentStatusView(doc);
  const badge = status.firstChild;
  badge.textContent = label;
  badge.className = `badge ${badgeClass}`;
  chunks.textContent = String(doc.chunk_count);
  size.textContent = formatSize(doc.size_bytes);
}

/**
 * Shows a document's latest state (upload progress): updates its row, or
 * appends it when the table already shows the end of the list (ids ascend).
 */
export function updateDocumentRow(collection, doc) {
  if (collection.id !== documentsOf || deletedIds.has(doc.id)) return;
  const row = $("documents").querySelector(`tr[data-id="${Number(doc.id)}"]`);
  if (row) fillDocumentRow(row, doc);
  else if (documentsAfter == null) $("documents").append(documentRow(collection, doc));
  showDocumentsEmpty();
}

function removeDocumentRow(id) {
  deletedIds.add(id);
  $("documents").querySelector(`tr[data-id="${Number(id)}"]`)?.remove();
  showDocumentsEmpty();
}

function showDocumentsEmpty() {
  const empty = $("documents").children.length === 0;
  $("documents-empty").hidden = !empty;
  $("documents-table").hidden = empty;
}

function showListError(err, fallback) {
  $("documents-status").replaceChildren(errorBox(err, fallback));
}

/**
 * Loads the first page again (`reset`) or the next one. Errors are shown in
 * the section, and only by the newest load, so an outdated request (for a
 * collection no longer selected) never overwrites a newer result.
 */
async function loadDocuments(reset) {
  const seq = ++documentsSeq;
  const collection = selectedCollection();
  const more = $("documents-more");
  more.disabled = true;
  if (reset) {
    documentsAfter = null;
    more.hidden = true;
  }
  $("documents-status").replaceChildren();
  if (!collection || collection.id !== documentsOf) {
    // Never show (or offer to delete) another collection's rows meanwhile.
    $("documents").replaceChildren();
    $("documents-empty").hidden = true;
    $("documents-table").hidden = true;
    documentsOf = collection ? collection.id : null;
  }
  if (!collection) return;
  const cursor = reset || documentsAfter == null ? "" : `&after_id=${documentsAfter}`;
  try {
    // One extra row tells whether another page exists, so a collection of
    // exactly 50 documents offers no empty "more" page.
    const body = await getJson(
      `/v1/collections/${collection.id}/documents?limit=${PAGE_SIZE + 1}${cursor}`,
    );
    if (seq !== documentsSeq) return; // another collection or a newer load
    const page = body.documents.slice(0, PAGE_SIZE);
    const rows = page.map((doc) => documentRow(collection, doc));
    if (reset) $("documents").replaceChildren(...rows);
    else $("documents").append(...rows);
    documentsAfter = body.documents.length > PAGE_SIZE ? page[page.length - 1].id : null;
    more.hidden = documentsAfter == null;
    showDocumentsEmpty();
  } catch (err) {
    if (seq === documentsSeq) showListError(err, "Belge listesi yüklenemedi.");
  } finally {
    if (seq === documentsSeq) more.disabled = false;
  }
}

export function refreshDocuments() {
  loadDocuments(true);
}

/** Rows still queued or indexing are refreshed until they finish. */
async function watchUnfinishedDocuments() {
  const collection = selectedCollection();
  if (collection && collection.id === documentsOf && !document.hidden) {
    const ids = [...$("documents").querySelectorAll("tr[data-unfinished]")]
      .map((row) => Number(row.dataset.id))
      .filter((id) => !followedIds.has(id))
      .slice(0, WATCH_LIMIT);
    const results = await Promise.allSettled(
      ids.map((id) => getJson(`/v1/collections/${collection.id}/documents/${id}`)),
    );
    results.forEach((result, i) => {
      if (result.status === "fulfilled") updateDocumentRow(collection, result.value);
      else if (result.reason.status === 404) removeDocumentRow(ids[i]);
      // Other errors: try again on the next tick.
    });
    if (ids.length > 0 && $("documents").querySelector("tr[data-unfinished]") === null) {
      await refreshCollections(); // e.g. a finished reindex
    }
  }
  setTimeout(watchUnfinishedDocuments, POLL_MS);
}

async function deleteDocument(collection, row, button) {
  const doc = rowDocs.get(row);
  const question = `"${documentName(doc)}" belgesi "${collection.name}" koleksiyonundan kalıcı olarak silinsin mi?`;
  if (!window.confirm(question)) return;
  button.disabled = true;
  $("documents-status").replaceChildren();
  try {
    await request(`/v1/collections/${collection.id}/documents/${doc.id}`, { method: "DELETE" });
  } catch (err) {
    // 404: already deleted elsewhere, which is the wanted result too.
    if (err.status !== 404) {
      showListError(err, "Belge silinemedi.");
      button.disabled = false;
      return;
    }
  }
  // Removed in place, so the pages the user has opened stay open.
  removeDocumentRow(doc.id);
  await refreshCollections(); // new document count
}

export function initDocuments() {
  document.addEventListener("orag:collections", ({ detail: current }) => {
    $("documents-target").textContent = current ? current.name : "";
    // Only another collection reloads the table: a refresh of the same one
    // (new counts) keeps the pages the user has opened.
    if ((current ? current.id : null) !== documentsOf) refreshDocuments();
  });
  document.addEventListener("orag:reindexed", refreshDocuments);
  $("documents-more").addEventListener("click", () => loadDocuments(false));
  watchUnfinishedDocuments();
}
