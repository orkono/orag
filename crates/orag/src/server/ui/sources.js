// The sources of an answer and the citation links into them.
import { $, el } from "./dom.js";


/** Source numbers of the answer being shown, for its citation links. */
let shownSources = new Set();

/** An excerpt, folded to a few lines, with a button that unfolds it. */
function excerptBlock(text) {
  const quote = el("blockquote", text, "excerpt folded");
  const toggle = el("button", "Devamını göster", "link small");
  toggle.type = "button";
  toggle.addEventListener("click", () => {
    const folded = quote.classList.toggle("folded");
    toggle.textContent = folded ? "Devamını göster" : "Kısalt";
  });
  return [quote, toggle];
}

/** Once shown: an excerpt that fits in its folded height needs no button. */
function dropNeedlessFolds() {
  for (const quote of $("sources").querySelectorAll(".excerpt.folded")) {
    if (quote.scrollHeight <= quote.clientHeight + 1) {
      quote.classList.remove("folded");
      quote.nextElementSibling?.remove();
    }
  }
}

export function renderSources(sources) {
  const list = $("sources");
  list.replaceChildren();
  shownSources = new Set(sources.map((source) => source.number));
  for (const source of sources) {
    const item = el("li");
    item.id = `source-${source.number}`;
    item.dataset.number = String(source.number);
    const name = source.filename || source.title || `Belge #${source.document_id}`;
    item.append(el("strong", `[${source.number}] ${name}`));
    if (source.heading_path && source.heading_path.length > 0) {
      item.append(el("div", source.heading_path.join(" › "), "heading"));
    }
    item.append(...excerptBlock(source.excerpt));
    list.append(item);
  }
  $("sources-block").hidden = sources.length === 0;
  dropNeedlessFolds(); // measured now that the list is shown
}

/** Highlights a source, unfolds it and brings it into view. */
function openSource(number) {
  for (const item of $("sources").children) {
    const opened = Number(item.dataset.number) === number;
    item.classList.toggle("opened", opened);
    if (opened) {
      item.querySelector(".excerpt")?.classList.remove("folded");
      const toggle = item.querySelector("button.link");
      if (toggle) toggle.textContent = "Kısalt";
    }
  }
  $(`source-${number}`)?.scrollIntoView({ behavior: "smooth", block: "nearest" });
}

/** One citation marker, as written: digits it cites become links to their sources. */
function citationMarker(text, numbers) {
  const marker = el("span", undefined, "citation-marker");
  marker.title = `Kaynak ${numbers.join(", ")}`;
  const cited = new Set(numbers);
  for (const part of text.split(/(\d+)/)) {
    const number = Number(part);
    if (!/^\d+$/.test(part) || !cited.has(number)) {
      marker.append(document.createTextNode(part));
    } else if (shownSources.has(number)) {
      const link = el("a", part, "citation");
      link.href = `#source-${number}`;
      link.addEventListener("click", (event) => {
        event.preventDefault(); // no history entry, no stale :target later
        openSource(number);
      });
      marker.append(link);
    } else {
      const missing = el("span", part, "citation-invalid");
      missing.title = "Bu numarada bir kaynak yok";
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
export function linkCitations(answer, markers) {
  const bytes = new TextEncoder().encode(answer);
  const decoder = new TextDecoder();
  const text = (from, to) => decoder.decode(bytes.subarray(from, to));
  const nodes = [];
  let last = 0;
  for (const { start, end, numbers } of markers) {
    if (start < last || end > bytes.length || start >= end) continue; // defensive
    nodes.push(document.createTextNode(text(last, start)));
    nodes.push(citationMarker(text(start, end), numbers));
    last = end;
  }
  nodes.push(document.createTextNode(text(last, bytes.length)));
  return nodes;
}

/** Marks the sources the answer cites; the others were only context. */
export function markCitedSources(citations) {
  const cited = new Set(citations);
  for (const item of $("sources").children) {
    const used = cited.has(Number(item.dataset.number));
    item.classList.toggle("cited", used);
    item.classList.toggle("uncited", !used);
    if (!used) item.querySelector("strong").after(el("span", " · yanıtta kullanılmadı", "uncited-note"));
  }
}
