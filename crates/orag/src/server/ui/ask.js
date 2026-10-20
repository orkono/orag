// Questions and answers: each question gets a card that stays on screen
// while the collection stays selected; the newest answer streams in, then
// its citations link to its sources.
// Every label is kept translated (`data-i18n`), so the cards on screen follow
// a language change; the question and the answer text stay as written.
import { $, el, tEl } from "./dom.js";
import { errorBox, request } from "./api.js";
import { displayName, selectedCollection } from "./collections.js";
import { toast } from "./dialog.js";
import { icon } from "./icons.js";
import { answerNoticeKeys, retryAfterMs } from "./format.js";
import { setText, t } from "./i18n.js";
import { sseEvents } from "./sse.js";
import { linkCitations, markCitedSources, relabelPages, renderSources } from "./sources.js";

/** The abort reason of the stop button (not shown: the notice is translated). */
const STOPPED = "stopped";
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
  label.append(icon("sparkle"), tEl("span", "ask.answer"));
  turn.status = tEl("span", "ask.searching", undefined, "answer-status");
  turn.copy = el("button", undefined, "button ghost small");
  turn.copy.type = "button";
  turn.copy.hidden = true;
  turn.copy.append(icon("copy"), tEl("span", "ask.copy"));
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

/** Adds the notices `keys` (one sentence each) under the answer. */
function showNotice(turn, ...keys) {
  for (const key of keys) {
    if (turn.notice.childNodes.length > 0) turn.notice.append(" ");
    turn.notice.append(tEl("span", key));
  }
  turn.notice.hidden = turn.notice.childNodes.length === 0;
}

function settle(turn) {
  turn.answer.classList.remove("pending");
  setText(turn.status, null);
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
      setText(turn.status, data.sources.length > 0 ? "ask.writing" : null);
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
      showNotice(turn, ...answerNoticeKeys(data));
      return true;
    case "error": {
      const err = new Error(`${data.error.message} (${data.error.code})`);
      err.code = data.error.code;
      turn.error.replaceChildren(errorBox(err, "ask.incomplete"));
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

/** An *ask again* button that asks the same question again. */
function retryButton(query) {
  const button = el("button", undefined, "button ghost small");
  button.type = "button";
  button.append(icon("refresh"), tEl("span", "ask.retry"));
  button.addEventListener("click", () => {
    if (activeQuery) return;
    // Asks through the form, then puts back whatever was being typed.
    const input = $("ask-text");
    const draft = input.value;
    input.value = query;
    $("ask-form").requestSubmit();
    input.value = draft;
    fitComposer();
    // Once asked again, the old card's button is spent.
    button.remove();
  }, { once: true });
  return button;
}

async function ask(event) {
  event.preventDefault();
  const input = $("ask-text");
  const query = input.value.trim();
  const collection = selectedCollection();
  if (!query || activeQuery) return;
  if (!collection) {
    toast(t("ask.pickCollection"), "error");
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
      // The server answers 429 only after waiting for the slot itself, so
      // the retry is a single, last attempt.
      setText(turn.status, "ask.busy");
      await pause(retryAfterMs(err.headers ?? new Headers()), controller.signal);
      if (controller.signal.aborted) return;
      setText(turn.status, "ask.searching");
      complete = await streamAnswer(turn, collection, query, controller);
    }
    if (!complete && !controller.signal.aborted) {
      const lost = new Error(t("ask.lost"));
      lost.code = "lost";
      throw lost;
    }
  } catch (err) {
    if (!controller.signal.aborted) {
      const box = errorBox(err, "ask.failed", { lost: "ask.lost" });
      // A dropped connection or a server that went away can be asked again.
      const again = err.code === "lost" || err.code === "unreachable" || err.code === "busy";
      turn.error.replaceChildren(box, ...(again ? [retryButton(query)] : []));
    }
  } finally {
    settle(turn);
    // Stopping keeps the partial answer; clearing the answers removes it.
    if (controller.signal.reason === STOPPED) showNotice(turn, "ask.stopped");
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
    setText(label, "ask.copied");
  } catch {
    setText(label, "ask.copyFailed");
  }
  setTimeout(() => setText(label, "ask.copy"), 1500);
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
  window.addEventListener("orag:language", () => relabelPages($("turns")));
  document.addEventListener("orag:collections", ({ detail: current }) => {
    $("ask-target").textContent = current ? displayName(current) : "";
    $("ask-empty").hidden = !current || current.document_count > 0;
  });
  input.focus();
}
