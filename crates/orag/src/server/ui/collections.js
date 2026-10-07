// Collection selector and its management: create, reindex, delete.
// Other sections follow the selection through two events on `document`:
// `orag:collections` (the list or the selection was shown again; detail: the
// selected collection) and `orag:switched` (the user changed context, so an
// answer on screen no longer applies).
import { $, el } from "./dom.js";
import { errorBox, getJson, request } from "./api.js";

export const DEFAULT_COLLECTION = "default";
const SELECTED_KEY = "orag.collection";
const STATUS_MS = 6000;

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
export function selectedCollection() {
  const id = Number($("collection").value);
  return collections.find((c) => c.id === id) || null;
}

function switched() {
  document.dispatchEvent(new Event("orag:switched"));
}

/** A success message, cleared after a while unless something replaced it. */
function showStatus(text) {
  const message = el("p", text, "ok");
  $("collection-status").replaceChildren(message);
  setTimeout(() => message.remove(), STATUS_MS);
}

function showError(err, fallback, byCode) {
  $("collection-status").replaceChildren(errorBox(err, fallback, byCode));
}

function selectionShown() {
  const current = selectedCollection();
  $("collection-delete").disabled = !current || current.name === DEFAULT_COLLECTION;
  $("collection-reindex").disabled = !current;
  if (current) prefer(current.id);
  document.dispatchEvent(new CustomEvent("orag:collections", { detail: current }));
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
  selectionShown();
}

/** Best-effort refresh: a failure is shown but never undoes what succeeded. */
export async function refreshCollections() {
  try {
    await loadCollections();
  } catch (err) {
    showError(err, "Koleksiyon listesi yenilenemedi.");
  }
}

async function createCollection(event) {
  event.preventDefault();
  const name = $("collection-name").value.trim();
  if (!name) return;
  const button = $("collection-create");
  button.disabled = true;
  try {
    const response = await request("/v1/collections", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name }),
    });
    const created = await response.json();
    $("collection-name").value = "";
    prefer(created.id);
    switched();
    showStatus(`"${created.name}" oluşturuldu ve seçildi.`);
  } catch (err) {
    showError(err, "Koleksiyon oluşturulamadı.", {
      conflict: "Bu adla bir koleksiyon zaten var (büyük/küçük harf fark etmez).",
      invalid_input: "Geçersiz ad: en fazla 64 harf, rakam, boşluk, \"_\", \".\" veya \"-\".",
    });
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
    await request(`/v1/collections/${current.id}`, { method: "DELETE" });
    prefer(null);
    switched();
    showStatus(`"${current.name}" silindi.`);
  } catch (err) {
    showError(err, "Koleksiyon silinemedi.", { conflict: "Varsayılan koleksiyon silinemez." });
  } finally {
    selectionShown(); // re-enables the button
    // After success or failure (e.g. deleted elsewhere): show what exists now.
    await refreshCollections();
  }
}

async function reindexCollection() {
  const current = selectedCollection();
  if (!current) return;
  const question =
    `"${current.name}" koleksiyonundaki belgeler saklanan kopyalarından mevcut modelle ` +
    "yeniden işlensin mi? Bitene kadar sorular eksik yanıt verebilir; dosyaları yeniden yüklemeniz gerekmez.";
  if (!window.confirm(question)) return;
  const button = $("collection-reindex");
  button.disabled = true;
  try {
    const response = await request(`/v1/collections/${current.id}/reindex`, { method: "POST" });
    const result = await response.json();
    switched();
    showStatus(`"${current.name}": ${result.queued_documents} belge yeniden indeksleniyor.`);
    document.dispatchEvent(new Event("orag:reindexed"));
  } catch (err) {
    showError(err, "Yeniden indeksleme başlatılamadı.", {
      conflict: "Bu koleksiyonda hâlâ indekslenen belgeler var; bitince tekrar deneyin.",
    });
  } finally {
    selectionShown(); // re-enables the button
  }
}

export function initCollections() {
  $("collection").addEventListener("change", () => {
    selectionShown();
    switched();
    $("collection-status").replaceChildren();
  });
  $("collection-form").addEventListener("submit", createCollection);
  $("collection-delete").addEventListener("click", deleteCollection);
  $("collection-reindex").addEventListener("click", reindexCollection);
  refreshCollections();
}
