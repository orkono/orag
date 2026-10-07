// ORAG built-in page: manage collections and their documents, upload a file,
// ask a question.
// Plain DOM, no libraries. Document text and names are untrusted: they are
// only ever set as textContent.
"use strict";

const POLL_MS = 1000;
const PAGE_SIZE = 50;
const DEFAULT_COLLECTION = "default";
const SELECTED_KEY = "orag.collection";

const DOCUMENT_STATUS = {
  queued: "Sırada",
  indexing: "İndeksleniyor",
  ready: "Hazır",
  failed: "Başarısız",
};
const JOB_STATUS = {
  queued: "Sırada",
  running: "Çalışıyor",
  succeeded: "Tamamlandı",
  failed: "Başarısız",
};
const FINISH_NOTICE = {
  length: "Yanıt uzunluk sınırına ulaştı; kesilmiş olabilir.",
  repetition: "Model aynı satırı tekrarladığı için yanıt durduruldu; eksik olabilir.",
};

const $ = (id) => document.getElementById(id);

function el(tag, text, className) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}

/** A readable message for a failed response: the API's `{error:{code,message}}` if any. */
async function errorText(response) {
  let detail = response.statusText;
  try {
    const body = await response.json();
    if (body && body.error) detail = `${body.error.message} (${body.error.code})`;
  } catch {
    // Not JSON: keep the status text.
  }
  return `HTTP ${response.status}: ${detail}`;
}

async function getJson(path) {
  const response = await fetch(path, { headers: { Accept: "application/json" } });
  if (!response.ok) {
    const err = new Error(await errorText(response));
    err.status = response.status;
    throw err;
  }
  return response.json();
}

/** A document's display name: JSON text uploads may have no file name. */
function documentName(doc) {
  return doc.filename || doc.title || `Belge #${doc.id}`;
}

/** Label and CSS class of a document status, shared by the upload panel and the table. */
function documentStatusView(doc) {
  const className = doc.status === "ready" ? "ok" : doc.status === "failed" ? "failed" : "";
  return [DOCUMENT_STATUS[doc.status] || doc.status, className];
}

// ---- Collections ----------------------------------------------------------

let collections = [];
/** The collection to select after the next load; mirrored in localStorage. */
let preferredId = readStoredId();
/** Only the newest list load may update the selector. */
let loadSeq = 0;

// Storage may be blocked (privacy settings, embedded views): the page then
// works without remembering the choice.
function readStoredId() {
  try {
    return Number(localStorage.getItem(SELECTED_KEY)) || null;
  } catch {
    return null;
  }
}

function prefer(id) {
  preferredId = id;
  try {
    if (id == null) localStorage.removeItem(SELECTED_KEY);
    else localStorage.setItem(SELECTED_KEY, String(id));
  } catch {
    // Not remembered across reloads; preferredId still holds it.
  }
}

/** The collection chosen in the selector, or null before the list loads. */
function selectedCollection() {
  const id = Number($("collection").value);
  return collections.find((c) => c.id === id) || null;
}

function showCollectionStatus(text, className) {
  $("collection-status").replaceChildren(el("p", text, className));
}

function onCollectionChanged() {
  const current = selectedCollection();
  const name = current ? current.name : "";
  $("upload-target").textContent = name;
  $("ask-target").textContent = name;
  $("documents-target").textContent = name;
  $("collection-delete").disabled = !current || current.name === DEFAULT_COLLECTION;
  if (current) prefer(current.id);
  // Only another collection reloads the table: a refresh of the same one
  // (new counts) keeps the pages the user has opened.
  if ((current ? current.id : null) !== documentsOf) refreshDocuments();
}

/** Reloads the list; keeps the preferred collection, else selects the first. */
async function loadCollections() {
  const seq = ++loadSeq;
  const body = await getJson("/v1/collections");
  if (seq !== loadSeq) return; // a newer load is under way
  collections = body.collections;
  const select = $("collection");
  select.replaceChildren(
    ...collections.map((c) => {
      const option = el("option", `${c.name} (${c.document_count} belge)`);
      option.value = String(c.id);
      return option;
    }),
  );
  if (collections.some((c) => c.id === preferredId)) select.value = String(preferredId);
  onCollectionChanged();
}

/** Best-effort refresh: a failure is shown but never undoes what succeeded. */
async function refreshCollections() {
  try {
    await loadCollections();
  } catch (err) {
    showCollectionStatus(`Koleksiyon listesi yenilenemedi: ${err.message}`, "failed");
  }
}

async function createCollection(event) {
  event.preventDefault();
  const name = $("collection-name").value.trim();
  if (!name) return;
  const button = $("collection-create");
  button.disabled = true;
  try {
    const response = await fetch("/v1/collections", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name }),
    });
    if (!response.ok) throw new Error(await errorText(response));
    const created = await response.json();
    $("collection-name").value = "";
    prefer(created.id);
    resetAnswer();
    showCollectionStatus(`"${created.name}" oluşturuldu ve seçildi.`, "ok");
  } catch (err) {
    showCollectionStatus(err.message, "failed");
  } finally {
    button.disabled = false;
  }
  await refreshCollections();
}

async function deleteCollection() {
  const current = selectedCollection();
  if (!current || current.name === DEFAULT_COLLECTION) return;
  const button = $("collection-delete");
  button.disabled = true;
  try {
    // The count shown in the list may be old: ask with the current one.
    const latest = (await getJson("/v1/collections")).collections.find((c) => c.id === current.id);
    if (!latest) throw new Error(`"${current.name}" artık yok; liste yenilendi.`);
    const question =
      `"${latest.name}" koleksiyonu ve içindeki ${latest.document_count} belge kalıcı olarak silinsin mi?`;
    if (!window.confirm(question)) return;
    const response = await fetch(`/v1/collections/${current.id}`, { method: "DELETE" });
    if (!response.ok) throw new Error(await errorText(response));
    prefer(null);
    resetAnswer();
    showCollectionStatus(`"${current.name}" silindi.`, "ok");
  } catch (err) {
    showCollectionStatus(err.message, "failed");
  } finally {
    onCollectionChanged(); // re-enables the button
    // After success or failure (e.g. deleted elsewhere): show what exists now.
    await refreshCollections();
  }
}

// ---- Documents ------------------------------------------------------------

/** Cursor of the next page, or null when the list is complete. */
let documentsAfter = null;
/** Only the newest document load may change the table. */
let documentsSeq = 0;
/** The collection the table shows. */
let documentsOf = null;

function formatSize(bytes) {
  const units = ["bayt", "KB", "MB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toLocaleString("tr-TR", { maximumFractionDigits: 1 })} ${units[unit]}`;
}

function documentRow(collection, doc) {
  const row = el("tr");
  row.dataset.id = String(doc.id);
  const name = el("td", documentName(doc), "name");
  for (const warning of doc.warnings || []) name.append(el("div", warning, "warning"));
  if (doc.error) name.append(el("div", doc.error, "failed"));
  const [status, statusClass] = documentStatusView(doc);
  const remove = el("button", "Sil");
  remove.type = "button";
  remove.addEventListener("click", () => deleteDocument(collection, doc, row, remove));
  const action = el("td");
  action.append(remove);
  row.append(
    name,
    el("td", status, statusClass),
    el("td", String(doc.chunk_count), "number"),
    el("td", formatSize(doc.size_bytes), "number"),
    action,
  );
  return row;
}

/**
 * Shows a document's latest state (upload progress): replaces its row, or
 * appends it when the table already shows the end of the list (ids ascend).
 */
function updateDocumentRow(collection, doc) {
  if (collection.id !== documentsOf) return;
  const row = $("documents").querySelector(`tr[data-id="${Number(doc.id)}"]`);
  if (row) row.replaceWith(documentRow(collection, doc));
  else if (documentsAfter == null) $("documents").append(documentRow(collection, doc));
  showDocumentsEmpty();
}

function showDocumentsEmpty() {
  $("documents-empty").hidden = $("documents").children.length > 0;
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
  $("documents-status").textContent = "";
  if (!collection || collection.id !== documentsOf) {
    // Never show (or offer to delete) another collection's rows meanwhile.
    $("documents").replaceChildren();
    $("documents-empty").hidden = true;
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
    if (seq === documentsSeq) {
      $("documents-status").textContent = `Belge listesi yüklenemedi: ${err.message}`;
    }
  } finally {
    if (seq === documentsSeq) more.disabled = false;
  }
}

function refreshDocuments() {
  loadDocuments(true);
}

async function deleteDocument(collection, doc, row, button) {
  const question = `"${documentName(doc)}" belgesi "${collection.name}" koleksiyonundan kalıcı olarak silinsin mi?`;
  if (!window.confirm(question)) return;
  button.disabled = true;
  $("documents-status").textContent = "";
  try {
    const path = `/v1/collections/${collection.id}/documents/${doc.id}`;
    const response = await fetch(path, { method: "DELETE" });
    // 404: already deleted elsewhere, which is the wanted result too.
    if (!response.ok && response.status !== 404) throw new Error(await errorText(response));
  } catch (err) {
    $("documents-status").textContent = err.message;
    button.disabled = false;
    return;
  }
  // Removed in place, so the pages the user has opened stay open.
  row.remove();
  showDocumentsEmpty();
  await refreshCollections(); // new document count
}

// ---- Upload ---------------------------------------------------------------

function showUpload(rows, extra) {
  const box = $("upload-status");
  const list = el("dl");
  for (const [label, value, className] of rows) {
    list.append(el("dt", label), el("dd", value, className));
  }
  box.replaceChildren(list, ...(extra || []));
}

function renderProgress(filename, collection, doc, job, note) {
  const finished = doc.status === "ready" || doc.status === "failed";
  const rows = [
    ["Dosya", filename],
    ["Koleksiyon", collection.name],
    ["Durum", ...documentStatusView(doc)],
  ];
  if (job) rows.push(["İş", `#${job.id} ${JOB_STATUS[job.status] || job.status}`]);
  if (finished) rows.push(["Parça sayısı", String(doc.chunk_count)]);
  const error = doc.error || (job && job.error);
  if (error) rows.push(["Hata", error, "failed"]);
  const extra = [];
  if (note) extra.push(el("p", note, "hint"));
  if (doc.warnings && doc.warnings.length > 0) {
    extra.push(el("p", "Uyarılar:"));
    const list = el("ul");
    for (const warning of doc.warnings) list.append(el("li", warning));
    extra.push(list);
  }
  showUpload(rows, extra);
  return finished;
}

async function followUpload(filename, collection, accepted) {
  const note = accepted.duplicate ? "Bu dosya bu koleksiyonda zaten var." : "";
  const docPath = `/v1/collections/${collection.id}/documents/${accepted.document_id}`;
  const jobPath = accepted.job_id == null ? null : `/v1/jobs/${accepted.job_id}`;
  let job = null;
  for (;;) {
    const jobFinal = job && (job.status === "succeeded" || job.status === "failed");
    let doc;
    try {
      [doc, job] = await Promise.all([
        getJson(docPath),
        jobPath && !jobFinal ? getJson(jobPath) : job,
      ]);
    } catch (err) {
      if (err.status !== 404) throw err;
      // Deleted while indexing (the Sil button): not an upload failure.
      showUpload([["Dosya", filename], ["Koleksiyon", collection.name], ["Durum", "Silindi"]]);
      return;
    }
    updateDocumentRow(collection, doc);
    if (renderProgress(filename, collection, doc, job, note)) return;
    await new Promise((resolve) => setTimeout(resolve, POLL_MS));
  }
}

async function upload(event) {
  event.preventDefault();
  const file = $("upload-file").files[0];
  const collection = selectedCollection();
  if (!file) return;
  if (!collection) {
    showUpload([["Hata", "Önce bir koleksiyon seçin.", "failed"]]);
    return;
  }
  const button = $("upload-button");
  button.disabled = true;
  const target = ["Koleksiyon", collection.name];
  showUpload([["Dosya", file.name], target, ["Durum", "Gönderiliyor…"]]);
  try {
    const form = new FormData();
    form.append("file", file, file.name);
    const response = await fetch(`/v1/collections/${collection.id}/documents`, {
      method: "POST",
      body: form,
    });
    if (!response.ok) throw new Error(await errorText(response));
    const accepted = await response.json();
    await followUpload(file.name, collection, accepted);
  } catch (err) {
    showUpload([["Dosya", file.name], target, ["Hata", err.message, "failed"]]);
  } finally {
    button.disabled = false;
  }
  await refreshCollections(); // new document count
}

// ---- Ask ------------------------------------------------------------------

function renderSources(sources) {
  const list = $("sources");
  list.replaceChildren();
  for (const source of sources) {
    const item = el("li");
    const title = el("strong", `[${source.number}] ${source.filename}`);
    item.append(title);
    if (source.heading_path && source.heading_path.length > 0) {
      item.append(el("div", source.heading_path.join(" › "), "heading"));
    }
    item.append(el("blockquote", source.excerpt));
    list.append(item);
  }
  $("sources-block").hidden = sources.length === 0;
}

function showNotice(text) {
  const notice = $("answer-notice");
  notice.textContent = text;
  notice.hidden = false;
}

/**
 * Splits an SSE byte stream into `{event, data}` records. Stopping early (a
 * final event, an error) cancels the stream, which frees the answer slot.
 */
async function* sseEvents(body) {
  const reader = body.pipeThrough(new TextDecoderStream()).getReader();
  try {
    yield* sseRecords(reader);
  } finally {
    reader.cancel().catch(() => {});
  }
}

async function* sseRecords(reader) {
  let buffer = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) return;
    // A trailing "\r" waits for the next chunk: it may be half of "\r\n".
    buffer = (buffer + value).replace(/\r\n|\r(?!$)/g, "\n");
    let end;
    while ((end = buffer.indexOf("\n\n")) !== -1) {
      const block = buffer.slice(0, end);
      buffer = buffer.slice(end + 2);
      let name = "message";
      const data = [];
      for (const line of block.split("\n")) {
        if (line.startsWith("event:")) name = line.slice(6).trim();
        else if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));
      }
      if (data.length > 0) yield { event: name, data: JSON.parse(data.join("\n")) };
    }
  }
}

/** Handles one event; returns true when the stream is complete. */
function handleEvent({ event, data }) {
  switch (event) {
    case "sources":
      renderSources(data.sources);
      return false;
    case "token":
      $("answer").append(document.createTextNode(data.text));
      return false;
    case "done":
      if (data.abstained) {
        showNotice("Belgelerde bu sorunun yanıtı bulunamadı; model yanıt vermekten kaçındı.");
      } else if (FINISH_NOTICE[data.finish_reason]) {
        showNotice(FINISH_NOTICE[data.finish_reason]);
      }
      return true;
    case "error":
      $("ask-error").textContent = `${data.error.message} (${data.error.code})`;
      return true;
    default:
      return false;
  }
}

/** The question being answered; aborted when the answer area is reset. */
let activeQuery = null;

/** Clears the answer area and stops a streaming answer, so an answer from a
 * previously selected collection never appears under another one. */
function resetAnswer() {
  if (activeQuery) activeQuery.abort();
  activeQuery = null;
  $("ask-error").textContent = "";
  $("answer").textContent = "";
  $("answer-notice").hidden = true;
  $("answer-block").hidden = true;
  renderSources([]);
}

async function ask(event) {
  event.preventDefault();
  const query = $("ask-text").value.trim();
  const collection = selectedCollection();
  if (!query) return;
  if (!collection) {
    resetAnswer();
    $("ask-error").textContent = "Önce bir koleksiyon seçin.";
    return;
  }
  const button = $("ask-button");
  button.disabled = true;
  resetAnswer();
  const controller = new AbortController();
  activeQuery = controller;
  try {
    const response = await fetch(`/v1/collections/${collection.id}/query`, {
      signal: controller.signal,
      method: "POST",
      headers: { "Content-Type": "application/json", Accept: "text/event-stream" },
      body: JSON.stringify({ query, stream: true }),
    });
    if (!response.ok) throw new Error(await errorText(response));
    $("answer-block").hidden = false;
    let complete = false;
    for await (const record of sseEvents(response.body)) {
      if (controller.signal.aborted) return;
      if (handleEvent(record)) {
        complete = true;
        break;
      }
    }
    if (!complete) throw new Error("Bağlantı kesildi; yanıt tamamlanmadı.");
  } catch (err) {
    if (!controller.signal.aborted) $("ask-error").textContent = err.message;
  } finally {
    if (activeQuery === controller) activeQuery = null;
    button.disabled = false;
  }
}

document.addEventListener("DOMContentLoaded", () => {
  $("collection").addEventListener("change", () => {
    onCollectionChanged();
    resetAnswer();
    $("collection-status").replaceChildren();
  });
  $("collection-form").addEventListener("submit", createCollection);
  $("collection-delete").addEventListener("click", deleteCollection);
  $("upload-form").addEventListener("submit", upload);
  $("ask-form").addEventListener("submit", ask);
  $("documents-more").addEventListener("click", () => {
    loadDocuments(false).catch((err) => {
      $("documents-status").textContent = err.message;
      $("documents-more").disabled = false;
    });
  });
  refreshCollections();
});
