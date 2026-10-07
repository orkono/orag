// ORAG built-in page: manage collections, upload a file, ask a question.
// Plain DOM, no libraries. Document text and names are untrusted: they are
// only ever set as textContent.
"use strict";

const POLL_MS = 1000;
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
  if (!response.ok) throw new Error(await errorText(response));
  return response.json();
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
  $("collection-delete").disabled = !current || current.name === DEFAULT_COLLECTION;
  if (current) prefer(current.id);
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
    [
      "Durum",
      DOCUMENT_STATUS[doc.status] || doc.status,
      doc.status === "ready" ? "ok" : doc.status === "failed" ? "failed" : "",
    ],
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
    const [doc, latestJob] = await Promise.all([
      getJson(docPath),
      jobPath && !jobFinal ? getJson(jobPath) : job,
    ]);
    job = latestJob;
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
    await followUpload(file.name, collection, await response.json());
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
  refreshCollections();
});
