// Collections: the list in the sidebar, the title bar, create, reindex and
// delete. Other sections follow the selection through events on `document`:
// `orag:collections` (the list or the selection was shown again; detail: the
// selected collection) and `orag:switched` (the user changed context, so the
// answers on screen no longer apply).
import { $, el } from "./dom.js";
import { errorBox, getJson, request } from "./api.js";
import { confirmAction, toast, toastError } from "./dialog.js";
import { icon } from "./icons.js";
import { t } from "./i18n.js";

export const DEFAULT_COLLECTION = "default";
const SELECTED_KEY = "orag.collection";

let collections = [];
/** The collection to select after the next load; mirrored in localStorage. */
let preferredId = readStoredId();
let selectedId = null;
/** Only the newest list load may update the list. */
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

/** The selected collection, or null before the list loads. */
export function selectedCollection() {
  return collections.find((c) => c.id === selectedId) || null;
}

export const displayName = (c) => (c.name === DEFAULT_COLLECTION ? t("collections.default") : c.name);
/** "3 belge" as a message, so it can fill another message's placeholder. */
const countMessage = (count) => ({ key: "collections.count", params: { count } });
const countLabel = (count) => t("collections.count", { count });

function renderList() {
  // Rebuilding the buttons must not lose keyboard focus.
  const focusedId = document.activeElement?.closest?.("#collection-list")
    ? Number(document.activeElement.dataset.collectionId)
    : null;
  $("collection-list").replaceChildren(
    ...collections.map((c) => {
      const button = el("button", undefined, "nav-item");
      button.type = "button";
      button.dataset.collectionId = String(c.id);
      if (c.id === selectedId) button.setAttribute("aria-current", "true");
      button.append(icon("folder"), el("span", displayName(c), "nav-name"), el("span", String(c.document_count), "nav-count"));
      button.title = `${displayName(c)} · ${countLabel(c.document_count)}`;
      button.addEventListener("click", () => choose(c.id));
      const item = el("li");
      item.append(button);
      return item;
    }),
  );
  if (focusedId) $("collection-list").querySelector(`[data-collection-id="${focusedId}"]`)?.focus();
}

function selectionShown() {
  const current = selectedCollection();
  renderList();
  $("collection-title").textContent = current ? displayName(current) : "ORAG";
  $("collection-meta").textContent = current
    ? [countLabel(current.document_count), ...(current.name === DEFAULT_COLLECTION ? [t("collections.undeletable")] : [])].join(" · ")
    : "";
  $("collection-delete").disabled = !current || current.name === DEFAULT_COLLECTION;
  $("collection-reindex").disabled = !current || current.document_count === 0;
  if (current) prefer(current.id);
  document.dispatchEvent(new CustomEvent("orag:collections", { detail: current }));
}

function choose(id) {
  if (id === selectedId) return;
  selectedId = id;
  $("collection-status").replaceChildren();
  selectionShown();
  document.dispatchEvent(new Event("orag:switched"));
}

/** Reloads the list; keeps the preferred collection, else selects the first. */
async function loadCollections() {
  const seq = ++loadSeq;
  const body = await getJson("/v1/collections");
  if (seq !== loadSeq) return; // a newer load is under way
  collections = body.collections;
  const keep = collections.find((c) => c.id === preferredId) || collections[0];
  const changed = (keep ? keep.id : null) !== selectedId;
  selectedId = keep ? keep.id : null;
  selectionShown();
  if (changed) document.dispatchEvent(new Event("orag:switched"));
}

/** Best-effort refresh: a failure is shown but never undoes what succeeded. */
export async function refreshCollections() {
  try {
    await loadCollections();
  } catch (err) {
    $("collection-status").replaceChildren(errorBox(err, "collections.loadFailed"));
  }
}

function showForm(shown) {
  $("collection-form").hidden = !shown;
  $("collection-status").replaceChildren();
  if (shown) $("collection-name").focus();
  else $("collection-name").value = "";
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
    showForm(false);
    prefer(created.id);
    toast({ key: "collections.created", params: { name: created.name } });
  } catch (err) {
    $("collection-status").replaceChildren(
      errorBox(err, "collections.createFailed", {
        conflict: "collections.exists",
        invalid_input: "collections.invalidName",
      }),
    );
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
    // The count shown may be old: ask with the current one.
    const latest = (await getJson("/v1/collections")).collections.find((c) => c.id === current.id);
    if (!latest) throw new Error(t("collections.gone", { name: current.name }));
    const confirmed = await confirmAction({
      title: t("collections.deleteQuestion"),
      message: t("collections.deleteMessage", { name: latest.name, count: countLabel(latest.document_count) }),
      confirmLabel: t("collections.deleteConfirm"),
      danger: true,
    });
    if (!confirmed) return;
    await request(`/v1/collections/${current.id}`, { method: "DELETE" });
    prefer(null);
    toast({ key: "common.deleted", params: { name: current.name } });
  } catch (err) {
    toastError(err, "collections.deleteFailed", { conflict: "collections.deleteDefault" });
  } finally {
    selectionShown(); // re-enables the button even if the refresh fails
    // After success or failure (e.g. deleted elsewhere): show what exists now.
    await refreshCollections();
  }
}

async function reindexCollection() {
  const current = selectedCollection();
  if (!current) return;
  const confirmed = await confirmAction({
    title: t("collections.reindexQuestion"),
    message: t("collections.reindexMessage", { name: displayName(current) }),
    confirmLabel: t("collection.reindex"),
  });
  if (!confirmed) return;
  const button = $("collection-reindex");
  button.disabled = true;
  try {
    const response = await request(`/v1/collections/${current.id}/reindex`, { method: "POST" });
    const result = await response.json();
    document.dispatchEvent(new Event("orag:switched"));
    document.dispatchEvent(new Event("orag:reindexed"));
    toast({ key: "collections.reindexQueued", params: { count: countMessage(result.queued_documents) } });
  } catch (err) {
    toastError(err, "collections.reindexFailed", {
      conflict: "collections.reindexBusy",
    });
  } finally {
    selectionShown(); // re-enables the button
  }
}

export function initCollections() {
  $("collection-new").addEventListener("click", () => showForm($("collection-form").hidden));
  $("collection-cancel").addEventListener("click", () => showForm(false));
  $("collection-name").addEventListener("keydown", (event) => {
    if (event.key === "Escape") showForm(false);
  });
  $("collection-form").addEventListener("submit", createCollection);
  $("collection-delete").addEventListener("click", deleteCollection);
  $("collection-reindex").addEventListener("click", reindexCollection);
  document.addEventListener("orag:reconnected", refreshCollections);
  // Names, counts and the title bar in the new language.
  window.addEventListener("orag:language", selectionShown);
  refreshCollections();
}
