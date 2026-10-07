// ORAG built-in page: upload a file, ask a question. Plain DOM, no libraries.
// Document text is untrusted: it is only ever set as textContent.
"use strict";

const COLLECTION = 1;
const POLL_MS = 1000;

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

// ---- Upload ---------------------------------------------------------------

function showUpload(rows, extra) {
  const box = $("upload-status");
  const list = el("dl");
  for (const [label, value, className] of rows) {
    list.append(el("dt", label), el("dd", value, className));
  }
  box.replaceChildren(list, ...(extra || []));
}

function renderProgress(filename, doc, job, note) {
  const finished = doc.status === "ready" || doc.status === "failed";
  const rows = [
    ["Dosya", filename],
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

async function followUpload(filename, accepted) {
  const note = accepted.duplicate ? "Bu dosya bu koleksiyonda zaten var." : "";
  const docPath = `/v1/collections/${COLLECTION}/documents/${accepted.document_id}`;
  const jobPath = accepted.job_id == null ? null : `/v1/jobs/${accepted.job_id}`;
  let job = null;
  for (;;) {
    const jobFinal = job && (job.status === "succeeded" || job.status === "failed");
    const [doc, latestJob] = await Promise.all([
      getJson(docPath),
      jobPath && !jobFinal ? getJson(jobPath) : job,
    ]);
    job = latestJob;
    if (renderProgress(filename, doc, job, note)) return;
    await new Promise((resolve) => setTimeout(resolve, POLL_MS));
  }
}

async function upload(event) {
  event.preventDefault();
  const file = $("upload-file").files[0];
  if (!file) return;
  const button = $("upload-button");
  button.disabled = true;
  showUpload([["Dosya", file.name], ["Durum", "Gönderiliyor…"]]);
  try {
    const form = new FormData();
    form.append("file", file, file.name);
    const response = await fetch(`/v1/collections/${COLLECTION}/documents`, {
      method: "POST",
      body: form,
    });
    if (!response.ok) throw new Error(await errorText(response));
    await followUpload(file.name, await response.json());
  } catch (err) {
    showUpload([["Dosya", file.name], ["Hata", err.message, "failed"]]);
  } finally {
    button.disabled = false;
  }
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

function resetAnswer() {
  $("ask-error").textContent = "";
  $("answer").textContent = "";
  $("answer-notice").hidden = true;
  $("answer-block").hidden = true;
  renderSources([]);
}

async function ask(event) {
  event.preventDefault();
  const query = $("ask-text").value.trim();
  if (!query) return;
  const button = $("ask-button");
  button.disabled = true;
  resetAnswer();
  try {
    const response = await fetch(`/v1/collections/${COLLECTION}/query`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Accept: "text/event-stream" },
      body: JSON.stringify({ query, stream: true }),
    });
    if (!response.ok) throw new Error(await errorText(response));
    $("answer-block").hidden = false;
    let complete = false;
    for await (const record of sseEvents(response.body)) {
      if (handleEvent(record)) {
        complete = true;
        break;
      }
    }
    if (!complete) throw new Error("Bağlantı kesildi; yanıt tamamlanmadı.");
  } catch (err) {
    $("ask-error").textContent = err.message;
  } finally {
    button.disabled = false;
  }
}

document.addEventListener("DOMContentLoaded", () => {
  $("upload-form").addEventListener("submit", upload);
  $("ask-form").addEventListener("submit", ask);
});
