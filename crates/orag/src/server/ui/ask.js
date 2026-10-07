// Questions: the answer streams in, then its citations link to its sources.
import { $, el } from "./dom.js";
import { errorBox, request } from "./api.js";
import { selectedCollection } from "./collections.js";
import { sseEvents } from "./sse.js";
import { linkCitations, markCitedSources, renderSources } from "./sources.js";

const FINISH_NOTICE = {
  length: "Yanıt uzunluk sınırına ulaştı; kesilmiş olabilir.",
  repetition: "Model aynı satırı tekrarladığı için yanıt durduruldu; eksik olabilir.",
};
const ABSTAINED = "Belgelerde bu sorunun yanıtı bulunamadı; model yanıt vermekten kaçındı.";
const STOPPED = "Yanıt durduruldu.";

/** The question being answered; aborted when the answer area is reset. */
let activeQuery = null;
/** The finished answer's text, for the copy button. */
let answerText = "";

function showNotice(text) {
  const notice = $("answer-notice");
  notice.textContent = text;
  notice.hidden = false;
}

function showError(err, fallback) {
  $("ask-error").replaceChildren(errorBox(err, fallback));
}

/** Clears the answer area and stops a streaming answer, so an answer from a
 * previously selected collection never appears under another one. */
function resetAnswer() {
  if (activeQuery) activeQuery.abort();
  activeQuery = null;
  setAsking(false);
  answerText = "";
  $("ask-error").replaceChildren();
  $("answer").textContent = "";
  $("answer-notice").hidden = true;
  $("answer-block").hidden = true;
  $("answer-copy").hidden = true;
  renderSources([]);
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
      answerText = data.answer;
      $("answer").replaceChildren(...linkCitations(data.answer, data.citation_markers || []));
      markCitedSources(data.citations || []);
      $("answer-copy").hidden = false;
      if (data.abstained) showNotice(ABSTAINED);
      else if (FINISH_NOTICE[data.finish_reason]) showNotice(FINISH_NOTICE[data.finish_reason]);
      return true;
    case "error": {
      const err = new Error(`${data.error.message} (${data.error.code})`);
      err.code = data.error.code;
      showError(err, "Yanıt tamamlanamadı.");
      return true;
    }
    default:
      return false;
  }
}

function setAsking(asking) {
  $("ask-button").disabled = asking;
  $("ask-button").textContent = asking ? "Yanıtlanıyor…" : "Sorgu yap";
  $("ask-stop").hidden = !asking;
}

async function ask(event) {
  event.preventDefault();
  const query = $("ask-text").value.trim();
  const collection = selectedCollection();
  if (!query || activeQuery) return;
  resetAnswer();
  if (!collection) {
    $("ask-error").replaceChildren(el("p", "Önce bir koleksiyon seçin.", "failed"));
    return;
  }
  const controller = new AbortController();
  activeQuery = controller;
  setAsking(true);
  try {
    const response = await request(`/v1/collections/${collection.id}/query`, {
      signal: controller.signal,
      method: "POST",
      headers: { "Content-Type": "application/json", Accept: "text/event-stream" },
      body: JSON.stringify({ query, stream: true }),
    });
    $("answer-block").hidden = false;
    let complete = false;
    for await (const record of sseEvents(response.body)) {
      if (controller.signal.aborted) break;
      if (handleEvent(record)) {
        complete = true;
        break;
      }
    }
    if (!complete && !controller.signal.aborted) {
      throw new Error("Bağlantı kesildi; yanıt tamamlanmadı.");
    }
  } catch (err) {
    if (!controller.signal.aborted) showError(err, "Soru yanıtlanamadı.");
  } finally {
    // "Durdur" keeps the partial answer; a reset (another collection) clears it.
    if (controller.signal.reason === STOPPED) {
      $("answer-block").hidden = false; // also when stopped before any answer
      showNotice(STOPPED);
    }
    // A reset already cleared the buttons, and a newer question may own them.
    if (activeQuery === controller) {
      activeQuery = null;
      setAsking(false);
    }
  }
}

async function copyAnswer() {
  const button = $("answer-copy");
  try {
    await navigator.clipboard.writeText(answerText);
    button.textContent = "Kopyalandı";
  } catch {
    button.textContent = "Kopyalanamadı";
  }
  setTimeout(() => {
    button.textContent = "Kopyala";
  }, 1500);
}

export function initAsk() {
  $("ask-form").addEventListener("submit", ask);
  $("ask-text").addEventListener("keydown", (event) => {
    // Ctrl+Enter (Cmd+Enter on a Mac) sends; Enter alone starts a new line.
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      $("ask-form").requestSubmit();
    }
  });
  $("ask-stop").addEventListener("click", () => activeQuery?.abort(STOPPED));
  $("answer-copy").addEventListener("click", copyAnswer);
  document.addEventListener("orag:switched", resetAnswer);
  document.addEventListener("orag:collections", ({ detail: current }) => {
    $("ask-target").textContent = current ? current.name : "";
    $("ask-empty").hidden = !current || current.document_count > 0;
  });
  $("ask-text").focus();
}
