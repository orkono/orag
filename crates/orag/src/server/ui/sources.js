// The sources of one answer and the citation links into them. Every answer
// on screen has its own sources, so anchors carry the answer's id.
// Labels are kept translated (`data-i18n`); page labels are rewritten on a
// language change.
import { el, tEl } from "./dom.js";
import { pageLabel } from "./format.js";
import { setAttrText, setText } from "./i18n.js";
import { icon } from "./icons.js";

/** An excerpt, folded to a few lines, with a button that unfolds it. */
function excerptBlock(text) {
  const quote = el("blockquote", text, "excerpt folded");
  const toggle = tEl("button", "sources.more", undefined, "link-button");
  toggle.type = "button";
  toggle.addEventListener("click", () => {
    const folded = quote.classList.toggle("folded");
    setText(toggle, folded ? "sources.more" : "sources.less");
  });
  return [quote, toggle];
}

/** Writes the page labels of the sources under `root` again (a language change). */
export function relabelPages(root) {
  for (const node of root.querySelectorAll(".source-pages[data-pages]")) {
    node.textContent = pageLabel(JSON.parse(node.dataset.pages));
  }
}

/** Once shown: an excerpt that fits in its folded height needs no button. */
function dropNeedlessFolds(root) {
  for (const quote of root.querySelectorAll(".excerpt.folded")) {
    if (quote.scrollHeight <= quote.clientHeight + 1) {
      quote.classList.remove("folded");
      quote.nextElementSibling?.remove();
    }
  }
}

function sourceItem(turn, source) {
  const item = el("li", undefined, "source");
  item.id = `${turn.id}-source-${source.number}`;
  item.dataset.number = String(source.number);
  const head = el("div", undefined, "source-head");
  const name = source.filename || source.title
    ? el("span", source.filename || source.title, "source-name")
    : tEl("span", "documents.unnamed", { id: source.document_id }, "source-name");
  head.append(el("span", String(source.number), "source-number"), icon("file"), name);
  const pages = pageLabel(source.pages);
  if (pages) {
    const label = el("span", pages, "source-pages");
    label.dataset.pages = JSON.stringify(source.pages);
    head.append(label);
  }
  item.append(head);
  if (source.heading_path && source.heading_path.length > 0) {
    item.append(el("div", source.heading_path.join(" › "), "source-path"));
  }
  item.append(...excerptBlock(source.excerpt));
  return item;
}

/** Shows an answer's sources (they arrive before its text). */
export function renderSources(turn, sources) {
  turn.shownSources = new Set(sources.map((source) => source.number));
  turn.sources.replaceChildren();
  if (sources.length === 0) return;
  const list = el("ol", undefined, "source-list");
  list.append(...sources.map((source) => sourceItem(turn, source)));
  turn.sources.append(tEl("h3", "sources.title", { count: sources.length }, "sources-title"), list);
  dropNeedlessFolds(list);
}

/** Highlights a source, unfolds it and brings it into view. */
function openSource(turn, number) {
  for (const item of turn.sources.querySelectorAll(".source")) {
    const opened = Number(item.dataset.number) === number;
    item.classList.toggle("opened", opened);
    if (!opened) continue;
    item.closest("details")?.setAttribute("open", "");
    item.querySelector(".excerpt")?.classList.remove("folded");
    const toggle = item.querySelector(".link-button");
    if (toggle) setText(toggle, "sources.less");
    item.scrollIntoView({ behavior: "smooth", block: "nearest" });
  }
}

/** One citation marker, as written: digits it cites become links to their sources. */
function citationMarker(turn, text, numbers) {
  const marker = el("span", undefined, "citation-marker");
  setAttrText(marker, "title", "sources.citation", { numbers: numbers.join(", ") });
  const cited = new Set(numbers);
  for (const part of text.split(/(\d+)/)) {
    const number = Number(part);
    if (!/^\d+$/.test(part) || !cited.has(number)) {
      marker.append(document.createTextNode(part));
    } else if (turn.shownSources.has(number)) {
      const link = el("a", part, "citation");
      link.href = `#${turn.id}-source-${number}`;
      link.addEventListener("click", (event) => {
        event.preventDefault(); // no history entry, no stale :target later
        openSource(turn, number);
      });
      marker.append(link);
    } else {
      const missing = el("span", part, "citation-invalid");
      setAttrText(missing, "title", "sources.missing");
      marker.append(missing);
    }
  }
  return marker;
}

/**
 * The answer as text nodes and marker elements, from the server's
 * `citation_markers` (UTF-8 byte offsets into `answer`): the server decides
 * what a marker is, so the page never parses citations itself.
 */
export function linkCitations(turn, answer, markers) {
  const bytes = new TextEncoder().encode(answer);
  const decoder = new TextDecoder();
  const text = (from, to) => decoder.decode(bytes.subarray(from, to));
  const nodes = [];
  let last = 0;
  for (const { start, end, numbers } of markers) {
    if (start < last || end > bytes.length || start >= end) continue; // defensive
    nodes.push(document.createTextNode(text(last, start)));
    nodes.push(citationMarker(turn, text(start, end), numbers));
    last = end;
  }
  nodes.push(document.createTextNode(text(last, bytes.length)));
  return nodes;
}

/**
 * Keeps the cited sources in view and folds the others (only context the
 * answer did not use) under one line.
 */
export function markCitedSources(turn, citations) {
  const list = turn.sources.querySelector(".source-list");
  if (!list) return;
  const cited = new Set(citations);
  const unused = [...list.children].filter((item) => !cited.has(Number(item.dataset.number)));
  if (unused.length === 0) return;
  const others = el("ol", undefined, "source-list");
  others.append(...unused);
  const key = unused.length === turn.shownSources.size ? "sources.reviewed" : "sources.unused";
  const more = el("details", undefined, "more-sources");
  more.append(tEl("summary", key, { count: unused.length }), others);
  turn.sources.append(more);
  if (list.children.length === 0) list.remove();
}
