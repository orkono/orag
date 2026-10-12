// Questions and answers: each question gets a card that stays on screen
// while the collection stays selected; the newest answer streams in, then
// its citations link to its sources.
import { $, el } from "./dom.js";
import { errorBox, request } from "./api.js";
import { displayName, selectedCollection } from "./collections.js";
import { toast } from "./dialog.js";
import { icon } from "./icons.js";
import { finishNotice, retryAfterMs } from "./format.js";
import { sseEvents } from "./sse.js";
import { linkCitations, markCitedSources, renderSources } from "./sources.js";

const BUSY = "Başka bir yanıt üretiliyor; birazdan kendiliğinden yeniden denenecek.";
const LOST = "Bağlantı kesildi; yanıt tamamlanmadı.";
const ABSTAINED = "Belgelerde bu sorunun yanıtı bulunamadı.";
const STOPPED = "Yanıt durduruldu.";
const MAX_COMPOSER_PX = 200;

/** The question being answered; aborted when the answers are cleared. */
let activeQuery = null;
let turnSeq = 0;

/** A question card: the question, then the answer area that fills in. */
function newTurn(query) {
  const turn = { id: `turn-${++turnSeq}`, text: "", shownSources: new Set() };
  const item = el("li", undefined, "turn");
  item.id = turn.id;
  const question = el("div", undefined, "question");
  question.append(el("p", query));
  const card = el("article", undefined, "answer-card");
  const head = el("div", undefined, "answer-head");
  const label = el("span", undefined, "answer-label");
  label.append(icon("sparkle"), el("span", "Yanıt"));
  turn.status = el("span", "Belgelerde aranıyor…", "answer-status");
  turn.copy = el("button", undefined, "button ghost small");
  turn.copy.type = "button";
  turn.copy.hidden = true;
  turn.copy.append(icon("copy"), el("span", "Kopyala"));
  turn.copy.addEventListener("click", () => copyAnswer(turn));
  head.append(label, turn.status, turn.copy);
  turn.answer = el("p", undefined, "answer pending");
  turn.notice = el("p", undefined, "notice");
  turn.notice.hidden = true;
  turn.error = el("div");
  turn.sources = el("div", undefined, "sources");
  card.append(head, turn.answer, turn.notice, turn.error, turn.sources);
  item.append(question, card);
  $("turns").append(item);
  $("welcome").hidden = true;
  item.scrollIntoView({ behavior: "smooth", block: "start" });
  return turn;
}

function showNotice(turn, text) {
  turn.notice.textContent = text;
  turn.notice.hidden = false;
}

function settle(turn, status = "") {
  turn.answer.classList.remove("pending");
  turn.status.textContent = status;
}

/** Clears the answers and stops a streaming one, so an answer from a
 * previously selected collection never appears under another one. */
function clearTurns() {
  if (activeQuery) activeQuery.abort();
  activeQuery = null;
  setAsking(false);
  $("turns").replaceChildren();
  $("welcome").hidden = false;
}

/** Handles one event; returns true when the stream is complete. */
function handleEvent(turn, { event, data }) {
  switch (event) {
    case "sources":
      renderSources(turn, data.sources);
      turn.status.textContent = data.sources.length > 0 ? "Yanıt yazılıyor…" : "";
      return false;
    case "token":
      turn.answer.append(document.createTextNode(data.text));
      return false;
    case "done":
      turn.text = data.answer;
      turn.answer.replaceChildren(...linkCitations(turn, data.answer, data.citation_markers || []));
      markCitedSources(turn, data.citations || []);
      settle(turn);
      turn.copy.hidden = false;
      if (data.abstained) showNotice(turn, ABSTAINED);
      else if (finishNotice(data.finish_reason)) showNotice(turn, finishNotice(data.finish_reason));
      return true;
    case "error": {
      const err = new Error(`${data.error.message} (${data.error.code})`);
      err.code = data.error.code;
      turn.error.replaceChildren(errorBox(err, "Yanıt tamamlanamadı."));
      return true;
    }
    default:
      return false;
  }
}

function setAsking(asking) {
  $("ask-button").hidden = asking;
  $("ask-stop").hidden = !asking;
}

/** Streams one answer into `turn`; returns true once it is complete. */
async function streamAnswer(turn, collection, query, controller) {
  const response = await request(`/v1/collections/${collection.id}/query`, {
    signal: controller.signal,
    method: "POST",
    headers: { "Content-Type": "application/json", Accept: "text/event-stream" },
    body: JSON.stringify({ query, stream: true }),
  });
  for await (const record of sseEvents(response.body)) {
    if (controller.signal.aborted) return false;
    if (handleEvent(turn, record)) return true;
  }
  return false;
}

/** Waits `ms` unless the question is stopped first. */
function pause(ms, signal) {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms);
    signal.addEventListener("abort", () => {
      clearTimeout(timer);
      resolve();
    }, { once: true });
  });
}

/** A *Tekrar sor* button that asks the same question again. */
function retryButton(query) {
  const button = el("button", undefined, "button ghost small");
  button.type = "button";
  button.append(icon("refresh"), el("span", "Tekrar sor"));
  button.addEventListener("click", () => {
    if (activeQuery) return;
    $("ask-text").value = query;
    $("ask-form").requestSubmit();
  });
  return button;
}

async function ask(event) {
  event.preventDefault();
  const input = $("ask-text");
  const query = input.value.trim();
  const collection = selectedCollection();
  if (!query || activeQuery) return;
  if (!collection) {
    toast("Önce soldan bir koleksiyon seçin.", "error");
    return;
  }
  input.value = "";
  fitComposer();
  const turn = newTurn(query);
  const controller = new AbortController();
  activeQuery = controller;
  setAsking(true);
  try {
    let complete;
    try {
      complete = await streamAnswer(turn, collection, query, controller);
    } catch (err) {
      // The answer slot is taken: wait as long as the server asks, once.
      if (err.code !== "busy" || controller.signal.aborted) throw err;
      turn.status.textContent = BUSY;
      await pause(retryAfterMs(err.headers ?? new Headers()), controller.signal);
      if (controller.signal.aborted) return;
      turn.status.textContent = "Belgelerde aranıyor…";
      complete = await streamAnswer(turn, collection, query, controller);
    }
    if (!complete && !controller.signal.aborted) {
      const lost = new Error(LOST);
      lost.code = "lost";
      throw lost;
    }
  } catch (err) {
    if (!controller.signal.aborted) {
      const box = errorBox(err, "Soru yanıtlanamadı.", { lost: LOST });
      // A dropped connection or a server that went away can be asked again.
      const again = err.code === "lost" || err.code === "unreachable" || err.code === "busy";
      turn.error.replaceChildren(box, ...(again ? [retryButton(query)] : []));
    }
  } finally {
    settle(turn);
    // "Durdur" keeps the partial answer; clearing the answers removes it.
    if (controller.signal.reason === STOPPED) showNotice(turn, STOPPED);
    // Clearing already reset the buttons, and a newer question may own them.
    if (activeQuery === controller) {
      activeQuery = null;
      setAsking(false);
      input.focus();
    }
  }
}

async function copyAnswer(turn) {
  const label = turn.copy.querySelector("span");
  try {
    await navigator.clipboard.writeText(turn.text);
    label.textContent = "Kopyalandı";
  } catch {
    label.textContent = "Kopyalanamadı";
  }
  setTimeout(() => {
    label.textContent = "Kopyala";
  }, 1500);
}

/** The question box grows with its text, up to a few lines. */
function fitComposer() {
  const input = $("ask-text");
  input.style.height = "auto";
  input.style.height = `${Math.min(input.scrollHeight, MAX_COMPOSER_PX)}px`;
}

export function initAsk() {
  const input = $("ask-text");
  $("ask-form").addEventListener("submit", ask);
  input.addEventListener("input", fitComposer);
  input.addEventListener("keydown", (event) => {
    // Enter sends; Shift+Enter starts a new line; an IME composition keeps Enter.
    // keyCode 229: Safari's commit of a composition reports isComposing false.
    const composing = event.isComposing || event.keyCode === 229;
    if (event.key === "Enter" && !event.shiftKey && !composing) {
      event.preventDefault();
      $("ask-form").requestSubmit();
    }
  });
  $("ask-stop").addEventListener("click", () => activeQuery?.abort(STOPPED));
  document.addEventListener("orag:switched", clearTurns);
  document.addEventListener("orag:collections", ({ detail: current }) => {
    $("ask-target").textContent = current ? displayName(current) : "";
    $("ask-empty").hidden = !current || current.document_count > 0;
  });
  input.focus();
}
